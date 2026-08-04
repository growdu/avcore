//! Drift 评估：比较 base / new persona_version 在 face / voice / style 三个维度的
//! anchor embedding 算 cosine 相似度；与阈值比较得到 passed。
//!
//! Phase 2.5.1: 之前仅 voice 一维；现在 face / style 也走相同的 pipeline（embed Provider
//! 真算一组向量 → 落 persona_versions.face_embed / style_embed / voice_embed 三个独立列）。
//! 未配 embed 时调用方应 fallback 到手 mock drift_report，不破坏行为。
//!
//! 协议：每个 dimension 的"anchor embedding" = embed.<name>.embed("persona:<name>:<dim>:<v>")
//! （与 voice 已有的 seed_text 模式一致）。不下载 CLIP / face_recognition 等本地模型；
//! 通过"维度不同 → 种子文本不同 → 同一 embed 空间不同切片"实现多维 drift。

use crate::config::Config;
use crate::db::Db;
use crate::error::{AvcError, AvcResult};
use crate::provider::real::make_embed;
use crate::provider::EmbedProvider;

/// Drift 维度。PersonaModel 在每个维度上都有 anchor embedding。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimension {
    Face,
    Voice,
    Style,
}

impl Dimension {
    /// 该维度在 persona_versions 表的 (blob, dim, sha) 三列名前缀。
    pub fn columns(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            Dimension::Face => ("face_embed", "face_embed_dim", "face_embed_sha256"),
            Dimension::Voice => ("voice_embed", "voice_embed_dim", "voice_embed_sha256"),
            Dimension::Style => ("style_embed", "style_embed_dim", "style_embed_sha256"),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Dimension::Face => "face",
            Dimension::Voice => "voice",
            Dimension::Style => "style",
        }
    }

    /// 给定 persona 名 + version，返回该维度的 seed text；
    /// 同维度 + 同 persona + 同 version 总是返回同一 seed（drift 评估的稳定锚点）。
    pub fn seed_text(&self, persona: &str, version: i64) -> String {
        format!("persona:{}:{}:{}", persona, self.as_str(), version)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoiceDrift {
    pub base_dim: Option<usize>,
    pub new_dim: Option<usize>,
    pub cosine: Option<f32>,
}

/// 读 persona_version 的某维度 embed；无值 → None。
pub fn fetch_embed(
    db: &Db,
    name: &str,
    version: i64,
    dim: Dimension,
) -> AvcResult<Option<Vec<f32>>> {
    let p = crate::svc::persona::get_persona(db, name)?;
    let (blob_col, dim_col, _) = dim.columns();
    let sql = format!(
        "SELECT {blob}, {dim_col} FROM persona_versions
         WHERE persona_model_id = ? AND version = ?",
        blob = blob_col,
        dim_col = dim_col
    );
    let conn = db.conn.lock().unwrap();
    let row: Option<(Option<Vec<u8>>, Option<i64>)> = conn
        .query_row(&sql, rusqlite::params![&p.id, version], |r| {
            Ok((r.get::<_, Option<Vec<u8>>>(0)?, r.get::<_, Option<i64>>(1)?))
        })
        .ok();
    let Some((Some(blob), Some(d))) = row else {
        return Ok(None);
    };
    if d <= 0 || blob.len() != (d as usize) * 4 {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(d as usize);
    for chunk in blob.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().unwrap();
        out.push(f32::from_le_bytes(arr));
    }
    Ok(Some(out))
}

/// 把一组 f32 + dim 写进 persona_version 的某维度列。
pub fn write_embed(
    db: &Db,
    name: &str,
    version: i64,
    dim: Dimension,
    values: &[f32],
) -> AvcResult<()> {
    let p = crate::svc::persona::get_persona(db, name)?;
    let (blob_col, dim_col, sha_col) = dim.columns();
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for v in values {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let d = values.len() as i64;
    // sha256 hex（用 sha2 crate）
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&bytes);
    let sha = hex::encode(h.finalize());
    let sql = format!(
        "UPDATE persona_versions SET {blob} = ?, {dim_col} = ?, {sha_col} = ?
         WHERE persona_model_id = ? AND version = ?",
        blob = blob_col,
        dim_col = dim_col,
        sha_col = sha_col,
    );
    let conn = db.conn.lock().unwrap();
    conn.execute(&sql, rusqlite::params![&bytes, &d, &sha, &p.id, &version])?;
    Ok(())
}

/// 算两组向量的 cosine similarity。`None` 当任一为空或维度不一致。
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

/// DB-only cosine：base 和 new 都从 DB 读。
pub fn eval_from_db(
    db: &Db,
    name: &str,
    base_v: i64,
    new_v: i64,
    dim: Dimension,
) -> AvcResult<Option<f32>> {
    let base = fetch_embed(db, name, base_v, dim)?;
    let new = fetch_embed(db, name, new_v, dim)?;
    Ok(match (&base, &new) {
        (Some(b), Some(n)) => cosine_similarity(b, n),
        _ => None,
    })
}

/// Provider 真算：用 embed.<name>.embed(seed_text) → new_vec，与 base 算 cosine。
/// 阈值不达标 → Conflict。
pub async fn eval_with_provider(
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
            "{} drift below threshold: cosine={:.4} < {}",
            embed_name, cosine, threshold
        )));
    }
    Ok(cosine)
}

