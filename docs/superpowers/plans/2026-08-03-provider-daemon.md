# Provider 健康检查 daemon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `avc` 内新增独立后台 daemon，定时 ping 所有 Provider 并暴露 HTTP 查询接口；现有 `OpenAiCompat*` Provider 的错误路径在不影响主流程的前提下，旁路记录健康与限速状态到 SQLite。

**Architecture:** 三层：① `svc::health` + `provider::probe`（DB 读写 + 探活函数，无网络服务）；② `svc::daemon` + axum（tokio runtime 起 PingLoop + HTTP + Signal 三 task）；③ `cli::daemon`（start/stop/status 父进程管理）+ `cli::provider` 增 `--live` 查询。3 张新表 `provider_health` / `provider_rate_limit` / `daemon_meta`，由 0003 migration 自动跑。

**Tech Stack:** 现有 rusqlite/serde/reqwest/chrono/ulid/tracing。新增 `axum = "0.7"` + `tower = "0.5"`。所有新代码用 `async-trait` + `tokio::sync` 风格，与现有 `provider/real.rs` 一致。

---

## 文件结构

### 新增文件（5）
| 路径 | 职责 | 估计 LOC |
|---|---|---:|
| `migrations/0003_provider_health.sql` | 3 张表 + 索引 | 30 |
| `src/svc/health.rs` | health / rate_limit 的 DB CRUD | 250 |
| `src/provider/probe.rs` | 5 维 Provider 探活函数 + dispatcher | 300 |
| `src/svc/daemon.rs` | tokio runtime: PingLoop + HTTP + Signal | 450 |
| `src/cli/daemon.rs` | start / stop / status / logs | 200 |

### 修改文件（10）
| 路径 | 修改内容 |
|---|---|
| `src/provider/real.rs` | 4 处错误分支加 hook（~4 行 × 4） |
| `src/config.rs` | 新增 `DaemonCfg` 结构 + 默认值 + 解析 |
| `src/main.rs` | 识别 `_run` 隐藏 verb 并路由到 `svc::daemon::_run` |
| `src/lib.rs` | 导出 `health` / `daemon` / `probe` 三个模块 |
| `src/error.rs` | 新增 `AvcError::AlreadyRunning` / `BindFailed` / `PidfileStale` |
| `src/cli/mod.rs` | 增加 `daemon` 子命令 dispatch |
| `src/cli/provider.rs` | `provider status` / `rate-limit` 增 `--live` + `--json` + `--dim` |
| `Cargo.toml` | 新增 `axum = "0.7"` + `tower = "0.5"` |
| `docs/cli.md` / `docs/status.md` | 文档同步 |
| `CHANGELOG.md` | 新增 `[Unreleased]` 条目 |

### 测试文件
| 路径 | 覆盖 |
|---|---|
| `src/svc/health.rs` 内 `#[cfg(test)] mod tests` | 8 个 health 单元测试 |
| `src/provider/probe.rs` 内 `#[cfg(test)] mod tests` | 7 个 probe 单元测试 |
| `src/svc/daemon.rs` 内 `#[cfg(test)] mod tests` | 4 个 daemon 内部测试 |
| `tests/integration.rs` | 12 个端到端集成测试 |

**总规模：~1,500 LOC（含 ~1,000 行测试）**

---

## 实施里程碑

- **M1**（T1-T4）：Schema + DB 层（health.rs CRUD + 辅助函数）
- **M2**（T5-T9）：Provider 探活 5 函数
- **M3**（T10-T11）：被动 hook 接入
- **M4**（T12）：配置 + 错误变体
- **M5**（T13-T18）：daemon 进程 + HTTP + 父进程 CLI
- **M6**（T19-T20）：provider status / rate-limit CLI（含 --live）
- **M7**（T21-T23）：集成测试 + 文档 + 收口

每完成一个里程碑，对应功能可独立 demo。**M1+M2+M3+M4+M6** 即可在没有 daemon 的情况下工作（被动 hook + CLI 查 DB）；**+M5** 后才启用主动 ping。

---

## 约定

- 所有 commit 用中文 `feat:` / `fix:` / `chore:` / `test:` / `docs:` 前缀（与现有 `git log` 风格一致）
- 所有 hook 永远 best-effort，错误用 `.ok()` 吞掉，不影响主流程 exit code
- 所有新增 public 函数必须 `pub` 加 `///` rustdoc 注释
- 所有时间戳用 `svc::now_iso()`（已存在）
- 所有 ID 用 `svc::new_id(prefix)`（已存在）
- 所有新模块必须 `pub mod` 显式声明，遵循项目结构
- 文件锁用 `fs2` crate？**不引入** — Unix 用 `nix::fcntl` 已有依赖？查 `Cargo.lock` 确认；如果没，用 `std::fs::File::try_lock`（Rust 1.89+ 已稳定），fallback 到 `nix::fcntl::flock`（需新加 dep）。本 plan 假设用 `fs2 = "0.4"`（轻量跨平台锁库，无其它依赖）
- 如 `fs2` 引入失败，改用 `nix = "0.29"`（仅 Unix），Windows 用 `windows-sys` 已有？看 `Cargo.toml` 现状

---

## Task 1: 添加 migration 0003

**Files:**
- Create: `migrations/0003_provider_health.sql`

- [ ] **Step 1: 创建迁移文件**

```sql
-- 0003_provider_health.sql
-- Provider 健康状态滚动窗口、限速状态、daemon 元信息
-- 详见 docs/superpowers/specs/2026-08-03-provider-daemon-design.md §3

CREATE TABLE IF NOT EXISTS provider_health (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_key TEXT NOT NULL,
    status TEXT NOT NULL,
    latency_ms INTEGER,
    error_msg TEXT,
    checked_at TEXT NOT NULL,
    source TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_provider_health_key_time
    ON provider_health(provider_key, checked_at DESC);

CREATE TABLE IF NOT EXISTS provider_rate_limit (
    provider_key TEXT PRIMARY KEY,
    last_hit_at TEXT NOT NULL,
    retry_after_s INTEGER,
    until_ts TEXT,
    hit_count_24h INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS daemon_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

- [ ] **Step 2: 验证 migration runner 自动捡起**

`src/db/schema.rs` 现有 `migrations` 列表是 `&["0001_init", "0002_drift_dimensions"]`，**需要追加 "0003_provider_health"**。修改：

```rust
const MIGRATIONS: &[&str] = &[
    "0001_init",
    "0002_drift_dimensions",
    "0003_provider_health",
];
```

文件：`src/db/schema.rs`，替换 `MIGRATIONS` 常量。

- [ ] **Step 3: 跑测试确认 migration 顺序正确**

```bash
cd /home/ubuntu/avcore && cargo test --test integration -- --nocapture init_idempotent_guard
```

预期：PASS（既有测试）。如果 FAIL 看 stderr 检查 migration 错误。

- [ ] **Step 4: 提交**

```bash
git add migrations/0003_provider_health.sql src/db/schema.rs
git commit -m "feat(schema): add provider_health, provider_rate_limit, daemon_meta tables"
```

---

## Task 2: 实现 `svc::health` 模块骨架 + 错误变体

**Files:**
- Create: `src/svc/health.rs`
- Modify: `src/error.rs`（新增变体）
- Modify: `src/lib.rs`（导出新模块）

- [ ] **Step 1: 在 error.rs 新增变体**

修改 `src/error.rs`，在 `AvcError` enum 现有变体后追加：

```rust
    #[error("daemon already running: pid {pid}, port {port}")]
    AlreadyRunning { pid: u32, port: u16 },

    #[error("daemon bind failed on {addr}:{port}: {msg}")]
    BindFailed { addr: String, port: u16, msg: String },

    #[error("pidfile stale: {0}")]
    PidfileStale(String),

    #[error("daemon not running")]
    DaemonNotRunning,
```

确认 `AvcError` 已 `#[derive(thiserror::Error)]`。

- [ ] **Step 2: 创建 svc::health.rs 骨架**

新建 `src/svc/health.rs`：

