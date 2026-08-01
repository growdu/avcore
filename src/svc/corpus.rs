//! corpus-svc：知识语料生命周期
//!
//! Phase 1：以"段落 / 换行 / 双换行"为分隔切 chunk，调 embed Provider 真算向量，
//! 落 `corpus_chunks` 表（BLOB）。`search` 调 embed API 算 query 向量，然后
//! 全表 cosine top-K。
//!
//! 文件作为切分源：UTF-8 文本，按 `\n\n` 段落优先 / `\n\n` 不存在退回 `\n` 行 / 都不存在则整段。
//! Chunk 上限 2000 字符（超长行截断不补，保证 DB BLOB 体积可控）。

use std::path::Path;

use crate::config::Config;
use crate::db::Db;
use crate::error::{AvcError, AvcResult};
use crate::provider::real::make_embed;
use crate::provider::EmbedProvider;

pub const CHUNK_MAX_CHARS: usize = 2000;

pub fn split_into_chunks(text: &str) -> Vec<String> {
    let buf = text.replace("\r\n", "\n");
    let has_double_newline = buf.contains("\n\n");

    // 双换行模式：每段独立 chunk，超长按字符窗口裁断。
    let mut chunks: Vec<String> = if has_double_newline {
        buf.split("\n\n")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .flat_map(|s| {
                if s.len() <= CHUNK_MAX_CHARS {
                    vec![s]
                } else {
                    s.chars()
                        .collect::<Vec<_>>()
                        .chunks(CHUNK_MAX_CHARS)
                        .map(|c| c.iter().collect::<String>())
                        .collect()
                }
            })
            .collect()
    } else {
        // 单换行模式：每行独立 chunk
        buf.split('\n')
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    };

    // 完全无换行：整段为一个 chunk
    if chunks.is_empty() {
        let whole = buf.trim().to_string();
        if !whole.is_empty() {
            chunks.push(whole);
        }
    }
    chunks
}

pub fn create_from_file(
    db: &Db,
    cfg: &Config,
    embed_name: &str,
    name: &str,
    source_type: &str,
    language: &str,
    path: &Path,
) -> AvcResult<String> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        AvcError::Io(format!("read {}: {}", path.display(), e))
    })?;
    let chunks = split_into_chunks(&text);
    if chunks.is_empty() {
        return Err(AvcError::Arg(format!("file '{}' empty", path.display())));
    }
    let provider: std::sync::Arc<dyn EmbedProvider> = make_embed(cfg, embed_name)?;
    let now = crate::svc::now_iso();
    let corpus_id = crate::svc::new_id("corpus");
    let mut conn = db.conn.lock().unwrap();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO knowledge_corpora
            (id, name, source_type, language, chunk_count, index_version, created_at)
         VALUES (?, ?, ?, ?, ?, 1, ?)",
        rusqlite::params![&corpus_id, name, source_type, language, chunks.len() as i64, &now],
    )?;
    let chunk_id = crate::svc::new_id("chunk");
    // 每 chunk：调 embed → 写 corpus_chunks
    let vecs = {
        let refs: Vec<&str> = chunks.iter().map(|s| s.as_str()).collect();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| AvcError::Internal(format!("corpus tokio: {}", e)))?;
        rt.block_on(async move { provider.embed(&refs).await })?
    };
    if vecs.len() != chunks.len() {
        return Err(AvcError::ProviderUpstream(format!(
            "embed.{}: returned {} vecs, expected {}",
            embed_name,
            vecs.len(),
            chunks.len()
        )));
    }
    for (i, (text, vec)) in chunks.iter().zip(vecs.iter()).enumerate() {
        let blob: Vec<u8> = vec.iter().flat_map(|f| f.to_le_bytes()).collect();
        let id = format!("{}_{:04}", &chunk_id, i);
        tx.execute(
            "INSERT INTO corpus_chunks
                (id, corpus_id, ordinal, content, embed_blob, embed_dim, token_count, deprecated, meta_json)
             VALUES (?, ?, ?, ?, ?, ?, NULL, 0, NULL)",
            rusqlite::params![
                &id,
                &corpus_id,
                i as i64,
                text,
                &blob,
                vec.len() as i64
            ],
        )?;
    }
    tx.commit()?;
    Ok(corpus_id)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchHit {
    pub chunk_id: String,
    pub ordinal: i64,
    pub content: String,
    pub cosine: f32,
}

pub async fn search_async(
    db: &Db,
    cfg: &Config,
    embed_name: &str,
    corpus_id: &str,
    query: &str,
    topk: usize,
) -> AvcResult<Vec<SearchHit>> {
    let provider = make_embed(cfg, embed_name)?;
    let q_vecs = provider.embed(&[query]).await?;
    let q = q_vecs
        .into_iter()
        .next()
        .ok_or_else(|| AvcError::ProviderUpstream(format!("embed.{}: empty result", embed_name)))?;
    let mut hits = Vec::new();
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, ordinal, content, embed_blob, embed_dim
         FROM corpus_chunks
         WHERE corpus_id = ? AND deprecated = 0",
    )?;
    let rows = stmt.query_map([corpus_id], |r| {
        let id: String = r.get(0)?;
        let ordinal: i64 = r.get(1)?;
        let content: String = r.get(2)?;
        let blob: Vec<u8> = r.get(3)?;
        let dim: i64 = r.get(4)?;
        Ok((id, ordinal, content, blob, dim))
    })?;
    for row in rows {
        let (id, ordinal, content, blob, dim) = row?;
        if dim <= 0 || blob.len() != (dim as usize) * 4 {
            continue;
        }
        let mut vec = Vec::with_capacity(dim as usize);
        for chunk in blob.chunks_exact(4) {
            let arr: [u8; 4] = chunk.try_into().unwrap();
            vec.push(f32::from_le_bytes(arr));
        }
        if let Some(cos) = crate::svc::drift::cosine_similarity(&q, &vec) {
            hits.push(SearchHit {
                chunk_id: id,
                ordinal,
                content,
                cosine: cos,
            });
        }
    }
    hits.sort_by(|a, b| b.cosine.partial_cmp(&a.cosine).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(topk);
    Ok(hits)
}

pub fn search(
    db: &Db,
    cfg: &Config,
    embed_name: &str,
    corpus_id: &str,
    query: &str,
    topk: usize,
) -> AvcResult<Vec<SearchHit>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AvcError::Internal(format!("corpus search tokio: {}", e)))?;
    rt.block_on(search_async(db, cfg, embed_name, corpus_id, query, topk))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_double_newlines() {
        let t = "alpha\n\nbeta\n\ngamma";
        let c = split_into_chunks(t);
        // 双换行 → 每段独立 chunk
        assert_eq!(c, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn split_single_newlines_fallback() {
        let t = "alpha\nbeta\ngamma";
        let c = split_into_chunks(t);
        // 无双换行时按单换行切，每行独立 chunk
        assert_eq!(c, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn split_no_newlines_whole_text() {
        let t = "oneline";
        let c = split_into_chunks(t);
        assert_eq!(c, vec!["oneline"]);
    }
}