/// 一站式 dimension drift 评估：先 DB（若 base + new 都已写 embed），否则 fallback
/// 到 Provider（用 seed_text 真算 new_vec）。返回 Option<f32>：None 当 base 也无 embed
/// （调用方应 Conflict 提示用户先有 base 再能 eval）。
#[allow(clippy::too_many_arguments)]
pub async fn eval_dimension(
    cfg: &Config,
    db: &Db,
    embed_name: &str,
    name: &str,
    base_v: i64,
    new_v: i64,
    dim: Dimension,
    threshold: f32,
) -> AvcResult<Option<f32>> {
    // 1. DB 优先
    if let Some(c) = eval_from_db(db, name, base_v, new_v, dim)? {
        if c < threshold {
            return Err(AvcError::Conflict(format!(
                "{} drift below threshold: cosine={:.4} < {} (db)",
                dim.as_str(),
                c,
                threshold
            )));
        }
        return Ok(Some(c));
    }
    // 2. Provider fallback
    let base = fetch_embed(db, name, base_v, dim)?.ok_or_else(|| {
        AvcError::Conflict(format!(
            "persona '{}' v{} has no {} embed; cannot evaluate drift",
            name,
            base_v,
            dim.as_str()
        ))
    })?;
    let seed = dim.seed_text(name, new_v);
    let cos = eval_with_provider(cfg, embed_name, &base, &seed, threshold).await?;
    Ok(Some(cos))
}

// ── 兼容旧 API：voice 维度的便捷包装 ──────────────────────────

/// Phase 1 / Phase 2 兼容：等价 `fetch_embed(..., Dimension::Voice)`。
pub fn fetch_voice_embed(db: &Db, name: &str, version: i64) -> AvcResult<Option<Vec<f32>>> {
    fetch_embed(db, name, version, Dimension::Voice)
}

/// Phase 1 / Phase 2 兼容：等价 `eval_from_db(..., Dimension::Voice)` + VoiceDrift 结构。
pub fn eval_voice_from_db(db: &Db, name: &str, base_v: i64, new_v: i64) -> AvcResult<VoiceDrift> {
    let base = fetch_embed(db, name, base_v, Dimension::Voice)?;
    let new = fetch_embed(db, name, new_v, Dimension::Voice)?;
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

/// Phase 1 / Phase 2 兼容：等价 `eval_with_provider(...)`。
pub async fn eval_voice_with_provider(
    cfg: &Config,
    embed_name: &str,
    base: &[f32],
    seed_text: &str,
    threshold: f32,
) -> AvcResult<f32> {
    eval_with_provider(cfg, embed_name, base, seed_text, threshold).await
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

    #[test]
    fn dimension_seed_text_is_stable() {
        let s1 = Dimension::Voice.seed_text("yu", 1);
        let s2 = Dimension::Voice.seed_text("yu", 1);
        assert_eq!(s1, s2);
        assert_eq!(s1, "persona:yu:voice:1");
        assert_eq!(
            Dimension::Face.seed_text("alice", 5),
            "persona:alice:face:5"
        );
        assert_eq!(Dimension::Style.seed_text("bob", 3), "persona:bob:style:3");
    }

    #[test]
    fn dimension_columns_correct() {
        assert_eq!(Dimension::Face.columns().0, "face_embed");
        assert_eq!(Dimension::Voice.columns().0, "voice_embed");
        assert_eq!(Dimension::Style.columns().0, "style_embed");
    }
}