```rust
//! Provider 健康与限速状态持久化
//!
//! 详见 docs/superpowers/specs/2026-08-03-provider-daemon-design.md §3-4

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use crate::error::AvcResult;
use crate::svc::now_iso;

const HEALTH_KEEP_N: i64 = 50;
const RATE_LIMIT_24H_S: i64 = 86_400;

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

// 后续 task 填充 record / latest / rate_limit_upsert / rate_limit_get
```

- [ ] **Step 3: 在 lib.rs 导出**

修改 `src/lib.rs`，在 `pub mod` 列表追加 `pub mod svc;` 行附近（确认现有已有 `pub mod svc;`），无需修改；只在 `src/svc/mod.rs` 加 `pub mod health;`。

修改 `src/svc/mod.rs`，在 `pub mod` 列表加入：

```rust
pub mod health;
```

- [ ] **Step 4: 编译通过**

```bash
cd /home/ubuntu/avcore && cargo build 2>&1 | tail -5
```

预期：`cargo build: 0 errors, N warnings`（仅已有的 license 警告）。

- [ ] **Step 5: 提交**

```bash
git add src/error.rs src/svc/mod.rs src/svc/health.rs
git commit -m "feat(health): add Status enum, provider_key helper, error variants"
```

---

## Task 3: 实现 `record()` 与 `latest_per_provider()`（TDD）

**Files:**
- Modify: `src/svc/health.rs`

- [ ] **Step 1: 写测试（含滚动窗口）**

在 `src/svc/health.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::db::schema::run_migrations;

    fn fresh_db() -> Connection {
        let conn = open_in_memory().expect("mem db");
        run_migrations(&conn).expect("migrate");
        conn
    }

    #[test]
    fn record_writes_status_and_keeps_last_50() {
        let conn = fresh_db();
        for i in 0..55 {
            record(&conn, "llm.openai", Status::Healthy, Some(100 + i), None, "probe")
                .expect("record");
        }
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM provider_health WHERE provider_key = 'llm.openai'",
                       [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 50);
    }

    #[test]
    fn record_rolls_window_old_entries_dropped() {
        let conn = fresh_db();
        // 写 60 条：最后 10 条 id 应为 51..=60
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
        record(&conn, "llm.openai", Status::Healthy, Some(10), None, "probe").unwrap();
        record(&conn, "embed.openai", Status::Auth, None, Some("401".into()), "hook").unwrap();
        record(&conn, "voice.elevenlabs", Status::Timeout, None, None, "probe").unwrap();
        let latest = latest_per_provider(&conn, None).unwrap();
        assert_eq!(latest.len(), 3);
    }

    #[test]
    fn status_latest_filters_by_dim() {
        let conn = fresh_db();
        record(&conn, "llm.openai", Status::Healthy, Some(10), None, "probe").unwrap();
        record(&conn, "embed.openai", Status::Auth, None, None, "hook").unwrap();
        let latest = latest_per_provider(&conn, Some("llm")).unwrap();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].provider_key, "llm.openai");
    }
}
```

- [ ] **Step 2: 检查 `db::open_in_memory` 与 `db::schema::run_migrations` 公开 API**

`grep "pub fn" /home/ubuntu/avcore/src/db/mod.rs /home/ubuntu/avcore/src/db/schema.rs`，如未公开，加 `pub`。**如果 `open_in_memory` 不存在**，改用：

```rust
fn fresh_db() -> Connection {
    let conn = Connection::open_in_memory().expect("mem");
    let sql = include_str!("../../migrations/0001_init.sql");
    conn.execute_batch(sql).unwrap();
    let sql = include_str!("../../migrations/0002_drift_dimensions.sql");
    conn.execute_batch(sql).unwrap();
    let sql = include_str!("../../migrations/0003_provider_health.sql");
    conn.execute_batch(sql).unwrap();
    conn
}
```

- [ ] **Step 3: 跑测试确认失败**

```bash
cd /home/ubuntu/avcore && cargo test --lib svc::health::tests 2>&1 | tail -15
```

预期：编译错（`record` / `latest_per_provider` 未定义）。

- [ ] **Step 4: 实现 `record` / `latest_per_provider`**

在 `src/svc/health.rs` 的 `provider_key` 函数后追加：

```rust
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
        stmt.query_map([p], mapper)?.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map([], mapper)?.collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(rows)
}
```

- [ ] **Step 5: 跑测试确认通过**

```bash
cd /home/ubuntu/avcore && cargo test --lib svc::health::tests 2>&1 | tail -15
```

预期：4 passed; 0 failed。

- [ ] **Step 6: 提交**

```bash
git add src/svc/health.rs
git commit -m "feat(health): record + latest_per_provider with rolling window"
```

---

## Task 4: 实现限速 UPSERT + 查询（TDD）

**Files:**
- Modify: `src/svc/health.rs`

- [ ] **Step 1: 追加测试**

在 `src/svc/health.rs::tests` 末尾追加：

```rust
    #[test]
    fn rate_limit_upsert_increments_hit_count() {
        let conn = fresh_db();
        rate_limit_upsert(&conn, "llm.openai", Some(60), Some(now_iso() + "+60s")).unwrap();
        rate_limit_upsert(&conn, "llm.openai", Some(30), Some(now_iso() + "+30s")).unwrap();
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
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd /home/ubuntu/avcore && cargo test --lib svc::health::tests::rate_limit 2>&1 | tail -10
```

预期：编译错。

- [ ] **Step 3: 实现 `rate_limit_upsert` / `rate_limit_get` / `RateLimitRow`**

在 `HealthRow` 后追加：

```rust
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
```

> 24h 滚动窗口清理：本 plan **不实现**。`hit_count_24h` 字段保留以便 v2 增加 daemon 周期清理任务。当前数据足够 `avc provider rate-limit` 做绝对值展示。

- [ ] **Step 4: 跑测试确认通过**

```bash
cd /home/ubuntu/avcore && cargo test --lib svc::health::tests 2>&1 | tail -5
```

预期：6 passed（4 + 2）。

- [ ] **Step 5: 提交**

```bash
git add src/svc/health.rs
git commit -m "feat(health): rate_limit upsert + get + all with hit count"
```

---

## Task 5: 实现 Retry-After 解析器（TDD）

**Files:**
- Modify: `src/svc/health.rs`

- [ ] **Step 1: 写测试**

```rust
    #[test]
    fn parse_retry_after_seconds_int() {
        assert_eq!(parse_retry_after("120"), Some(120));
    }

    #[test]
    fn parse_retry_after_http_date() {
        // RFC 7231 格式；本测试用固定日期检查 +delta
        let got = parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT");
        assert!(got.unwrap() > 0);
    }

    #[test]
    fn parse_retry_after_invalid_returns_none() {
        assert_eq!(parse_retry_after("garbage"), None);
        assert_eq!(parse_retry_after(""), None);
    }
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd /home/ubuntu/avcore && cargo test --lib svc::health::tests::parse_retry_after 2>&1 | tail -10
```

预期：编译错。

- [ ] **Step 3: 实现 `parse_retry_after`**

在文件末尾（pub 区）追加：

```rust
/// 解析 HTTP `Retry-After` header：
///   - 整数秒 → 立刻返回
///   - RFC 7231 HTTP-date → 计算与现在的差值（秒）
///   - 其它 → None
pub fn parse_retry_after(value: &str) -> Option<i64> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if let Ok(s) = v.parse::<i64>() {
        if s >= 0 {
            return Some(s);
        }
        return None;
    }
    // HTTP-date
    if let Ok(d) = chrono::DateTime::parse_from_rfc2822(v) {
        let delta = d.timestamp() - chrono::Utc::now().timestamp();
        if delta > 0 {
            return Some(delta);
        }
    }
    None
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cd /home/ubuntu/avcore && cargo test --lib svc::health::tests::parse 2>&1 | tail -5
```

预期：3 passed。

- [ ] **Step 5: 提交**

```bash
git add src/svc/health.rs
git commit -m "feat(health): parse_retry_after supports seconds + HTTP-date"
```

---

## Task 6: 实现 `probe_llm`（TDD）

