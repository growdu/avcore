//! 后台 daemon runtime
//!
//! 详见 docs/superpowers/specs/2026-08-03-provider-daemon-design.md §4-5
//!
//! T13 adds pidfile/lockfile helpers. T14-T18 add HTTP server, ping loop, and CLI.

use std::path::PathBuf;

use crate::error::{AvcError, AvcResult};

/// Returns the pidfile path: `<data_dir>/avc.pid`
pub fn pid_path() -> AvcResult<PathBuf> {
    let dir = data_dir()?;
    Ok(dir.join("avc.pid"))
}

/// Returns the log file path: `<data_dir>/avc.log`
pub fn log_path() -> AvcResult<PathBuf> {
    let dir = data_dir()?;
    Ok(dir.join("avc.log"))
}

fn data_dir() -> AvcResult<PathBuf> {
    let dir = dirs::data_dir()
        .ok_or_else(|| AvcError::Internal("no data_dir".into()))?
        .join("avc");
    Ok(dir)
}

/// Write the current process pid to the pidfile
pub fn write_pid(pid: u32) -> AvcResult<()> {
    let p = pid_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(p, pid.to_string())?;
    Ok(())
}

/// Read the pidfile; returns None if absent, or `Err(PidfileStale)` if contents are invalid
pub fn read_pid() -> AvcResult<Option<u32>> {
    let p = pid_path()?;
    if !p.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(&p)?;
    let pid = s.trim().parse::<u32>().ok();
    if pid.is_none() {
        return Err(AvcError::PidfileStale(p.display().to_string()));
    }
    Ok(pid)
}

/// Delete the pidfile (no-op if it doesn't exist)
pub fn clear_pid() -> AvcResult<()> {
    let p = pid_path()?;
    if p.exists() {
        std::fs::remove_file(&p)?;
    }
    Ok(())
}

/// Check if a process with the given pid is alive
#[cfg(target_os = "linux")]
pub fn is_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(not(target_os = "linux"))]
pub fn is_alive(_pid: u32) -> bool {
    // macOS / Windows: no portable check without adding dependencies.
    // T18 (start command) handles this case by checking exit status.
    // For v1, we accept "true" (assume alive if pidfile exists).
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: override the data dir for tests by setting XDG_DATA_HOME
    fn with_temp_data_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        std::env::set_var("XDG_DATA_HOME", dir.path());
        dir
    }

    #[test]
    fn pid_round_trip() {
        let _dir = with_temp_data_dir();
        // data_dir() might not honor XDG_DATA_HOME on all systems, so use write_pid's return value
        // to determine the actual path
        let p = pid_path().unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        write_pid(12345).unwrap();
        assert_eq!(read_pid().unwrap(), Some(12345));
        clear_pid().unwrap();
        assert_eq!(read_pid().unwrap(), None);
    }
}

// ---- T14: axum HTTP server ----
//
// 提供 /health/all、/limits/all、/version 三个端点。
// 连接通过 Arc<Mutex<Connection>> 共享（rusqlite::Connection 非 Sync）。
// 监听地址默认 127.0.0.1，禁止远程访问。

use axum::http::StatusCode;
use axum::routing::get;
use axum::{extract::State, Json, Router};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
struct AppState {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

#[derive(Serialize, serde::Deserialize)]
struct HealthDto {
    dim: String,
    name: String,
    status: String,
    latency_ms: Option<i64>,
    last_check_at: String,
    last_error: Option<String>,
    source: String,
}

#[derive(Serialize)]
struct LimitDto {
    dim: String,
    name: String,
    in_cooldown: bool,
    until_ts: Option<String>,
    retry_after_s: Option<i64>,
    hit_count_24h: i64,
}

async fn health_all(State(s): State<AppState>) -> Result<Json<Vec<HealthDto>>, StatusCode> {
    let conn = s.conn.lock().await;
    let rows = crate::svc::health::latest_per_provider(&conn, None)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let out: Vec<HealthDto> = rows
        .into_iter()
        .map(|r| {
            let (dim, name) = r
                .provider_key
                .split_once('.')
                .unwrap_or(("?", r.provider_key.as_str()));
            HealthDto {
                dim: dim.into(),
                name: name.into(),
                status: r.status.as_str().into(),
                latency_ms: r.latency_ms,
                last_check_at: r.checked_at,
                last_error: r.error_msg,
                source: r.source,
            }
        })
        .collect();
    Ok(Json(out))
}

async fn limits_all(State(s): State<AppState>) -> Result<Json<Vec<LimitDto>>, StatusCode> {
    let conn = s.conn.lock().await;
    let rows =
        crate::svc::health::rate_limit_all(&conn).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let now = chrono::Utc::now().timestamp();
    let out: Vec<LimitDto> = rows
        .into_iter()
        .map(|r| {
            let (dim, name) = r
                .provider_key
                .split_once('.')
                .unwrap_or(("?", r.provider_key.as_str()));
            let in_cooldown = r
                .until_ts
                .as_ref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|d| d.timestamp() > now)
                .unwrap_or(false);
            LimitDto {
                dim: dim.into(),
                name: name.into(),
                in_cooldown,
                until_ts: r.until_ts,
                retry_after_s: r.retry_after_s,
                hit_count_24h: r.hit_count_24h,
            }
        })
        .collect();
    Ok(Json(out))
}

async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "started_at": std::env::var("AVC_DAEMON_STARTED_AT").unwrap_or_default(),
    }))
}

pub fn build_router(conn: Arc<Mutex<rusqlite::Connection>>) -> Router {
    Router::new()
        .route("/health/all", get(health_all))
        .route("/limits/all", get(limits_all))
        .route("/version", get(version))
        .with_state(AppState { conn })
}

pub async fn run_http(
    bind: &str,
    port: u16,
    conn: Arc<Mutex<rusqlite::Connection>>,
) -> AvcResult<()> {
    let app = build_router(conn);
    let addr = format!("{}:{}", bind, port);
    let listener =
        tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| AvcError::BindFailed {
                addr: bind.into(),
                port,
                msg: e.to_string(),
            })?;
    tracing::info!("daemon listening on {}", addr);
    axum::serve(listener, app)
        .await
        .map_err(|e| AvcError::Internal(format!("axum: {}", e)))?;
    Ok(())
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use std::net::SocketAddr;

    #[tokio::test]
    async fn http_health_all_returns_empty_array() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let sqls = [
            include_str!("../../migrations/0001_init.sql"),
            include_str!("../../migrations/0002_drift_dimensions.sql"),
            include_str!("../../migrations/0003_provider_health.sql"),
        ];
        for s in sqls {
            conn.execute_batch(s).unwrap();
        }
        let conn = Arc::new(Mutex::new(conn));
        let app = build_router(conn);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        // Wait briefly for server to be ready
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let resp: Vec<HealthDto> = reqwest::get(format!("http://{}/health/all", addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp.len(), 0);
    }
}
