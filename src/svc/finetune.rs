//! finetune-svc：finetune（Provider SFT 调用）
//!
//! Phase 1：仅记账 + 调 Provider 的 SFT/clone 端点 + 漂移兜底。
//! 详见 docs/modules/persona-iteration.md §4。

use crate::db::Db;
use crate::error::AvcResult;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinetuneConfig {
    pub full_retrain: bool,
    pub epochs: u32,
    pub consistency_threshold: f32,
}

impl Default for FinetuneConfig {
    fn default() -> Self {
        Self {
            full_retrain: false,
            epochs: 1,
            consistency_threshold: 0.85,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftReport {
    pub face: f32,
    pub voice: f32,
    pub style: f32,
    pub avg: f32,
    pub passed: bool,
}

pub fn start(
    db: &Db,
    name: &str,
    scope: &[String],
    base_version: i64,
    cfg: &FinetuneConfig,
) -> AvcResult<String> {
    let p = crate::svc::persona::get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();

    // 预占 v(N+1) 行
    let target = base_version + 1;
    let now = crate::svc::now_iso();
    conn.execute(
        "INSERT OR IGNORE INTO persona_versions
            (persona_model_id, version, parent_version, status, created_at)
         VALUES (?, ?, ?, 'building', ?)",
        rusqlite::params![&p.id, target, base_version, &now],
    )?;

    let job_id = crate::svc::new_id("fj");
    let scope_json = serde_json::to_string(scope)?;
    let config_json = serde_json::to_string(cfg)?;

    conn.execute(
        "INSERT INTO finetune_jobs
            (id, persona_model_id, base_version, target_version, scope_json, config_json, status, started_at)
         VALUES (?, ?, ?, ?, ?, ?, 'running', ?)",
        rusqlite::params![&job_id, &p.id, base_version, target, &scope_json, &config_json, &now],
    )?;

    Ok(job_id)
}

/// 漂移兜底：不达标 → DELETE v(N+1) + UPDATE finetune_jobs failed_drift
pub fn publish(db: &Db, fj_id: &str, drift: &DriftReport) -> AvcResult<()> {
    let mut conn = db.conn.lock().unwrap();
    let tx = conn.transaction()?;

    if drift.passed {
        // commit v(N+1) = ready
        tx.execute(
            "UPDATE finetune_jobs SET status = 'succeeded', result_version = target_version,
                drift_report_json = ?, finished_at = ?
             WHERE id = ?",
            rusqlite::params![
                serde_json::to_string(drift)?,
                crate::svc::now_iso(),
                fj_id,
            ],
        )?;
        // UPDATE persona_versions v(N+1) ready
        tx.execute(
            "UPDATE persona_versions SET status = 'ready'
             WHERE persona_model_id = (SELECT persona_model_id FROM finetune_jobs WHERE id = ?)
               AND version = (SELECT target_version FROM finetune_jobs WHERE id = ?)",
            rusqlite::params![fj_id, fj_id],
        )?;
    } else {
        // 回退：DELETE v(N+1)
        tx.execute(
            "DELETE FROM persona_versions
             WHERE persona_model_id = (SELECT persona_model_id FROM finetune_jobs WHERE id = ?)
               AND version = (SELECT target_version FROM finetune_jobs WHERE id = ?)",
            rusqlite::params![fj_id, fj_id],
        )?;
        tx.execute(
            "UPDATE finetune_jobs SET status = 'failed_drift', drift_report_json = ?, finished_at = ?
             WHERE id = ?",
            rusqlite::params![
                serde_json::to_string(drift)?,
                crate::svc::now_iso(),
                fj_id,
            ],
        )?;
    }

    tx.commit()?;
    Ok(())
}