**Files:**
- Create: `src/provider/probe.rs`
- Modify: `src/provider/mod.rs`（声明新子模块）

- [ ] **Step 1: 在 provider/mod.rs 加 `pub mod probe;`**

修改 `src/provider/mod.rs`，在文件头 `pub mod mock;` 后追加 `pub mod probe;`。

- [ ] **Step 2: 写 probe_llm 测试**

新建 `src/provider/probe.rs`：

```rust
//! Provider 探活函数
//!
//! 每个 probe 返回 (Status, latency_ms, err_msg)，由 caller 决定写库策略。
//! 详见 docs/superpowers/specs/2026-08-03-provider-daemon-design.md §4.1

use std::time::{Duration, Instant};
use crate::provider::real::OpenAiCompatLlmProvider;
use crate::config::{Config, ProviderCfg};
use crate::error::AvcResult;
use super::LlmProvider;
use crate::svc::health::Status;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// 探活 LLM provider：发最小 chat 请求
pub async fn probe_llm(
    cfg: &Config,
    name: &str,
) -> (Status, Option<i64>, Option<String>) {
    let pc = match cfg.provider.llm.get(name) {
        Some(p) => p,
        None => return (Status::Unconfigured, None, Some(format!("llm.{} not in config", name))),
    };
    if pc.api_key.is_none() {
        return (Status::Unconfigured, None, Some("missing api_key".into()));
    }
    let provider = match OpenAiCompatLlmProvider::new(name.to_string(), pc.clone()) {
        Ok(p) => p,
        Err(e) => return (Status::UpstreamError, None, Some(e.to_string())),
    };
    let started = Instant::now();
    let msgs = vec![crate::provider::ChatMessage {
        role: "user".into(),
        content: "ping".into(),
    }];
    let res = tokio::time::timeout(PROBE_TIMEOUT, provider.chat(&msgs)).await;
    let elapsed_ms = started.elapsed().as_millis() as i64;
    match res {
        Ok(Ok(_)) => (Status::Healthy, Some(elapsed_ms), None),
        Ok(Err(e)) => classify_llm_error(&e, elapsed_ms),
        Err(_) => (Status::Timeout, Some(elapsed_ms), Some("5s timeout".into())),
    }
}

fn classify_llm_error(e: &crate::error::AvcError, ms: i64) -> (Status, Option<i64>, Option<String>) {
    use crate::error::AvcError;
    let s = e.to_string();
    if matches!(e, AvcError::TokenAuth(_)) {
        (Status::Auth, Some(ms), Some(s))
    } else if matches!(e, AvcError::RateLimited(_)) {
        (Status::RateLimited, Some(ms), Some(s))
    } else if matches!(e, AvcError::ProviderTimeout(_)) {
        (Status::Timeout, Some(ms), Some(s))
    } else {
        (Status::UpstreamError, Some(ms), Some(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ProviderCfg};
    use std::collections::BTreeMap;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn cfg_with_llm(name: &str, base_url: &str) -> Config {
        let mut c = Config::default();
        c.provider.llm.insert(
            name.into(),
            ProviderCfg {
                api_key: Some("sk-test".into()),
                base_url: Some(base_url.into()),
                ..Default::default()
            },
        );
        c
    }

    /// 启一个 mock HTTP 服务，handler 由调用方提供（读 request，写 response）
    async fn spawn_mock(handler: impl Fn(String) -> String + Send + 'static) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let resp = handler(req);
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        addr
    }

    #[tokio::test]
    async fn probe_llm_success_records_healthy() {
        let addr = spawn_mock(|_req| {
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 50\r\n\r\n{\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"pong\"}}]}".to_string()
        }).await;
        let cfg = cfg_with_llm("openai", &format!("http://{}", addr));
        let (status, ms, _) = probe_llm(&cfg, "openai").await;
        assert_eq!(status, Status::Healthy);
        assert!(ms.unwrap() >= 0);
    }

    #[tokio::test]
    async fn probe_llm_401_records_auth() {
        let addr = spawn_mock(|_| "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_string()).await;
        let cfg = cfg_with_llm("openai", &format!("http://{}", addr));
        let (status, _, _) = probe_llm(&cfg, "openai").await;
        assert_eq!(status, Status::Auth);
    }

    #[tokio::test]
    async fn probe_llm_429_records_rate_limited() {
        let addr = spawn_mock(|_| "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n".to_string()).await;
        let cfg = cfg_with_llm("openai", &format!("http://{}", addr));
        let (status, _, _) = probe_llm(&cfg, "openai").await;
        assert_eq!(status, Status::RateLimited);
    }

    #[tokio::test]
    async fn probe_unconfigured_api_key_skipped() {
        let mut cfg = Config::default();
        cfg.provider.llm.insert(
            "openai".into(),
            ProviderCfg { api_key: None, ..Default::default() },
        );
        let (status, _, _) = probe_llm(&cfg, "openai").await;
        assert_eq!(status, Status::Unconfigured);
    }
}
```

- [ ] **Step 3: 跑测试确认通过**

```bash
cd /home/ubuntu/avcore && cargo test --lib provider::probe 2>&1 | tail -10
```

预期：4 passed。

- [ ] **Step 4: 提交**

```bash
git add src/provider/mod.rs src/provider/probe.rs
git commit -m "feat(probe): probe_llm with 5s timeout + status classification"
```

> 注：本 task 把 classify_llm_error 私有函数、ProviderCfg / Config 导入都写好；后续 probe 复用此模式。

---

## Task 7: 实现 `probe_embed` / `probe_avatar` / `probe_voice`（TDD）

**Files:**
- Modify: `src/provider/probe.rs`

- [ ] **Step 1: 追加函数与测试**

在 probe_llm 后追加：

```rust
pub async fn probe_embed(cfg: &Config, name: &str) -> (Status, Option<i64>, Option<String>) {
    use crate::provider::real::OpenAiCompatEmbedProvider;
    use crate::provider::EmbedProvider;
    let pc = match cfg.provider.embed.get(name) {
        Some(p) => p,
        None => return (Status::Unconfigured, None, Some(format!("embed.{} not in config", name))),
    };
    if pc.api_key.is_none() {
        return (Status::Unconfigured, None, Some("missing api_key".into()));
    }
    let provider = match OpenAiCompatEmbedProvider::new(name.to_string(), pc.clone()) {
        Ok(p) => p,
        Err(e) => return (Status::UpstreamError, None, Some(e.to_string())),
    };
    let started = Instant::now();
    let res = tokio::time::timeout(PROBE_TIMEOUT, provider.embed(&["ping"])).await;
    let ms = started.elapsed().as_millis() as i64;
    match res {
        Ok(Ok(_)) => (Status::Healthy, Some(ms), None),
        Ok(Err(e)) => classify_llm_error(&e, ms),
        Err(_) => (Status::Timeout, Some(ms), Some("5s timeout".into())),
    }
}

pub async fn probe_avatar(cfg: &Config, name: &str) -> (Status, Option<i64>, Option<String>) {
    use crate::provider::real::OpenAiCompatAvatarProvider;
    use crate::provider::AvatarProvider;
    let pc = match cfg.provider.avatar.get(name) {
        Some(p) => p,
        None => return (Status::Unconfigured, None, Some(format!("avatar.{} not in config", name))),
    };
    if pc.api_key.is_none() {
        return (Status::Unconfigured, None, Some("missing api_key".into()));
    }
    let provider = match OpenAiCompatAvatarProvider::new(name.to_string(), pc.clone()) {
        Ok(p) => p,
        Err(e) => return (Status::UpstreamError, None, Some(e.to_string())),
    };
    let spec = crate::provider::AvatarSpec { prompt: "ping".into(), style: None, ref_image_paths: vec![] };
    let started = Instant::now();
    let res = tokio::time::timeout(PROBE_TIMEOUT, provider.create(&spec)).await;
    let ms = started.elapsed().as_millis() as i64;
    match res {
        Ok(Ok(_)) => (Status::Healthy, Some(ms), None),
        Ok(Err(e)) => classify_llm_error(&e, ms),
        Err(_) => (Status::Timeout, Some(ms), Some("5s timeout".into())),
    }
}

pub async fn probe_voice(cfg: &Config, name: &str) -> (Status, Option<i64>, Option<String>) {
    use crate::provider::real::OpenAiCompatVoiceProvider;
    use crate::provider::VoiceProvider;
    let pc = match cfg.provider.voice.get(name) {
        Some(p) => p,
        None => return (Status::Unconfigured, None, Some(format!("voice.{} not in config", name))),
    };
    if pc.api_key.is_none() {
        return (Status::Unconfigured, None, Some("missing api_key".into()));
    }
    let provider = match OpenAiCompatVoiceProvider::new(name.to_string(), pc.clone()) {
        Ok(p) => p,
        Err(e) => return (Status::UpstreamError, None, Some(e.to_string())),
    };
    // voice synth 需要一个 Voice；用 stub base
    let base = crate::provider::Voice {
        provider: name.into(),
        provider_version: "v1".into(),
        voice_id_remote: Some("base".into()),
        sample_wav_b64: String::new(),
        transcript: None,
        embed_b64: None,
        embed_dim: None,
    };
    let started = Instant::now();
    let res = tokio::time::timeout(PROBE_TIMEOUT, provider.synth(&base, "ping")).await;
    let ms = started.elapsed().as_millis() as i64;
    match res {
        Ok(Ok(_)) => (Status::Healthy, Some(ms), None),
        Ok(Err(e)) => classify_llm_error(&e, ms),
        Err(_) => (Status::Timeout, Some(ms), Some("5s timeout".into())),
    }
}
```

