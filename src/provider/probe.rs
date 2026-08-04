//! Provider 探活函数
//!
//! 每个 probe 返回 (Status, latency_ms, err_msg)，由 caller 决定写库策略。
//! 详见 docs/superpowers/specs/2026-08-03-provider-daemon-design.md §4.1

use std::time::{Duration, Instant};

use crate::config::Config;
use crate::error::{AvcError, AvcResult};
use crate::provider::real::OpenAiCompatLlmProvider;
use crate::provider::LlmProvider;
use crate::svc::health::Status;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// 把 `AvcError` 归类为 `Status`：
///   - `TokenAuth` → `Auth`
///   - `RateLimited` → `RateLimited`
///   - `ProviderTimeout` → `Timeout`
///   - 其它 → `UpstreamError`
pub(crate) fn classify_llm_error(e: &AvcError, ms: i64) -> (Status, Option<i64>, Option<String>) {
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

/// 探活 LLM provider：发最小 chat 请求
pub async fn probe_llm(cfg: &Config, name: &str) -> (Status, Option<i64>, Option<String>) {
    let pc = match cfg.provider.llm.get(name) {
        Some(p) => p,
        None => {
            return (
                Status::Unconfigured,
                None,
                Some(format!("llm.{} not in config", name)),
            )
        }
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

/// 探活 Embed provider：发最小 embed(["ping"]) 请求
pub async fn probe_embed(cfg: &Config, name: &str) -> (Status, Option<i64>, Option<String>) {
    use crate::provider::real::OpenAiCompatEmbedProvider;
    use crate::provider::EmbedProvider;
    let pc = match cfg.provider.embed.get(name) {
        Some(p) => p,
        None => {
            return (
                Status::Unconfigured,
                None,
                Some(format!("embed.{} not in config", name)),
            )
        }
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
    let elapsed_ms = started.elapsed().as_millis() as i64;
    match res {
        Ok(Ok(_)) => (Status::Healthy, Some(elapsed_ms), None),
        Ok(Err(e)) => classify_llm_error(&e, elapsed_ms),
        Err(_) => (Status::Timeout, Some(elapsed_ms), Some("5s timeout".into())),
    }
}

/// 探活 Avatar provider：发最小 create("ping") 请求
pub async fn probe_avatar(cfg: &Config, name: &str) -> (Status, Option<i64>, Option<String>) {
    use crate::provider::real::OpenAiCompatAvatarProvider;
    use crate::provider::AvatarProvider;
    let pc = match cfg.provider.avatar.get(name) {
        Some(p) => p,
        None => {
            return (
                Status::Unconfigured,
                None,
                Some(format!("avatar.{} not in config", name)),
            )
        }
    };
    if pc.api_key.is_none() {
        return (Status::Unconfigured, None, Some("missing api_key".into()));
    }
    let provider = match OpenAiCompatAvatarProvider::new(name.to_string(), pc.clone()) {
        Ok(p) => p,
        Err(e) => return (Status::UpstreamError, None, Some(e.to_string())),
    };
    let spec = crate::provider::AvatarSpec {
        prompt: "ping".into(),
        style: None,
        ref_image_paths: vec![],
    };
    let started = Instant::now();
    let res = tokio::time::timeout(PROBE_TIMEOUT, provider.create(&spec)).await;
    let elapsed_ms = started.elapsed().as_millis() as i64;
    match res {
        Ok(Ok(_)) => (Status::Healthy, Some(elapsed_ms), None),
        Ok(Err(e)) => classify_llm_error(&e, elapsed_ms),
        Err(_) => (Status::Timeout, Some(elapsed_ms), Some("5s timeout".into())),
    }
}

/// 探活 CLI video provider：仅检查 `which <binary>` 是否能定位到
///
/// CLI video provider 不走 HTTP，所以探活只看二进制是否存在。不发起实际 vendor 调用。
pub fn probe_cli_video(cfg: &Config, name: &str) -> (Status, Option<i64>, Option<String>) {
    let pc = match cfg.provider.video.get(name) {
        Some(p) => p,
        None => {
            return (
                Status::Unconfigured,
                None,
                Some(format!("video.{} not in config", name)),
            )
        }
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
        (
            Status::UpstreamError,
            Some(ms),
            Some(format!("binary not found: {}", bin)),
        )
    }
}

/// 探活 Voice provider：用 stub base Voice 发最小 synth("ping") 请求
pub async fn probe_voice(cfg: &Config, name: &str) -> (Status, Option<i64>, Option<String>) {
    use crate::provider::real::OpenAiCompatVoiceProvider;
    use crate::provider::VoiceProvider;
    let pc = match cfg.provider.voice.get(name) {
        Some(p) => p,
        None => {
            return (
                Status::Unconfigured,
                None,
                Some(format!("voice.{} not in config", name)),
            )
        }
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
    let elapsed_ms = started.elapsed().as_millis() as i64;
    match res {
        Ok(Ok(_)) => (Status::Healthy, Some(elapsed_ms), None),
        Ok(Err(e)) => classify_llm_error(&e, elapsed_ms),
        Err(_) => (Status::Timeout, Some(elapsed_ms), Some("5s timeout".into())),
    }
}

/// 遍历 config 中所有 provider，调用对应的 probe，并把结果写入 provider_health
///
/// 4 个 HTTP probe 并发跑（tokio::join!）；video probe 是 sync，串行跑（一次只跑一个，不阻塞）。
/// 写库失败不会回滚已写入的记录（best-effort）。
pub async fn probe_all(cfg: &Config, conn: &rusqlite::Connection) -> AvcResult<()> {
    use crate::svc::health::{provider_key, record};

    // 4 个 HTTP probe 并发
    let llm_fut = async {
        let names = collect_names(&cfg.provider.llm);
        for name in names {
            let key = provider_key("llm", &name);
            let (status, ms, err) = probe_llm(cfg, &name).await;
            record(conn, &key, status, ms, err.as_deref(), "probe").ok();
        }
    };
    let embed_fut = async {
        let names = collect_names(&cfg.provider.embed);
        for name in names {
            let key = provider_key("embed", &name);
            let (status, ms, err) = probe_embed(cfg, &name).await;
            record(conn, &key, status, ms, err.as_deref(), "probe").ok();
        }
    };
    let avatar_fut = async {
        let names = collect_names(&cfg.provider.avatar);
        for name in names {
            let key = provider_key("avatar", &name);
            let (status, ms, err) = probe_avatar(cfg, &name).await;
            record(conn, &key, status, ms, err.as_deref(), "probe").ok();
        }
    };
    let voice_fut = async {
        let names = collect_names(&cfg.provider.voice);
        for name in names {
            let key = provider_key("voice", &name);
            let (status, ms, err) = probe_voice(cfg, &name).await;
            record(conn, &key, status, ms, err.as_deref(), "probe").ok();
        }
    };
    tokio::join!(llm_fut, embed_fut, avatar_fut, voice_fut);

    // CLI video: 串行（一次只跑一个，避免多 which 竞争）
    let video_names = collect_names(&cfg.provider.video);
    for name in video_names {
        let key = provider_key("video", &name);
        let (status, ms, err) = probe_cli_video(cfg, &name);
        record(conn, &key, status, ms, err.as_deref(), "probe").ok();
    }
    Ok(())
}

fn collect_names<V>(map: &std::collections::HashMap<String, V>) -> Vec<String> {
    let mut names: Vec<String> = map.keys().cloned().collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderCfg;
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

    /// 启一个 mock HTTP 服务，handler 由调用方提供
    async fn spawn_mock(handler: impl Fn(String) -> String + Send + Sync + 'static) -> SocketAddr {
        use std::sync::Arc;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handler = Arc::new(handler);
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let handler = handler.clone();
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
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"pong"}}]}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let addr = spawn_mock(move |_req| resp.clone()).await;
        let cfg = cfg_with_llm("openai", &format!("http://{}", addr));
        let (status, ms, _) = probe_llm(&cfg, "openai").await;
        assert_eq!(status, Status::Healthy);
        assert!(ms.unwrap() >= 0);
    }

    #[tokio::test]
    async fn probe_llm_401_records_auth() {
        let addr =
            spawn_mock(|_| "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_string())
                .await;
        let cfg = cfg_with_llm("openai", &format!("http://{}", addr));
        let (status, _, _) = probe_llm(&cfg, "openai").await;
        assert_eq!(status, Status::Auth);
    }

    #[tokio::test]
    async fn probe_llm_429_records_rate_limited() {
        let addr = spawn_mock(|_| {
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n".to_string()
        })
        .await;
        let cfg = cfg_with_llm("openai", &format!("http://{}", addr));
        let (status, _, _) = probe_llm(&cfg, "openai").await;
        assert_eq!(status, Status::RateLimited);
    }

    #[tokio::test]
    async fn probe_unconfigured_api_key_skipped() {
        let mut cfg = Config::default();
        cfg.provider.llm.insert(
            "openai".into(),
            ProviderCfg {
                api_key: None,
                ..Default::default()
            },
        );
        let (status, _, _) = probe_llm(&cfg, "openai").await;
        assert_eq!(status, Status::Unconfigured);
    }

    #[tokio::test]
    async fn probe_embed_429_records_rate_limited() {
        let addr = spawn_mock(|_| {
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n".to_string()
        })
        .await;
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
        let addr =
            spawn_mock(|_| "HTTP/1.1 500 Internal\r\nContent-Length: 0\r\n\r\n".to_string()).await;
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
        // Mock hangs > 5s so probe's PROBE_TIMEOUT fires first.
        let addr = spawn_mock(|_| {
            std::thread::sleep(std::time::Duration::from_secs(8));
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_string()
        })
        .await;
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

    #[test]
    fn probe_cli_video_missing_binary_skipped() {
        let mut cfg = Config::default();
        cfg.provider.video.insert(
            "kling".into(),
            ProviderCfg {
                binary: None,
                ..Default::default()
            },
        );
        let (status, _, _) = probe_cli_video(&cfg, "kling");
        assert_eq!(status, Status::Unconfigured);
    }

    #[test]
    fn probe_cli_video_known_binary_healthy() {
        let mut cfg = Config::default();
        cfg.provider.video.insert(
            "sh".into(),
            ProviderCfg {
                binary: Some("/bin/sh".into()),
                ..Default::default()
            },
        );
        let (status, _, _) = probe_cli_video(&cfg, "sh");
        assert_eq!(status, Status::Healthy);
    }

    #[test]
    fn probe_all_empty_config_writes_nothing() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let sqls = [
            include_str!("../../migrations/0001_init.sql"),
            include_str!("../../migrations/0002_drift_dimensions.sql"),
            include_str!("../../migrations/0003_provider_health.sql"),
        ];
        for s in sqls {
            conn.execute_batch(s).unwrap();
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            probe_all(&Config::default(), &conn).await.unwrap();
        });
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM provider_health", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
