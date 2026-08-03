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