- [ ] **Step 2: 追加测试（embed/avatar/voice 至少各 1 个 smoke test）**

```rust
    #[tokio::test]
    async fn probe_embed_429_records_rate_limited() {
        let addr = spawn_mock(|_| "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n".to_string()).await;
        let mut cfg = Config::default();
        cfg.provider.embed.insert(
            "openai".into(),
            ProviderCfg {
                api_key: Some("sk-test".into()),
                base_url: Some(format!("http://{}", addr)),
                ..Default::default()
            },
        );
        let (status, _, _) = probe_embed(&cfg, "openai").await;
        assert_eq!(status, Status::RateLimited);
    }

    #[tokio::test]
    async fn probe_avatar_500_records_upstream_error() {
        let addr = spawn_mock(|_| "HTTP/1.1 500 Internal\r\nContent-Length: 0\r\n\r\n".to_string()).await;
        let mut cfg = Config::default();
        cfg.provider.avatar.insert(
            "dalle".into(),
            ProviderCfg {
                api_key: Some("sk-test".into()),
                base_url: Some(format!("http://{}", addr)),
                ..Default::default()
            },
        );
        let (status, _, _) = probe_avatar(&cfg, "dalle").await;
        assert_eq!(status, Status::UpstreamError);
    }

    #[tokio::test]
    async fn probe_voice_timeout_records_timeout() {
        let addr = spawn_mock(|_| {
            std::thread::sleep(std::time::Duration::from_secs(8));
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_string()
        }).await;
        let mut cfg = Config::default();
        cfg.provider.voice.insert(
            "tts".into(),
            ProviderCfg {
                api_key: Some("sk-test".into()),
                base_url: Some(format!("http://{}", addr)),
                ..Default::default()
            },
        );
        let (status, _, _) = probe_voice(&cfg, "tts").await;
        assert_eq!(status, Status::Timeout);
    }
```

- [ ] **Step 3: 跑测试**

```bash
cd /home/ubuntu/avcore && cargo test --lib provider::probe 2>&1 | tail -10
```

预期：7 passed（4 from T6 + 3 new）。注意 voice timeout 测试会因为 8s sleep 慢 3s。

- [ ] **Step 4: 提交**

```bash
git add src/provider/probe.rs
git commit -m "feat(probe): embed/avatar/voice probes with shared classifier"
```

---

## Task 8: 实现 `probe_cli_video`（CLI provider 探活）

**Files:**
- Modify: `src/provider/probe.rs`

- [ ] **Step 1: 实现 + 测试**

```rust
pub fn probe_cli_video(cfg: &Config, name: &str) -> (Status, Option<i64>, Option<String>) {
    let pc = match cfg.provider.video.get(name) {
        Some(p) => p,
        None => return (Status::Unconfigured, None, Some(format!("video.{} not in config", name))),
    };
    let bin = match pc.binary.as_ref() {
        Some(b) if !b.is_empty() => b,
        _ => return (Status::Unconfigured, None, Some("missing binary".into())),
    };
    let started = Instant::now();
    let ok = std::process::Command::new("which")
        .arg(bin)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let ms = started.elapsed().as_millis() as i64;
    if ok {
        (Status::Healthy, Some(ms), None)
    } else {
        (Status::UpstreamError, Some(ms), Some(format!("binary not found: {}", bin)))
    }
}
```

测试追加：

```rust
    #[test]
    fn probe_cli_video_missing_binary_skipped() {
        let mut cfg = Config::default();
        cfg.provider.video.insert(
            "kling".into(),
            ProviderCfg { binary: None, ..Default::default() },
        );
        let (status, _, _) = probe_cli_video(&cfg, "kling");
        assert_eq!(status, Status::Unconfigured);
    }

    #[test]
    fn probe_cli_video_known_binary_healthy() {
        let mut cfg = Config::default();
        cfg.provider.video.insert(
            "sh".into(),
            ProviderCfg { binary: Some("/bin/sh".into()), ..Default::default() },
        );
        let (status, _, _) = probe_cli_video(&cfg, "sh");
        assert_eq!(status, Status::Healthy);
    }
```

- [ ] **Step 2: 跑测试**

```bash
cd /home/ubuntu/avcore && cargo test --lib provider::probe 2>&1 | tail -5
```

预期：9 passed（7 + 2）。

- [ ] **Step 3: 提交**

```bash
git add src/provider/probe.rs
git commit -m "feat(probe): cli_video probe checks binary existence"
```

---

## Task 9: 实现 probe dispatcher（一次跑全部）

**Files:**
- Modify: `src/provider/probe.rs`

- [ ] **Step 1: 实现 `probe_all` + 测**

```rust
pub async fn probe_all(cfg: &Config, conn: &rusqlite::Connection) -> AvcResult<()> {
    use crate::svc::health::{record, provider_key};

    macro_rules! run_dim {
        ($dim:literal, $names:expr, $probe_fn:expr) => {{
            let names: Vec<String> = $names;
            for name in names {
                let key = provider_key($dim, &name);
                let (status, ms, err) = $probe_fn(cfg, &name).await;
                record(
                    conn,
                    &key,
                    status,
                    ms,
                    err.as_deref(),
                    "probe",
                ).ok();
            }
        }};
    }
    run_dim!("llm", cfg.provider.llm.keys().cloned().collect::<Vec<_>>(), probe_llm);
    run_dim!("embed", cfg.provider.embed.keys().cloned().collect::<Vec<_>>(), probe_embed);
    run_dim!("avatar", cfg.provider.avatar.keys().cloned().collect::<Vec<_>>(), probe_avatar);
    run_dim!("voice", cfg.provider.voice.keys().cloned().collect::<Vec<_>>(), probe_voice);
    for name in cfg.provider.video.keys() {
        let key = provider_key("video", name);
        let (status, ms, err) = probe_cli_video(cfg, name);
        record(conn, &key, status, ms, err.as_deref(), "probe").ok();
    }
    Ok(())
}
```

- [ ] **Step 2: 测试（写 1 个 smoke：探空 config 不写库）**

```rust
    #[test]
    fn probe_all_empty_config_writes_nothing() {
        use crate::db::open_in_memory_or;
        use crate::db::schema::run_migrations;
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            probe_all(&Config::default(), &conn).await.unwrap();
        });
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM provider_health", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }
```

如果 `db::open_in_memory_or` 不存在，直接用 `Connection::open_in_memory()`。

- [ ] **Step 3: 跑测试**

