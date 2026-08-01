//! Drift 评估：比较 base / new persona_version 上的 voice_embed，
//! 算 cosine 相似度；与阈值比较得到 passed。
//!
//! Phase 1：仅 voice 一维；face / style 留 stub（API 未稳定）。
//! 真算需调 embed Provider（同一 OpenAI 兼容 `/embeddings` 端点）。
//! 未配 embed 时调用方应 fallback 到手 mock drift_report，不破坏行为。

use crate::config::Config;
use crate::db::Db;
use crate::error::{AvcError, AvcResult};
use crate::provider::real::make_embed;
use crate::provider::EmbedProvider;

#[derive(Debug, Clone, PartialEq)]
pub struct VoiceDrift {
    pub base_dim: Option<usize>,
    pub new_dim: Option<usize>,
    pub cosine: Option<f32>,
}

pub fn fetch_voice_embed(db: &Db, name: &str, version: i64) -> AvcResult<Option<Vec<f32>>> {
    let p = crate::svc::persona::get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    let row: Option<(Option<Vec<u8>>, Option<i64>)> = conn
        .query_row(
            "SELECT voice_embed, voice_embed_dim FROM persona_versions
             WHERE persona_model_id = ? AND version = ?",
            rusqlite::params![&p.id, version],
            |r| Ok((r.get::<_, Option<Vec<u8>>>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .ok();
    let Some((Some(blob), Some(dim))) = row else {
        return Ok(None);
    };
    if dim <= 0 || blob.len() != (dim as usize) * 4 {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(dim as usize);
    for chunk in blob.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().unwrap();
        out.push(f32::from_le_bytes(arr));
    }
    Ok(Some(out))
}

/// 算两组 voice embed 的 cosine similarity。`None` 当任一为空。
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return None;
    }
    let mut dot = 0f64;
    let mut na = 0f64;
    let mut nb = 0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x = *x as f64;
        let y = *y as f64;
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = (na * nb).sqrt();
    if denom == 0.0 {
        None
    } else {
        Some((dot / denom) as f32)
    }
}

/// 算 drift：读 base / new 的 voice_embed 列，返回 cosine（若都有）。
pub fn eval_voice_from_db(db: &Db, name: &str, base_v: i64, new_v: i64) -> AvcResult<VoiceDrift> {
    let base = fetch_voice_embed(db, name, base_v)?;
    let new = fetch_voice_embed(db, name, new_v)?;
    let cosine = match (&base, &new) {
        (Some(b), Some(n)) => cosine_similarity(b, n),
        _ => None,
    };
    Ok(VoiceDrift {
        base_dim: base.as_ref().map(|v| v.len()),
        new_dim: new.as_ref().map(|v| v.len()),
        cosine,
    })
}

/// 用 embed Provider 真算 voice drift：
/// 1. provider.embed.<name> 配置 → make_embed
/// 2. 调 embed.embed(&[seed_text]) → new_vec
/// 3. 与 base 算 cosine
/// 4. 与 threshold 比较
pub async fn eval_voice_with_provider(
    cfg: &Config,
    embed_name: &str,
    base: &[f32],
    seed_text: &str,
    threshold: f32,
) -> AvcResult<f32> {
    let provider: std::sync::Arc<dyn EmbedProvider> = make_embed(cfg, embed_name)?;
    let new_vec = provider.embed(&[seed_text]).await?;
    let new_vec = new_vec
        .into_iter()
        .next()
        .ok_or_else(|| AvcError::ProviderUpstream(format!("embed.{}: empty result", embed_name)))?;
    let cosine = cosine_similarity(base, &new_vec)
        .ok_or_else(|| AvcError::Internal("cosine_similarity: dim mismatch".into()))?;
    if cosine < threshold {
        return Err(AvcError::Conflict(format!(
            "voice drift below threshold: cosine={:.4} < {}",
            cosine, threshold
        )));
    }
    Ok(cosine)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_is_one() {
        let a = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&a, &a).unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine_similarity(&a, &b).unwrap().abs() < 1e-6);
    }

    #[test]
    fn cosine_dim_mismatch_is_none() {
        let a = vec![1.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), None);
    }

    #[test]
    fn cosine_empty_is_none() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];
        assert_eq!(cosine_similarity(&a, &b), None);
    }
}
