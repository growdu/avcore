//! Provider 探活函数
//!
//! 每个 probe 返回 (Status, latency_ms, err_msg)，由 caller 决定写库策略。
//! 详见 docs/superpowers/specs/2026-08-03-provider-daemon-design.md §4.1

use std::time::{Duration, Instant};

use crate::config::{Config, ProviderCfg};
use crate::error::AvcError;
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