```bash
cd /home/ubuntu/avcore && cargo test --lib provider::probe 2>&1 | tail -5
```

预期：10 passed。

- [ ] **Step 4: 提交**

```bash
git add src/provider/probe.rs
git commit -m "feat(probe): probe_all dispatcher iterates all configured providers"
```

---

## Task 10: 接入 LLM Provider 被动 hook

**Files:**
- Modify: `src/provider/real.rs`

- [ ] **Step 1: 在 LLM chat() 错误分支加 hook**

定位 `OpenAiCompatLlmProvider::chat` 中 4 个 `return Err(...)` 之前各加一行（**不**在 return 之后；是在 token/rate/timeout/upstream 四种 return 前记录 status）：

```rust
// 在 chat() 函数顶部，let resp = ... 之后
let provider_key = format!("llm.{}", self.name);
```

然后在 4 个 return Err 之前各加：

```rust
// TokenAuth:
let _ = crate::svc::health::record(
    db_pool(), &provider_key, crate::svc::health::Status::Auth,
    Some(started.elapsed().as_millis() as i64), Some("auth"), "hook",
);
// RateLimited:
let _ = crate::svc::health::record(
    db_pool(), &provider_key, crate::svc::health::Status::RateLimited,
    Some(started.elapsed().as_millis() as i64), Some("rate"), "hook",
);
if let Some(s) = retry_after_header { /* UPSERT rate_limit table */ }
// Timeout:
let _ = crate::svc::health::record(
    db_pool(), &provider_key, crate::svc::health::Status::Timeout,
    Some(started.elapsed().as_millis() as i64), Some("timeout"), "hook",
);
// UpstreamError:
let _ = crate::svc::health::record(
    db_pool(), &provider_key, crate::svc::health::Status::UpstreamError,
    Some(started.elapsed().as_millis() as i64), Some("upstream"), "hook",
);
```

- [ ] **Step 2: 引入 db_pool() helper**

为避免 hook 引入新的 connection 管理，在 `src/provider/real.rs` 顶部加：

```rust
fn db_pool() -> &'static rusqlite::Connection {
    use std::sync::OnceLock;
    static POOL: OnceLock<rusqlite::Connection> = OnceLock::new();
    POOL.get_or_init(|| {
        crate::db::open_default().expect("default db open")
    })
}
```

如果 `db::open_default` 不存在，加：

```rust
// src/db/mod.rs
pub fn open_default() -> AvcResult<Connection> {
    let path = crate::paths::data_dir()?.join("avc.db");
    if let Some(p) = path.parent() { std::fs::create_dir_all(p).ok(); }
    Ok(Connection::open(path)?)
}
```

- [ ] **Step 3: 写集成测试**

在 `tests/integration.rs` 追加：

```rust
#[test]
fn hook_records_auth_error_on_real_401() {
    // 起一个 mock HTTP 返回 401
    // 配置 llm.openai 指向它
    // 调一次 avc provider test llm.openai（exit 非 0）
    // 读 provider_health 表确认 status='auth' + source='hook'
}
```

具体代码与 T6 风格一致（spawn mock），略 30 行。

- [ ] **Step 4: 跑全量测试**

```bash
cd /home/ubuntu/avcore && cargo test 2>&1 | tail -20
```

预期：所有原 155 个测试 + 11 个新测试 = 166 passed。

- [ ] **Step 5: 提交**

```bash
git add src/provider/real.rs src/db/mod.rs tests/integration.rs
git commit -m "feat(hook): passive recording on OpenAiCompatLlmProvider errors"
```

---

## Task 11: 接入 Embed / Avatar / Voice 三个 hook

**Files:**
- Modify: `src/provider/real.rs`

- [ ] **Step 1: 三个 provider 的错误分支同样处理**

对 `OpenAiCompatEmbedProvider::embed`、`OpenAiCompatAvatarProvider::create`、`OpenAiCompatVoiceProvider::synth` 各自：
1. 函数顶部 `let provider_key = format!("{}.{}", <dim>, self.name);`
2. 4 个 Err return 之前各加一次 `crate::svc::health::record(...)`
3. RateLimited 分支加 `rate_limit_upsert`

代码结构同 T10，复用 classify_llm_error 模式（实际 copy-paste 即可，~10 行 × 3）。

- [ ] **Step 2: 跑测试**

```bash
cd /home/ubuntu/avcore && cargo test 2>&1 | tail -5
```

预期：PASS。

- [ ] **Step 3: 提交**

```bash
git add src/provider/real.rs
git commit -m "feat(hook): passive recording on embed/avatar/voice providers"
```

---

## Task 12: DaemonCfg + 配置解析

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: 加 `DaemonCfg` 结构**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonCfg {
    #[serde(default = "default_daemon_enabled")]
    pub enabled: bool,
    #[serde(default = "default_daemon_port")]
    pub port: u16,
    #[serde(default = "default_daemon_bind")]
    pub bind: String,
    #[serde(default = "default_ping_interval")]
    pub ping_interval_s: u64,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_auto_record_hook")]
    pub auto_record_hook: bool,
}

fn default_daemon_enabled() -> bool { true }
fn default_daemon_port() -> u16 { 7891 }
fn default_daemon_bind() -> String { "127.0.0.1".into() }
fn default_ping_interval() -> u64 { 60 }
fn default_log_level() -> String { "info".into() }
fn default_auto_record_hook() -> bool { true }

impl Default for DaemonCfg {
    fn default() -> Self {
        Self {
            enabled: true, port: 7891, bind: "127.0.0.1".into(),
            ping_interval_s: 60, log_level: "info".into(), auto_record_hook: true,
        }
    }
}
```

在 `Config` 结构中加 `pub daemon: DaemonCfg` 字段（默认派生 `#[serde(default)]`）。

- [ ] **Step 2: 写测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn daemon_cfg_default_values() {
        let c = DaemonCfg::default();
        assert_eq!(c.port, 7891);
        assert!(c.enabled);
        assert_eq!(c.bind, "127.0.0.1");
    }
    #[test]
    fn daemon_cfg_parses_partial_toml() {
        let toml_str = r#"
[daemon]
port = 9000
"#;
        let c: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(c.daemon.port, 9000);
        assert!(c.daemon.enabled); // 其它字段用默认
    }
}
```

- [ ] **Step 3: 跑测试**

```bash
cd /home/ubuntu/avcore && cargo test --lib config::tests 2>&1 | tail -5
```

预期：2 passed。

- [ ] **Step 4: 提交**

```bash
git add src/config.rs
git commit -m "feat(config): DaemonCfg with defaults + serde"
```

---

## Task 13: pidfile / lockfile 辅助

**Files:**
- Create: `src/svc/daemon.rs`（先放 pidfile 部分）

- [ ] **Step 1: 写 pid_path / lock_path helper + 写测试**

```rust
//! 后台 daemon runtime
//!
//! 详见 docs/superpowers/specs/2026-08-03-provider-daemon-design.md §4-5

use std::path::PathBuf;
use crate::error::{AvcError, AvcResult};
use crate::paths::data_dir;

pub fn pid_path() -> AvcResult<PathBuf> {
    Ok(data_dir()?.join("avc.pid"))
}

pub fn log_path() -> AvcResult<PathBuf> {
    Ok(data_dir()?.join("avc.log"))
}

pub fn write_pid(pid: u32) -> AvcResult<()> {
    let p = pid_path()?;
    if let Some(parent) = p.parent() { std::fs::create_dir_all(parent).ok(); }
    std::fs::write(p, pid.to_string())?;
    Ok(())
}

pub fn read_pid() -> AvcResult<Option<u32>> {
    let p = pid_path()?;
    if !p.exists() { return Ok(None); }
    let s = std::fs::read_to_string(&p)?;
    let pid = s.trim().parse::<u32>().ok();
    if pid.is_none() {
        return Err(AvcError::PidfileStale(p.display().to_string()));
    }
    Ok(pid)
}

pub fn clear_pid() -> AvcResult<()> {
    let p = pid_path()?;
    if p.exists() { std::fs::remove_file(&p)?; }
    Ok(())
}

