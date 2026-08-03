//! Provider 健康与限速状态持久化
//!
//! 详见 docs/superpowers/specs/2026-08-03-provider-daemon-design.md §3-4

use crate::error::AvcResult;
use crate::svc::now_iso;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const HEALTH_KEEP_N: i64 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Healthy,
    Auth,
    RateLimited,
    Timeout,
    UpstreamError,
    Unconfigured,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Healthy => "healthy",
            Status::Auth => "auth",
            Status::RateLimited => "rate_limited",
            Status::Timeout => "timeout",
            Status::UpstreamError => "upstream_error",
            Status::Unconfigured => "unconfigured",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "healthy" => Some(Status::Healthy),
            "auth" => Some(Status::Auth),
            "rate_limited" => Some(Status::RateLimited),
            "timeout" => Some(Status::Timeout),
            "upstream_error" => Some(Status::UpstreamError),
            "unconfigured" => Some(Status::Unconfigured),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthRow {
    pub id: i64,
    pub provider_key: String,
    pub status: Status,
    pub latency_ms: Option<i64>,
    pub error_msg: Option<String>,
    pub checked_at: String,
    pub source: String, // "probe" or "hook"
}

pub fn provider_key(dim: &str, name: &str) -> String {
    format!("{}.{}", dim, name)
}

