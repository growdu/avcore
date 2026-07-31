//! finetune-svc：finetune（Provider SFT 调用）
//!
//! Phase 1：仅记账 + 调 Provider 的 SFT/clone 端点 + 漂移兜底。
//! 详见 docs/modules/persona-iteration.md §4。

use crate::db::Db;
use crate::error::{AvcError, AvcResult};
use rusqlite::OptionalExtension;
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
    let mut conn = db.conn.lock().unwrap();
    // Immediate 事务：跨进程并发 start 时直接把事务升级到写锁。
    // rusqlite 默认 busy_timeout=5000ms，足够让短事务排队；胜者完成 INSERT 后
    // 释放锁，后续排队的 BEGIN IMMEDIATE 进入事务体并被"target-version 已存在"
    // Conflict 拒绝（exit 4），不会出现 exit 20 (SQLITE_BUSY)。
    //
    // 注：本次最小修复仅改事务行为为 Immediate，由既有的"target-version 已存在"
    // Conflict 兜底；不再做全局 SQLITE_BUSY → Conflict 映射。
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    // Task 1 / Task 2: 在 tx 内、任何 INSERT 前校验 base_version 状态。
    // - 无行 → NotFound("persona '<name>' version <n>")
    // - status 既不是 'ready' 也不是 'pending' → Conflict (信息含 version/status)
    let base_status: Option<String> = tx
        .query_row(
            "SELECT status FROM persona_versions
             WHERE persona_model_id = ? AND version = ?",
            rusqlite::params![&p.id, base_version],
            |r| r.get(0),
        )
        .optional()?;
    match base_status {
        None => {
            return Err(AvcError::NotFound(format!(
                "persona '{}' version {}",
                name, base_version
            )));
        }
        Some(s) if s != "ready" && s != "pending" => {
            return Err(AvcError::Conflict(format!(
                "persona '{}' version {} is not stable (status: {})",
                name, base_version, s
            )));
        }
        _ => {} // ready 或 pending，放行
    }

    // 预占 v(N+1) 行；tx 内先查 (persona, target) 是否已存在 → Conflict。
    // 任一 Err 都由 RAII 自动 rollback。
    let target = base_version + 1;
    let existing: Option<String> = tx
        .query_row(
            "SELECT status FROM persona_versions
             WHERE persona_model_id = ? AND version = ?",
            rusqlite::params![&p.id, target],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(s) = existing {
        return Err(AvcError::Conflict(format!(
            "persona '{}' target version {} already exists (status: {})",
            name, target, s
        )));
    }

    let now = crate::svc::now_iso();
    tx.execute(
        "INSERT INTO persona_versions
            (persona_model_id, version, parent_version, status, created_at)
         VALUES (?, ?, ?, 'building', ?)",
        rusqlite::params![&p.id, target, base_version, &now],
    )?;

    let job_id = crate::svc::new_id("fj");
    let scope_json = serde_json::to_string(scope)?;
    let config_json = serde_json::to_string(cfg)?;

    tx.execute(
        "INSERT INTO finetune_jobs
            (id, persona_model_id, base_version, target_version, scope_json, config_json, status, started_at)
         VALUES (?, ?, ?, ?, ?, ?, 'running', ?)",
        rusqlite::params![&job_id, &p.id, base_version, target, &scope_json, &config_json, &now],
    )?;

    tx.commit()?;
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