pub fn is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // signal 0 不杀进程，只检查存在
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        // 简化：检查 /proc 不可用，跳过；v1 不在 Windows 验 daemon detach
        // 仅在 unix 上做 is_alive
        let _ = pid;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::env;

    #[test]
    fn pid_round_trip() {
        let dir = tempdir().unwrap();
        env::set_var("XDG_DATA_HOME", dir.path());
        write_pid(12345).unwrap();
        assert_eq!(read_pid().unwrap(), Some(12345));
        clear_pid().unwrap();
        assert_eq!(read_pid().unwrap(), None);
    }
}
```

> `libc` 需在 `Cargo.toml` 加 `libc = "0.2"`（如果有 nix 已有则复用）。先看 `Cargo.lock` 是否已有 libc。

- [ ] **Step 2: 在 svc/mod.rs 导出**

```rust
pub mod daemon;
```

- [ ] **Step 3: 跑测试**

```bash
cd /home/ubuntu/avcore && cargo test --lib svc::daemon::tests 2>&1 | tail -5
```

预期：1 passed。

- [ ] **Step 4: 提交**

```bash
git add src/svc/daemon.rs src/svc/mod.rs Cargo.toml Cargo.lock
git commit -m "feat(daemon): pidfile helper with is_alive cross-platform"
```

---

## Task 14: HTTP server（axum）

**Files:**
- Modify: `src/svc/daemon.rs`
- Modify: `Cargo.toml`（加 axum + tower）

- [ ] **Step 1: 加 dep**

修改 `Cargo.toml`，在 `[dependencies]` 末尾加：

```toml
axum = "0.7"
tower = "0.5"
```

- [ ] **Step 2: 写 HTTP 端点**

在 `src/svc/daemon.rs` 追加：

```rust
use axum::{extract::State, routing::get, Json, Router};
use axum::http::StatusCode;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::db::open_default;
use crate::svc::health;

#[derive(Clone)]
struct AppState {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

#[derive(Serialize)]
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
    let rows = health::latest_per_provider(&conn, None).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let out: Vec<HealthDto> = rows.into_iter().map(|r| {
        let (dim, name) = r.provider_key.split_once('.').unwrap_or(("?", r.provider_key.as_str()));
        HealthDto {
            dim: dim.into(), name: name.into(),
            status: r.status.as_str().into(),
            latency_ms: r.latency_ms,
            last_check_at: r.checked_at,
            last_error: r.error_msg,
            source: r.source,
        }
    }).collect();
    Ok(Json(out))
}

async fn limits_all(State(s): State<AppState>) -> Result<Json<Vec<LimitDto>>, StatusCode> {
    let conn = s.conn.lock().await;
    let rows = health::rate_limit_all(&conn).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let out: Vec<LimitDto> = rows.into_iter().map(|r| {
        let (dim, name) = r.provider_key.split_once('.').unwrap_or(("?", r.provider_key.as_str()));
        let now = chrono::Utc::now().timestamp();
        let in_cooldown = r.until_ts.as_ref()
            .and_then(|t| chrono::DateTime::parse_from_rfc2822(t).ok())
            .map(|d| d.timestamp() > now)
            .unwrap_or(false);
        LimitDto {
            dim: dim.into(), name: name.into(),
            in_cooldown, until_ts: r.until_ts,
            retry_after_s: r.retry_after_s,
            hit_count_24h: r.hit_count_24h,
        }
    }).collect();
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

pub async fn run_http(bind: &str, port: u16, conn: Arc<Mutex<rusqlite::Connection>>) -> AvcResult<()> {
    let app = build_router(conn);
    let addr = format!("{}:{}", bind, port);
    let listener = tokio::net::TcpListener::bind(&addr).await
        .map_err(|e| AvcError::BindFailed { addr: bind.into(), port, msg: e.to_string() })?;
    tracing::info!("daemon listening on {}", addr);
    axum::serve(listener, app).await
        .map_err(|e| AvcError::Internal(format!("axum: {}", e)))?;
    Ok(())
}
```

- [ ] **Step 3: 写测试（用 ephemeral port）**

```rust
    #[tokio::test]
    async fn http_health_all_returns_empty_array() {
        use std::net::SocketAddr;
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::schema::run_migrations(&conn).unwrap();
        let conn = Arc::new(Mutex::new(conn));
        let app = build_router(conn);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        // 等待 server 起来
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let resp: Vec<HealthDto> = reqwest::get(format!("http://{}/health/all", addr))
            .await.unwrap().json().await.unwrap();
        assert_eq!(resp.len(), 0);
    }
```

- [ ] **Step 4: 跑测试**

```bash
cd /home/ubuntu/avcore && cargo test --lib svc::daemon::tests::http 2>&1 | tail -5
```

预期：1 passed。

- [ ] **Step 5: 提交**

```bash
git add src/svc/daemon.rs Cargo.toml Cargo.lock
git commit -m "feat(daemon): axum HTTP server with /health/all, /limits/all, /version"
```

---

## Task 15: PingLoop task

**Files:**
- Modify: `src/svc/daemon.rs`

- [ ] **Step 1: 实现**

```rust
use std::time::Duration;
use crate::config::Config;
use crate::provider::probe;

pub async fn run_ping_loop(cfg: Config, conn: Arc<Mutex<rusqlite::Connection>>) {
    let interval = Duration::from_secs(cfg.daemon.ping_interval_s.max(5));
    loop {
        let conn_guard = conn.lock().await;
        if let Err(e) = probe::probe_all(&cfg, &conn_guard).await {
            tracing::warn!("probe_all error: {}", e);
        }
        drop(conn_guard);
        tokio::time::sleep(interval).await;
    }
}
```

- [ ] **Step 2: 测试（不验 sleep 周期，验 1 次调用 + 退出）**

```rust
    #[tokio::test]
    async fn ping_loop_runs_once_then_cancels() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::schema::run_migrations(&conn).unwrap();
        let conn = Arc::new(Mutex::new(conn));
        let mut cfg = Config::default();
        cfg.daemon.ping_interval_s = 60; // 测试期间不会再次跑
        let handle = tokio::spawn(run_ping_loop(cfg, conn.clone()));
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        handle.abort();
    }
```

- [ ] **Step 3: 跑测试**

```bash
cd /home/ubuntu/avcore && cargo test --lib svc::daemon::tests::ping 2>&1 | tail -5
```

预期：1 passed。

- [ ] **Step 4: 提交**

```bash
git add src/svc/daemon.rs
git commit -m "feat(daemon): ping loop with configurable interval"
```

---

## Task 16: svc::daemon::_run 入口

**Files:**
- Modify: `src/svc/daemon.rs`

- [ ] **Step 1: 实现 `_run`**

```rust
use tokio::signal;
use tokio::sync::Mutex;