pub fn record(
    conn: &Connection,
    key: &str,
    status: Status,
    latency_ms: Option<i64>,
    err_msg: Option<&str>,
    source: &str,
) -> AvcResult<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO provider_health (provider_key, status, latency_ms, error_msg, checked_at, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![key, status.as_str(), latency_ms, err_msg, now_iso(), source],
    )?;
    tx.execute(
        "DELETE FROM provider_health
         WHERE provider_key = ?1
           AND id NOT IN (
             SELECT id FROM provider_health
             WHERE provider_key = ?1
             ORDER BY id DESC LIMIT ?2
           )",
        params![key, HEALTH_KEEP_N],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn latest_per_provider(
    conn: &Connection,
    dim_filter: Option<&str>,
) -> AvcResult<Vec<HealthRow>> {
    let prefix: Option<String> = dim_filter.map(|d| format!("{}.", d));
    let sql = if dim_filter.is_some() {
        "SELECT h.id, h.provider_key, h.status, h.latency_ms, h.error_msg, h.checked_at, h.source
         FROM provider_health h
         INNER JOIN (
           SELECT provider_key, MAX(id) AS max_id FROM provider_health GROUP BY provider_key
         ) m ON h.id = m.max_id
         WHERE h.provider_key LIKE ?1 || '%'
         ORDER BY h.provider_key"
    } else {
        "SELECT id, provider_key, status, latency_ms, error_msg, checked_at, source
         FROM provider_health
         WHERE id IN (SELECT MAX(id) FROM provider_health GROUP BY provider_key)
         ORDER BY provider_key"
    };
    let mut stmt = conn.prepare(sql)?;
    let mapper = |r: &rusqlite::Row| -> rusqlite::Result<HealthRow> {
        let s: String = r.get(2)?;
        Ok(HealthRow {
            id: r.get(0)?,
            provider_key: r.get(1)?,
            status: Status::parse(&s).unwrap_or(Status::UpstreamError),
            latency_ms: r.get(3)?,
            error_msg: r.get(4)?,
            checked_at: r.get(5)?,
            source: r.get(6)?,
        })
    };
    let rows = if let Some(p) = prefix {
        stmt.query_map([p], mapper)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map([], mapper)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(rows)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitRow {
    pub provider_key: String,
    pub last_hit_at: String,
    pub retry_after_s: Option<i64>,
    pub until_ts: Option<String>,
    pub hit_count_24h: i64,
    pub updated_at: String,
}

pub fn rate_limit_upsert(
    conn: &Connection,
    key: &str,
    retry_after_s: Option<i64>,
    until_ts: Option<&str>,
) -> AvcResult<()> {
    let now = now_iso();
    conn.execute(
        "INSERT INTO provider_rate_limit
            (provider_key, last_hit_at, retry_after_s, until_ts, hit_count_24h, updated_at)
         VALUES (?1, ?2, ?3, ?4, 1, ?5)
         ON CONFLICT(provider_key) DO UPDATE SET
            last_hit_at = excluded.last_hit_at,
            retry_after_s = excluded.retry_after_s,
            until_ts = excluded.until_ts,
            hit_count_24h = provider_rate_limit.hit_count_24h + 1,
            updated_at = excluded.updated_at",
        params![key, now, retry_after_s, until_ts, now],
    )?;
    Ok(())
}

pub fn rate_limit_get(conn: &Connection, key: &str) -> AvcResult<Option<RateLimitRow>> {
    let row = conn
        .query_row(
            "SELECT provider_key, last_hit_at, retry_after_s, until_ts, hit_count_24h, updated_at
             FROM provider_rate_limit WHERE provider_key = ?1",
            params![key],
            |r| {
                Ok(RateLimitRow {
                    provider_key: r.get(0)?,
                    last_hit_at: r.get(1)?,
                    retry_after_s: r.get(2)?,
                    until_ts: r.get(3)?,
                    hit_count_24h: r.get(4)?,
                    updated_at: r.get(5)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

pub fn rate_limit_all(conn: &Connection) -> AvcResult<Vec<RateLimitRow>> {
    let mut stmt = conn.prepare(
        "SELECT provider_key, last_hit_at, retry_after_s, until_ts, hit_count_24h, updated_at
         FROM provider_rate_limit ORDER BY provider_key",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(RateLimitRow {
                provider_key: r.get(0)?,
                last_hit_at: r.get(1)?,
                retry_after_s: r.get(2)?,
                until_ts: r.get(3)?,
                hit_count_24h: r.get(4)?,
                updated_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().expect("mem db");
        let sqls = [
            include_str!("../../migrations/0001_init.sql"),
            include_str!("../../migrations/0002_drift_dimensions.sql"),
            include_str!("../../migrations/0003_provider_health.sql"),
        ];
        for s in sqls {
            conn.execute_batch(s).expect("migrate");
        }
        conn
    }

    #[test]
    fn record_writes_status_and_keeps_last_50() {
        let conn = fresh_db();
        for i in 0..55 {
            record(
                &conn,
                "llm.openai",
                Status::Healthy,
                Some(100 + i),
                None,
                "probe",
            )
            .expect("record");
        }
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM provider_health WHERE provider_key = 'llm.openai'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 50);
    }

    #[test]
    fn record_rolls_window_old_entries_dropped() {
        let conn = fresh_db();
        for i in 0..60 {
            record(&conn, "llm.openai", Status::Healthy, Some(i), None, "probe").unwrap();
        }
        let latest = latest_per_provider(&conn, None).unwrap();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].latency_ms, Some(59));
    }

    #[test]
    fn status_latest_per_provider_returns_distinct_rows() {
        let conn = fresh_db();
        record(
            &conn,
            "llm.openai",
            Status::Healthy,
            Some(10),
            None,
            "probe",
        )
        .unwrap();
        record(
            &conn,
            "embed.openai",
            Status::Auth,
            None,
            Some("401".into()),
            "hook",
        )
        .unwrap();
        record(
            &conn,
            "voice.elevenlabs",
            Status::Timeout,
            None,
            None,
            "probe",
        )
        .unwrap();
        let latest = latest_per_provider(&conn, None).unwrap();
        assert_eq!(latest.len(), 3);
    }

    #[test]
    fn status_latest_filters_by_dim() {
        let conn = fresh_db();
        record(
            &conn,
            "llm.openai",
            Status::Healthy,
            Some(10),
            None,
            "probe",
        )
        .unwrap();
        record(&conn, "embed.openai", Status::Auth, None, None, "hook").unwrap();
        let latest = latest_per_provider(&conn, Some("llm")).unwrap();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].provider_key, "llm.openai");
    }

    #[test]
    fn rate_limit_upsert_increments_hit_count() {
        let conn = fresh_db();
        rate_limit_upsert(&conn, "llm.openai", Some(60), Some("2030-01-01T00:00:00Z")).unwrap();
        rate_limit_upsert(&conn, "llm.openai", Some(30), Some("2030-01-01T00:00:30Z")).unwrap();
        let got = rate_limit_get(&conn, "llm.openai").unwrap().expect("row");
        assert_eq!(got.hit_count_24h, 2);
        assert_eq!(got.retry_after_s, Some(30)); // 最近一次覆盖
    }

    #[test]
    fn rate_limit_get_returns_none_when_absent() {
        let conn = fresh_db();
        let got = rate_limit_get(&conn, "missing").unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn rate_limit_all_returns_all_rows() {
        let conn = fresh_db();
        rate_limit_upsert(&conn, "llm.openai", None, None).unwrap();
        rate_limit_upsert(&conn, "embed.openai", Some(15), None).unwrap();
        let all = rate_limit_all(&conn).unwrap();
        assert_eq!(all.len(), 2);
        let keys: Vec<&str> = all.iter().map(|r| r.provider_key.as_str()).collect();
        assert!(keys.contains(&"llm.openai"));
        assert!(keys.contains(&"embed.openai"));
    }
}

// 后续 task 填充 record / latest / rate_limit_upsert / rate_limit_get