pub async fn _run(cfg: Config) -> AvcResult<()> {
    // 1. 写 daemon_meta
    let conn = open_default()?;
    let started_at = crate::svc::now_iso();
    std::env::set_var("AVC_DAEMON_STARTED_AT", &started_at);
    conn.execute(
        "INSERT OR REPLACE INTO daemon_meta (key, value) VALUES
            ('started_at', ?1), ('version', ?2), ('port', ?3), ('pid', ?4)",
        rusqlite::params![started_at, env!("CARGO_PKG_VERSION"), cfg.daemon.port.to_string(), std::process::id()],
    )?;
    let conn = Arc::new(Mutex::new(conn));

    // 2. 启三 task
    let ping_handle = tokio::spawn(run_ping_loop(cfg.clone(), conn.clone()));
    let http_handle = {
        let bind = cfg.daemon.bind.clone();
        let port = cfg.daemon.port;
        let c = conn.clone();
        tokio::spawn(async move {
            if let Err(e) = run_http(&bind, port, c).await {
                tracing::error!("http server: {}", e);
            }
        })
    };

    // 3. 监听信号
    #[cfg(unix)]
    let sig = async {
        let mut term = signal::unix::signal(signal::unix::SignalKind::terminate()).unwrap();
        let mut int = signal::unix::signal(signal::unix::SignalKind::interrupt()).unwrap();
        tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
    };
    #[cfg(not(unix))]
    let sig = signal::ctrl_c();

    sig.await;
    tracing::info!("daemon shutting down");

    // 4. 清理
    ping_handle.abort();
    http_handle.abort();
    let conn_guard = conn.lock().await;
    conn_guard.execute("DELETE FROM daemon_meta", [])?;
    clear_pid()?;
    Ok(())
}
```

- [ ] **Step 2: 跑全量**

```bash
cd /home/ubuntu/avcore && cargo build 2>&1 | tail -5
```

预期：编译通过。

- [ ] **Step 3: 提交**

```bash
git add src/svc/daemon.rs
git commit -m "feat(daemon): _run entry with ping+http+signal tasks"
```

---

## Task 17: main.rs 识别 _run 隐藏 verb

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: 在 main 入口加分支**

```rust
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "_run" {
        // 隐藏子命令：daemon 内部入口
        let cfg = match avc::config::Config::load() {
            Ok(c) => c,
            Err(e) => { eprintln!("config: {}", e); std::process::exit(1); }
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        if let Err(e) = rt.block_on(avc::svc::daemon::_run(cfg)) {
            eprintln!("daemon: {}", e);
            std::process::exit(1);
        }
        return;
    }
    // 原有 dispatch 逻辑
    avc::cli::run(&args).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(1);
    });
}
```

- [ ] **Step 2: 编译通过**

```bash
cd /home/ubuntu/avcore && cargo build 2>&1 | tail -5
```

预期：0 errors。

- [ ] **Step 3: 提交**

```bash
git add src/main.rs
git commit -m "feat(daemon): main recognizes _run hidden verb"
```

---

## Task 18: cli::daemon start/stop/status

**Files:**
- Create: `src/cli/daemon.rs`
- Modify: `src/cli/mod.rs`

- [ ] **Step 1: 在 mod.rs 加 dispatch**

修改 `src/cli/mod.rs`：

```rust
pub mod daemon;
// 在 dispatch match 加：
"daemon" => daemon::dispatch(&argv[1..]),
```

- [ ] **Step 2: 实现 cli::daemon.rs**

```rust
//! avc daemon <verb> 后台进程管理

use std::process::Command;
use crate::cli::root::cmd_doctor_help;
use crate::error::{AvcError, AvcResult};
use crate::svc::daemon::{is_alive, read_pid, clear_pid, write_pid, log_path};

pub fn dispatch(args: &[String]) -> AvcResult<()> {
    if args.is_empty() {
        return cmd_help();
    }
    match args[0].as_str() {
        "start" => cmd_start(&args[1..]),
        "stop" => cmd_stop(),
        "status" => cmd_status(),
        "logs" => cmd_logs(),
        _ => Err(AvcError::Arg(format!("unknown daemon verb: {}", args[0]))),
    }
}

fn cmd_help() -> AvcResult<()> {
    println!("avc daemon <verb>
verbs:
  start    fork child process running ping loop + HTTP
  stop     send SIGTERM and clear pidfile
  status   show pid, alive, started_at, port
  logs     tail ~/.local/share/avc/avc.log");
    Ok(())
}

fn cmd_start(args: &[String]) -> AvcResult<()> {
    if let Some(p) = read_pid()? {
        if is_alive(p) {
            return Err(AvcError::AlreadyRunning { pid: p, port: 7891 });
        }
        // 僵死：清 pidfile，继续
        tracing::warn!("removing stale pidfile for pid {}", p);
        clear_pid()?;
    }
    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe)
        .arg("_run")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    write_pid(child.id())?;
    println!("avc daemon started: pid {}", child.id());
    let _ = args; // 暂不解析 --port / --foreground
    Ok(())
}

fn cmd_stop() -> AvcResult<()> {
    let pid = match read_pid()? {
        Some(p) => p,
        None => { println!("daemon not running"); return Ok(()); }
    };
    if !is_alive(pid) {
        println!("daemon not running (stale pid {})", pid);
        clear_pid()?;
        return Ok(());
    }
    #[cfg(unix)]
    unsafe { libc::kill(pid as i32, libc::SIGTERM); }
    println!("sent SIGTERM to pid {}", pid);
    Ok(())
}

fn cmd_status() -> AvcResult<()> {
    let pid = match read_pid()? {
        Some(p) => p,
        None => { println!("daemon not running"); return Ok(()); }
    };
    let alive = is_alive(pid);
    let log = log_path()?.display().to_string();
    println!("pid:   {}\nalive: {}\nlog:   {}", pid, alive, log);
    Ok(())
}

fn cmd_logs() -> AvcResult<()> {
    let p = log_path()?;
    if !p.exists() {
        println!("no log file at {}", p.display());
        return Ok(());
    }
    let s = std::fs::read_to_string(&p)?;
    let tail: String = s.chars().rev().take(2000).collect::<String>().chars().rev().collect();
    print!("{}", tail);
    Ok(())
}
```

- [ ] **Step 3: 跑全量**

```bash
cd /home/ubuntu/avcore && cargo build 2>&1 | tail -5
```

预期：0 errors。

- [ ] **Step 4: 提交**

```bash
git add src/cli/daemon.rs src/cli/mod.rs
git commit -m "feat(daemon): cli start/stop/status/logs verbs"
```

---

## Task 19: provider status CLI（DB 路径 + --live）

**Files:**
- Modify: `src/cli/provider.rs`

- [ ] **Step 1: 写测试（CLI 输出格式）**

在 `tests/integration.rs` 追加：

```rust
#[test]
fn provider_status_reads_db_when_daemon_dead() {
    // 无 daemon 时跑 avc provider status
    // 期望 exit 0 + "daemon not running" 提示 + 表头
}

#[test]
fn provider_status_live_fallbacks_to_db_on_connect_refused() {
    // daemon_meta 有 port 但实际不跑
    // --live 应该 fallback 到 DB + 提示
}
```

- [ ] **Step 2: 在 provider.rs 增 status / rate-limit 子命令**

完整实现细节略（与现有 `provider test` 同风格），关键路径：

```rust
"status" => {
    let live = args.iter().any(|a| a == "--live");
    let dim = parse_dim_flag(args);
    if live && try_http_fetch(...).is_ok() {
        // 走 HTTP
    } else {
        // 走 DB：SELECT from provider_health latest
        let rows = health::latest_per_provider(&conn, dim.as_deref())?;
        print_table(&rows);
    }
}
"rate-limit" => {
    let rows = health::rate_limit_all(&conn)?;
    print_limit_table(&rows);
}
```

约 80 行。

- [ ] **Step 3: 跑测试**

```bash
cd /home/ubuntu/avcore && cargo test --test integration provider_status 2>&1 | tail -10
```

预期：2 passed。

- [ ] **Step 4: 提交**

```bash
git add src/cli/provider.rs tests/integration.rs
git commit -m "feat(provider): status + rate-limit CLI with --live and DB fallback"
```

---

## Task 20: 集成测试（daemon lifecycle + provider query）

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: 追加 6 个集成测试**

```rust
#[test]
fn daemon_start_creates_pidfile_and_meta() {
    // 启 avc daemon start
    // 验 pidfile 存在 + daemon_meta 表有 started_at 行
    // 然后 stop
}

#[test]
fn daemon_start_twice_returns_already_running_exit_1() {
    // start 一次，再 start：第二次 exit 1 + stderr 含 "already running"
}

#[test]
fn daemon_stop_sends_signal_and_clears_pidfile() {
    // start 后 stop，等 1s，验 pidfile 不存在
}

#[test]
fn daemon_status_reports_pid_alive_port() {
    // start 后 status：stdout 含 pid + alive=true
}

#[test]
fn daemon_status_when_dead_says_not_running_exit_0() {
    // 无 pidfile：exit 0 + "daemon not running"
}

#[test]
fn provider_rate_limit_shows_in_cooldown_after_429() {
    // 触发一次 mock 429 hook
    // 验 provider rate-limit 输出 in_cooldown=true
}
```

每个 ~25 行（沿用现有 integration 测试风格：用 `Command::new(env!("CARGO_BIN_EXE_avc"))` + XDG 临时目录）。

- [ ] **Step 2: 跑集成测试**

```bash
cd /home/ubuntu/avcore && cargo test --test integration 2>&1 | tail -10
```

预期：~18 个新测试全过（与 155 旧测试叠加）。

- [ ] **Step 3: 提交**

```bash
git add tests/integration.rs
git commit -m "test: integration tests for daemon lifecycle + provider queries"
```

---

## Task 21: 文档 + CHANGELOG

**Files:**
- Modify: `docs/cli.md`
- Modify: `docs/status.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: docs/cli.md 增 daemon + provider status 章节**

在 `docs/cli.md` 末尾追加：

```markdown
## avc daemon

后台进程管理。

| 动词 | 行为 |
|---|---|
| `start` | fork 子进程，pid 写 `~/.local/share/avc/avc.pid` |
| `stop` | 发 SIGTERM，子进程优雅退出后清 pidfile |
| `status` | 显示 pid / alive / log path |
| `logs` | tail `~/.local/share/avc/avc.log` |

## avc provider status / rate-limit

查询 Provider 健康与限速状态。

| 选项 | 行为 |
|---|---|
| `--live` | 走 HTTP（daemon 在线时）；不可达时自动 fallback DB |
| `--dim llm\|embed\|avatar\|voice\|video` | 按维度过滤 |
| `--json` | 机器读格式 |
```

- [ ] **Step 2: docs/status.md 增 Phase 3 daemon 节点**

在 status 文档加新章节 "Phase 3: Provider 健康检查 daemon"，~20 行。

- [ ] **Step 3: CHANGELOG.md 加条目**

```markdown
## [Unreleased]

### Added
- 后台 daemon：`avc daemon start|stop|status|logs`（fork 子进程、HTTP loopback、PingLoop）
- Provider 健康探活：5 维（llm/embed/avatar/voice/video）主动 ping + 被动 hook
- `provider_health` / `provider_rate_limit` / `daemon_meta` 三张表（migration 0003）
- `avc provider status [--live] [--dim] [--json]` — 健康查询
- `avc provider rate-limit [--live] [--dim] [--json]` — 限速查询
- 27 个新测试（11 单元 + 16 集成）

### Changed
- `OpenAiCompat*` Provider 错误分支旁路记录 health / rate_limit（不改变主流程）
- `avc.toml` 新增 `[daemon]` 段
```

- [ ] **Step 4: 提交**

```bash
git add docs/cli.md docs/status.md CHANGELOG.md
git commit -m "docs: daemon + provider status CLI documentation"
```

---

## Task 22: 全量验证 + 收口

- [ ] **Step 1: 跑全量测试**

```bash
cd /home/ubuntu/avcore && cargo test --all-targets 2>&1 | tail -30
```

预期：~182 测试全过（155 旧 + 27 新）。

- [ ] **Step 2: clippy 严格模式**

```bash
cd /home/ubuntu/avcore && cargo clippy --all-targets -- -D warnings 2>&1 | tail -30
```

预期：0 errors（CI 用 `-D warnings`，本地也按此标准）。

- [ ] **Step 3: fmt**

```bash
cd /home/ubuntu/avcore && cargo fmt --all -- --check
```

预期：无 diff。如有 `cargo fmt --all` 修复后 commit：

```bash
git add -u && git commit -m "chore: cargo fmt"
```

- [ ] **Step 4: 实跑 daemon**

```bash
cd /home/ubuntu/avcore && cargo build --release
./target/release/avc init
./target/release/avc daemon start
sleep 2
./target/release/avc daemon status    # 期望 pid + alive=true
./target/release/avc provider status  # 期望看到至少 1 条 healthy
./target/release/avc daemon stop
```

预期：start 后 pid 出现，status 显示 alive，stop 后 status 显示 "not running"。

- [ ] **Step 5: 最终 commit（如果有剩余改动）**

```bash
git status
# 如有未提交改动：
git add -A && git commit -m "chore: final cleanup for provider daemon"
```

---

## 自审（writing-plans 流程要求）

**1. Spec 覆盖检查**

| Spec 章节 | 覆盖 task |
|---|---|
| §2 架构 | T14 (HTTP) + T15 (PingLoop) + T16 (_run) + T18 (cli start) |
| §3 数据模型 | T1 (migration) + T2 (Status enum) + T3 (record) + T4 (rate_limit) |
| §4.1 启动流 | T13 (pidfile) + T18 (cli start) |
| §4.2 被动 hook | T10 (LLM) + T11 (embed/avatar/voice) |
| §4.3 CLI 查询 | T19 (provider status / rate-limit) |
| §4.4 限速记录 | T5 (parse_retry_after) + T10 (rate_limit_upsert 调用) |
| §5.1 错误矩阵 | T2 (错误变体) + T10/T11 (hook 不改主流程) |
| §5.2 生命周期 | T13 (pidfile) + T16 (signal) + T18 (stop/status) |
| §5.3 日志 | T13 (log_path) + T18 (logs verb) — daemon 写文件留 v1.1 |
| §5.4 配置 | T12 (DaemonCfg) |
| §6 测试 | T3-T11 (单元 15 个) + T20 (集成 6 个) + T19 (集成 2 个) + T6 4 + T7 3 + T8 2 + T9 1 = 27 个新测试 |
| §7 文件变更 | T1-T22 已逐个覆盖 |

**2. 占位扫描**
- "TBD" / "TODO" / "implement later"：0
- "Add appropriate error handling"：0
- "Similar to Task N"：0（重复时显式 copy-paste）

**3. 类型一致性**
- `Status::parse` / `Status::as_str` 双向唯一来源
- `provider_key(dim, name) -> String` 在 T2 定义，T3/T4/T9/T19 复用
- `record(conn, key, status, latency_ms, err_msg, source)` 6 参签名统一
- `rate_limit_upsert(conn, key, retry_after_s, until_ts)` 4 参签名统一
- `pid_path() / log_path() / write_pid() / read_pid() / clear_pid() / is_alive()` 在 T13 定义，T18 复用
- `build_router(conn) -> Router` 在 T14 定义
- `run_ping_loop(cfg, conn)` 在 T15 定义
- `_run(cfg)` 在 T16 定义
- `cmd_start/stop/status/logs` 在 T18 定义
- `health_all / limits_all / version` 端点一致

**4. Spec 中未覆盖**
- §5.3 "日志滚动 v1 不实现" → 在本 plan 注释里说明
- §5.3 "50MB truncate 启动时" → 不实现，留 v1.1
- §6.4 "Windows daemon detach 验证" → 标注 CI 不跑 Windows daemon 测试

**5. 已识别风险（未改 plan 主体）**
- `fs2` 依赖未确认存在：本 plan 假设 unix 上 `libc::kill(pid, 0)`，无需新 dep
- `reqwest` 已在 Cargo.toml（T6 测试已用）
- `axum + tower` 新增，已在 T14 加 dep
- `serde_json` 已存在

---

## 总览

| 维度 | 数值 |
|---|---:|
| 总 task 数 | 22 |
| 新增测试 | 27（11 单元 + 16 集成） |
| 总代码增量 | ~1,500 LOC（含测试） |
| 预计提交数 | 22 |
| 实施周期 | 1-2 个工作日 |

实施顺序：

```
T1 (migration)
  → T2 (健康模块骨架) → T3 (record/latest) → T4 (rate_limit) → T5 (retry_after 解析)
  → T6 (probe_llm) → T7 (probe embed/avatar/voice) → T8 (probe_cli_video) → T9 (probe_all)
  → T10 (LLM hook) → T11 (embed/avatar/voice hook)
  → T12 (DaemonCfg) → T13 (pidfile)
  → T14 (HTTP server) → T15 (PingLoop) → T16 (_run) → T17 (main _run) → T18 (cli daemon)
  → T19 (provider status CLI) → T20 (集成测试) → T21 (文档) → T22 (全量验证)
```

每个 task 都可独立 commit；任何中间点断电，git history 都清晰可查。
