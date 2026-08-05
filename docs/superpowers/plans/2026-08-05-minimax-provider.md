# MiniMax Provider 适配 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 AVCore 加 3 个 MiniMax 专有 API provider（avatar / voice / video），让用户用 MiniMax token 端到端出片。

**Architecture:** 新建 `src/provider/minimax.rs`（避免 `real.rs` 突破 70 KB）。3 个 struct 各自实现现有 `AvatarProvider` / `VoiceProvider` / `VideoProvider` trait。`src/provider/mod.rs` 在 `create_*` 工厂中加 `_minimax` 后缀路由：现有 `[provider.<dim>.<n>]` 走 OpenAI，新独立段 `[provider.<dim>_minimax.<n>]` 走 MiniMax。

**Tech Stack:** 现有 `reqwest = 0.12` (rustls-tls) + `serde` + `tokio` + `tracing`。`hex` 解码 MiniMax 音频字段。新增 1 个文件 + 改 4 个文件。

---

## 文件结构

| 文件 | 状态 | 用途 |
|---|---|---|
| `src/provider/minimax.rs` | **新建** | 3 个 provider 实现 + 公共 helper（auth_header / handle_response / decode_hex_audio / wait_video_done）~600 行 |
| `src/provider/mod.rs` | 改 | 加 `pub mod minimax;` + `create_avatar` / `create_voice` / `create_video` 加 `_minimax` 路由分支 ~30 行 |
| `tests/integration.rs` | 改 | 7 个 mock 单元测试 + 2 个工厂路由测试 + 3 个 `#[ignore]` 真实 API 集成测试 ~300 行 |
| `docs/cli.md` | 改 | 加 `[provider.<dim>_minimax.<n>]` 配置段说明 ~30 行 |
| `CHANGELOG.md` | 改 | `[Unreleased]` 加 MiniMax 适配条目 |

**总改动**：~1000 行（含测试和 doc）。

---

## 任务清单（3 commit，按 M2/M5/M6 顺序）

---

### Task 1: 公共 helpers + MiniMaxCompatAvatarProvider

**Files:**
- Create: `src/provider/minimax.rs`
- Modify: `src/provider/mod.rs`（加 `pub mod minimax;` + 路由）
- Test: `tests/integration.rs`

- [ ] **Step 1: 写公共 helper 的失败测试**

`tests/integration.rs` 末尾加：

```rust
#[test]
fn minimax_auth_header_includes_bearer() {
    use avc::provider::minimax::auth_header;
    let h = auth_header("sk-cp-abc");
    assert_eq!(h.get("Authorization").unwrap(), "Bearer sk-cp-abc");
}

#[test]
fn minimax_decode_hex_audio_succeeds() {
    use avc::provider::minimax::decode_hex_audio;
    // "494433" 是 "ID3" 头
    let bytes = decode_hex_audio("494433").unwrap();
    assert_eq!(bytes, b"ID3");
}

#[test]
fn minimax_decode_hex_audio_rejects_invalid() {
    use avc::provider::minimax::decode_hex_audio;
    assert!(decode_hex_audio("zz").is_err());
}
```

- [ ] **Step 2: 跑测试，验失败**

```bash
cd /home/ubuntu/avcore && rtk proxy cargo test --test integration minimax_auth
```

Expected: FAIL（`unresolved import avc::provider::minimax`）。

- [ ] **Step 3: 写公共 helpers + MiniMaxCompatAvatarProvider skeleton**

`src/provider/minimax.rs` 全文件（一次性写完 ~250 行）：

```rust
//! MiniMax 专有 API 适配：avatar / voice / video
//!
//! 详见 docs/superpowers/specs/2026-08-04-minimax-provider-design.md

use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::config::ProviderCfg;
use crate::error::{AvcError, AvcResult};
use crate::provider::{
    Avatar, AvatarProvider, AvatarSpec, Audio, Sample, TrainCfg, Video, VideoProvider, Voice,
    VoiceProvider,
};

const DEFAULT_BASE_URL: &str = "https://api.minimaxi.com";

pub fn auth_header(api_key: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&format!("Bearer {}", api_key)) {
        h.insert(AUTHORIZATION, v);
    }
    h
}

pub fn decode_hex_audio(hex: &str) -> AvcResult<Vec<u8>> {
    hex::decode(hex).map_err(|e| AvcError::Internal(format!("hex decode: {}", e)))
}

pub async fn handle_response<T: DeserializeOwned>(resp: reqwest::Response) -> AvcResult<T> {
    let status = resp.status();
    if status == 401 {
        return Err(AvcError::TokenAuth(format!("minimax 401: {}", status)));
    }
    if status == 429 {
        return Err(AvcError::RateLimited(format!("minimax 429: {}", status)));
    }
    if !status.is_success() {
        return Err(AvcError::ProviderUpstream(format!("minimax HTTP {}", status)));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| AvcError::Internal(e.to_string()))?;
    let base_resp = body.get("base_resp");
    if let Some(b) = base_resp {
        let code = b.get("status_code").and_then(|v| v.as_i64()).unwrap_or(0);
        if code != 0 {
            let msg = b.get("status_msg").and_then(|v| v.as_str()).unwrap_or("").to_string();
            return if code == 2013 {
                Err(AvcError::Arg(format!("minimax invalid params: {}", msg)))
            } else {
                Err(AvcError::ProviderUpstream(format!("minimax code {}: {}", code, msg)))
            };
        }
    }
    serde_json::from_value(body).map_err(|e| AvcError::Internal(format!("decode: {}", e)))
}

// ── Avatar ──────────────────────────────────────────

pub struct MiniMaxCompatAvatarProvider {
    pub name: String,
    pub cfg: ProviderCfg,
    client: Client,
    base_url: String,
}

impl MiniMaxCompatAvatarProvider {
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        let headers = auth_header(cfg.api_key.as_deref().unwrap_or(""));
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AvcError::Internal(format!("reqwest: {}", e)))?;
        let base_url = cfg.base_url.clone().unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Ok(Self { name, cfg, client, base_url })
    }
}

impl AvatarProvider for MiniMaxCompatAvatarProvider {
    fn name(&self) -> &str { &self.name }
    async fn create(&self, spec: &AvatarSpec) -> AvcResult<Avatar> {
        let body = serde_json::json!({
            "model": self.cfg.model.as_deref().unwrap_or("image-01"),
            "prompt": spec.prompt,
            "n": 1,
            "aspect_ratio": "1:1",
            "response_format": "url",
            "prompt_enhancer": true,
        });
        let url = format!("{}/v1/image_generation", self.base_url);
        let resp = self.client.post(&url).json(&body).send().await
            .map_err(|e| AvcError::ProviderUpstream(e.to_string()))?;
        #[derive(Deserialize)]
        struct ImageResp {
            data: ImageData,
            #[allow(dead_code)] base_resp: serde_json::Value,
        }
        #[derive(Deserialize)]
        struct ImageData { image_urls: Vec<String> }
        let parsed: ImageResp = handle_response(resp).await?;
        let image_url = parsed.data.image_urls.into_iter().next()
            .ok_or_else(|| AvcError::ProviderUpstream("no image_urls in response".into()))?;
        let bytes = self.client.get(&image_url).send().await
            .map_err(|e| AvcError::ProviderUpstream(e.to_string()))?
            .bytes().await
            .map_err(|e| AvcError::Internal(e.to_string()))?;
        Ok(Avatar {
            provider: self.name.clone(),
            provider_version: "v1".into(),
            model_id: self.cfg.model.clone(),
            primary_png_b64: base64::engine::general_purpose::STANDARD.encode(&bytes),
            views_zip_b64: None,
            face_id: None,
        })
    }
    async fn finetune(&self, _base: &Avatar, _samples: &[Sample], _cfg: &TrainCfg) -> AvcResult<Avatar> {
        Err(AvcError::Internal("minimax avatar finetune not implemented; use vendor CLI".into()))
    }
}
```

- [ ] **Step 4: 跑测试，验公共 helper 通过**

```bash
cd /home/ubuntu/avcore && rtk proxy cargo test --test integration minimax
```

Expected: 3 个 helper 测试 PASS。

- [ ] **Step 5: 写 MiniMaxCompatAvatarProvider::create 失败测试**

`tests/integration.rs` 加：

```rust
#[tokio::test]
async fn minimax_avatar_create_succeeds_with_mock_server() {
    use avc::provider::minimax::MiniMaxCompatAvatarProvider;
    use avc::provider::AvatarProvider;
    use avc::provider::AvatarSpec;
    use avc::config::ProviderCfg;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use std::sync::Arc;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                if req.contains("image_generation") {
                    let body = r#"{"data":{"image_urls":["http://127.0.0.1:1/x.png"]},"base_resp":{"status_code":0}}"#;
                    let r = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
                    let _ = sock.write_all(r.as_bytes()).await;
                } else {
                    let img = b"\x89PNG\r\n\x1a\nfake_png_bytes";
                    let r = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n", img.len());
                    let _ = sock.write_all(r.as_bytes()).await;
                    let _ = sock.write_all(img).await;
                }
            });
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut cfg = ProviderCfg::default();
    cfg.api_key = Some("sk-cp-test".into());
    cfg.model = Some("image-01".into());
    cfg.base_url = Some(format!("http://{}", addr));
    let p = MiniMaxCompatAvatarProvider::new("test_minimax".into(), cfg).unwrap();
    let avatar = p.create(&AvatarSpec {
        prompt: "a red apple".into(),
        style: None,
        ref_image_paths: vec![],
    }).await.unwrap();
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(&avatar.primary_png_b64).unwrap();
    assert!(bytes.starts_with(b"\x89PNG"));
}
```

- [ ] **Step 6: 跑测试，验失败**

```bash
cd /home/ubuntu/avcore && rtk proxy cargo test --test integration minimax_avatar_create_succeeds
```

Expected: FAIL（`unresolved import avc::provider::minimax::MiniMaxCompatAvatarProvider`，或之前的 helper 引入的 `unresolved import`）。**说明**：这一 step 在 src/provider/mod.rs 加 `pub mod minimax;` 之前必然失败——所以顺序应是：先做 Step 7 加 mod 声明，再做 Step 8 跑测试。

- [ ] **Step 7: 在 `src/provider/mod.rs` 加 minimax 模块声明 + 工厂路由**

改 `src/provider/mod.rs`：

```rust
// 现有：
pub mod mock;
pub mod probe;
pub mod real;

// 加：
pub mod minimax;
```

在 `create_avatar`（和 `create_voice` / `create_video`）工厂函数最前面加 `_minimax` 后缀路由。示例（avatar）：

```rust
pub async fn create_avatar(name: &str, pc: &ProviderCfg) -> AvcResult<Arc<dyn AvatarProvider>> {
    if name.ends_with("_minimax") {
        return Ok(Arc::new(minimax::MiniMaxCompatAvatarProvider::new(
            name.to_string(),
            pc.clone(),
        )?));
    }
    // 现有 OpenAI 路径
    let _cfg = find_provider_cfg(...);
    Ok(Arc::new(real::OpenAiCompatAvatarProvider::new(name.to_string(), pc.clone())?))
}
```

具体改 3 个 factory 函数。每个先 `_minimax` 检查，否则走现有路径。**保留现有代码**——`if-else` 第一个分支走 MiniMax，`else` 走 OpenAI。

- [ ] **Step 8: 跑 avatar 测试，验通过**

```bash
cd /home/ubuntu/avcore && rtk proxy cargo test --test integration minimax_avatar
```

Expected: PASS。

- [ ] **Step 9: 写并跑工厂路由测试**

`tests/integration.rs` 加：

```rust
#[tokio::test]
async fn minimax_factory_routes_avatar_by_name_suffix() {
    use avc::config::ProviderCfg;
    let mut pc = ProviderCfg::default();
    pc.api_key = Some("sk-cp-test".into());
    pc.model = Some("image-01".into());
    pc.base_url = Some("http://127.0.0.1:1".into());
    // 后缀 _minimax → MiniMax provider
    let r = avc::provider::create_avatar("yu_minimax", &pc).await;
    assert!(r.is_ok(), "_minimax suffix should route to MiniMax provider");
}
```

注：实际 `create_avatar` 是 async 还是 sync 看 mod.rs 当前签名（`pub fn` 还是 `pub async fn`）。读 mod.rs 后调整测试。

```bash
cd /home/ubuntu/avcore && rtk proxy cargo test --test integration minimax_factory
```

Expected: PASS。

- [ ] **Step 10: cargo fmt + clippy + test**

```bash
cd /home/ubuntu/avcore
rtk proxy cargo fmt --all
rtk proxy cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
rtk proxy cargo test --all-targets 2>&1 | tail -5
```

Expected: fmt 干净，clippy 0 errors，185+ tests pass。

- [ ] **Step 11: 提交**

```bash
cd /home/ubuntu/avcore
git add src/provider/minimax.rs src/provider/mod.rs tests/integration.rs
git commit -m "feat(provider): MiniMax avatar provider + factory routing"
```

---

### Task 2: MiniMaxCompatVoiceProvider

**Files:**
- Modify: `src/provider/minimax.rs`（追加 struct + impl + 2 测试）
- Modify: `src/provider/mod.rs`（factory voice 路由）
- Test: `tests/integration.rs`

- [ ] **Step 1: 写 voice 测试**

```rust
#[tokio::test]
async fn minimax_voice_synth_decodes_hex_mp3() {
    use avc::provider::minimax::MiniMaxCompatVoiceProvider;
    use avc::provider::VoiceProvider;
    use avc::config::ProviderCfg;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use base64::Engine;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let resp = r#"{"data":{"audio":"494433"},"base_resp":{"status_code":0}}"#;
                let r = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", resp.len(), resp);
                let _ = sock.write_all(r.as_bytes()).await;
                let _ = n;
            });
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut cfg = ProviderCfg::default();
    cfg.api_key = Some("sk-cp-test".into());
    cfg.model = Some("speech-01-turbo".into());
    cfg.base_url = Some(format!("http://{}", addr));
    let p = MiniMaxCompatVoiceProvider::new("test_minimax_voice".into(), cfg).unwrap();
    let voice = avc::provider::Voice {
        provider: "test".into(),
        provider_version: "v1".into(),
        voice_id_remote: Some("male-qn-qingse".into()),
        sample_wav_b64: String::new(),
        transcript: None,
        embed_b64: None,
        embed_dim: None,
    };
    let audio = p.synth(&voice, "hi").await.unwrap();
    // "494433" → "ID3"
    let bytes = base64::engine::general_purpose::STANDARD.decode(&audio.wav_b64).unwrap();
    assert_eq!(bytes, b"ID3");
}
```

- [ ] **Step 2: 跑测试，验失败**

```bash
cd /home/ubuntu/avcore && rtk proxy cargo test --test integration minimax_voice_synth
```

Expected: FAIL（unresolved import / function not found）。

- [ ] **Step 3: 写 MiniMaxCompatVoiceProvider 实现**

在 `src/provider/minimax.rs` 文件末尾加（avatar 之后）：

```rust
// ── Voice ──────────────────────────────────────────

pub struct MiniMaxCompatVoiceProvider {
    pub name: String,
    pub cfg: ProviderCfg,
    client: Client,
    base_url: String,
}

impl MiniMaxCompatVoiceProvider {
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        let headers = auth_header(cfg.api_key.as_deref().unwrap_or(""));
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AvcError::Internal(format!("reqwest: {}", e)))?;
        let base_url = cfg.base_url.clone().unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Ok(Self { name, cfg, client, base_url })
    }
}

impl VoiceProvider for MiniMaxCompatVoiceProvider {
    fn name(&self) -> &str { &self.name }
    async fn synth(&self, _voice: &Voice, text: &str) -> AvcResult<Audio> {
        let body = serde_json::json!({
            "model": self.cfg.model.as_deref().unwrap_or("speech-01-turbo"),
            "text": text,
            "voice_setting": { "voice_id": "male-qn-qingse" },
            "audio_setting": { "format": "mp3" },
        });
        let url = format!("{}/v1/t2a_v2", self.base_url);
        let resp = self.client.post(&url).json(&body).send().await
            .map_err(|e| AvcError::ProviderUpstream(e.to_string()))?;
        #[derive(Deserialize)]
        struct TtsResp {
            data: TtsData,
            #[allow(dead_code)] base_resp: serde_json::Value,
        }
        #[derive(Deserialize)]
        struct TtsData { audio: String }
        let parsed: TtsResp = handle_response(resp).await?;
        let mp3_bytes = decode_hex_audio(&parsed.data.audio)?;
        Ok(Audio {
            provider: self.name.clone(),
            provider_version: "v1".into(),
            voice_id_remote: None,
            sample_wav_b64: base64::engine::general_purpose::STANDARD.encode(&mp3_bytes),
            transcript: None,
        })
    }
    async fn clone(&self, _base: &Voice, _samples: &[Sample], _cfg: &TrainCfg) -> AvcResult<Voice> {
        // placeholder：v1 不实现 MiniMax voice clone（需 file_id upload 复杂 schema）
        Err(AvcError::Internal("minimax voice clone not implemented; use vendor CLI".into()))
    }
}
```

- [ ] **Step 4: 跑测试，验通过**

```bash
cd /home/ubuntu/avcore && rtk proxy cargo test --test integration minimax_voice
```

Expected: PASS。

- [ ] **Step 5: 加 voice factory 路由 + 测试**

`src/provider/mod.rs` 在 `create_voice` 函数最前面加：

```rust
if name.ends_with("_minimax") {
    return Ok(Arc::new(minimax::MiniMaxCompatVoiceProvider::new(
        name.to_string(),
        pc.clone(),
    )?));
}
```

测试（`tests/integration.rs`）：

```rust
#[tokio::test]
async fn minimax_factory_routes_voice_by_name_suffix() {
    use avc::config::ProviderCfg;
    let mut pc = ProviderCfg::default();
    pc.api_key = Some("sk-cp-test".into());
    pc.model = Some("speech-01-turbo".into());
    let r = avc::provider::create_voice("yu_minimax", &pc).await;
    assert!(r.is_ok());
}
```

跑 `rtk proxy cargo test --test integration minimax_factory` 验通过。

- [ ] **Step 6: cargo fmt + clippy + test**

```bash
cd /home/ubuntu/avcore
rtk proxy cargo fmt --all
rtk proxy cargo clippy --all-targets -- -D warnings 2>&1 | tail -3
rtk proxy cargo test --all-targets 2>&1 | tail -3
```

- [ ] **Step 7: 提交（合并到 Task 1 的 commit，或新 commit）**

```bash
cd /home/ubuntu/avcore
git add src/provider/minimax.rs src/provider/mod.rs tests/integration.rs
git commit -m "feat(provider): MiniMax voice provider + factory routing"
```

---

### Task 3: MiniMaxCompatVideoProvider

**Files:**
- Modify: `src/provider/minimax.rs`（追加 struct + 公共 wait_video_done + impl + 3 测试）
- Modify: `src/provider/mod.rs`（factory video 路由）
- Test: `tests/integration.rs`

- [ ] **Step 1: 写 wait_video_done 测试**

```rust
#[tokio::test]
async fn minimax_wait_video_done_polls_until_success() {
    use avc::provider::minimax::wait_video_done;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let call_count = Arc::new(AtomicUsize::new(0));
    let cc = call_count.clone();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            let cc = cc.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let i = cc.fetch_add(1, Ordering::SeqCst);
                let resp = if i == 0 {
                    r#"{"status":"Processing","file_id":"","base_resp":{"status_code":0}}"#
                } else {
                    r#"{"status":"Success","file_id":"file_42","base_resp":{"status_code":0}}"#
                };
                let r = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", resp.len(), resp);
                let _ = sock.write_all(r.as_bytes()).await;
                let _ = req;
            });
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let file_id = wait_video_done(
        &reqwest::Client::new(),
        &format!("http://{}", addr),
        &reqwest::header::HeaderMap::new(),
        "task_42",
        std::time::Duration::from_millis(50),
        std::time::Duration::from_secs(5),
    ).await.unwrap();
    assert_eq!(file_id, "file_42");
    assert!(call_count.load(Ordering::SeqCst) >= 2);
}
```

- [ ] **Step 2: 跑测试，验失败**

```bash
cd /home/ubuntu/avcore && rtk proxy cargo test --test integration minimax_wait_video
```

Expected: FAIL（`unresolved import avc::provider::minimax::wait_video_done`）。

- [ ] **Step 3: 写 wait_video_done + MiniMaxCompatVideoProvider 实现**

`src/provider/minimax.rs` 末尾追加：

```rust
// ── Video ──────────────────────────────────────────

pub async fn wait_video_done(
    client: &Client,
    base_url: &str,
    auth: &HeaderMap,
    task_id: &str,
    poll_interval: Duration,
    timeout: Duration,
) -> AvcResult<String> {
    let url = format!("{}/v1/query/video_generation?task_id={}", base_url, task_id);
    let started = std::time::Instant::now();
    loop {
        let resp = client.get(&url).headers(auth.clone()).send().await
            .map_err(|e| AvcError::ProviderUpstream(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(AvcError::ProviderUpstream(format!("minimax poll HTTP {}", resp.status())));
        }
        let body: serde_json::Value = resp.json().await
            .map_err(|e| AvcError::Internal(e.to_string()))?;
        let status = body.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let file_id = body.get("file_id").and_then(|v| v.as_str()).unwrap_or("");
        match status {
            "Success" if !file_id.is_empty() => return Ok(file_id.to_string()),
            "Success" => return Err(AvcError::ProviderUpstream("success but no file_id".into())),
            "Fail" => return Err(AvcError::ProviderUpstream("video task failed".into())),
            _ => {}
        }
        if started.elapsed() > timeout {
            return Err(AvcError::ProviderUpstream(format!(
                "video task {} did not complete within {:?}", task_id, timeout
            )));
        }
        tokio::time::sleep(poll_interval).await;
    }
}

pub struct MiniMaxCompatVideoProvider {
    pub name: String,
    pub cfg: ProviderCfg,
    client: Client,
    base_url: String,
    poll_interval: Duration,
    timeout: Duration,
}

impl MiniMaxCompatVideoProvider {
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        let headers = auth_header(cfg.api_key.as_deref().unwrap_or(""));
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AvcError::Internal(format!("reqwest: {}", e)))?;
        let base_url = cfg.base_url.clone().unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Ok(Self {
            name, cfg, client, base_url,
            poll_interval: Duration::from_secs(5),
            timeout: Duration::from_secs(300),
        })
    }
}

impl VideoProvider for MiniMaxCompatVideoProvider {
    fn name(&self) -> &str { &self.name }
    async fn submit(&self, prompt: &str, _avatar: &[u8], _voice: &[u8]) -> AvcResult<String> {
        let body = serde_json::json!({
            "model": self.cfg.model.as_deref().unwrap_or("video-01"),
            "prompt": prompt,
        });
        let url = format!("{}/v1/video_generation", self.base_url);
        let resp = self.client.post(&url).json(&body).send().await
            .map_err(|e| AvcError::ProviderUpstream(e.to_string()))?;
        #[derive(Deserialize)]
        struct SubmitResp { task_id: String }
        let parsed: SubmitResp = handle_response(resp).await?;
        Ok(parsed.task_id)
    }
    async fn fetch(&self, task_id: &str, out: &std::path::Path) -> AvcResult<()> {
        // 1. 轮询拿 file_id
        let auth = auth_header(self.cfg.api_key.as_deref().unwrap_or(""));
        let file_id = wait_video_done(
            &self.client, &self.base_url, &auth,
            task_id, self.poll_interval, self.timeout,
        ).await?;
        // 2. retrieve 拿 download_url
        let retrieve_url = format!("{}/v1/files/retrieve?file_id={}", self.base_url, file_id);
        let resp = self.client.get(&retrieve_url).headers(auth).send().await
            .map_err(|e| AvcError::ProviderUpstream(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(AvcError::ProviderUpstream(format!("minimax retrieve HTTP {}", resp.status())));
        }
        let body: serde_json::Value = resp.json().await
            .map_err(|e| AvcError::Internal(e.to_string()))?;
        let download_url = body.get("file").and_then(|f| f.get("download_url"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| AvcError::ProviderUpstream("no download_url".into()))?
            .to_string();
        // 3. 下载 mp4（⚠️ 无 auth header）
        let bytes = self.client.get(&download_url).send().await
            .map_err(|e| AvcError::ProviderUpstream(e.to_string()))?
            .bytes().await
            .map_err(|e| AvcError::Internal(e.to_string()))?;
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(out, &bytes).map_err(|e| AvcError::Internal(format!("write: {}", e)))?;
        Ok(())
    }
}
```

注：`fetch` 集成了 `submit → poll → download` 三步（`submit` 由调用方先调一次，`fetch` 拿 `task_id`）。**没有单独 `status()` 方法**——直接 `fetch` 时阻塞 poll 完成。这简化了 VideoProvider 的 trait 兼容。

- [ ] **Step 4: 跑测试，验通过**

```bash
cd /home/ubuntu/avcore && rtk proxy cargo test --test integration minimax_wait_video
```

Expected: PASS。

- [ ] **Step 5: 加 video factory 路由 + 测试**

`src/provider/mod.rs` 在 `create_video` 函数最前面加：

```rust
if name.ends_with("_minimax") {
    return Ok(Arc::new(minimax::MiniMaxCompatVideoProvider::new(
        name.to_string(),
        pc.clone(),
    )?));
}
```

测试（`tests/integration.rs`）：

```rust
#[test]
fn minimax_factory_routes_video_by_name_suffix() {
    use avc::config::ProviderCfg;
    let mut pc = ProviderCfg::default();
    pc.api_key = Some("sk-cp-test".into());
    pc.model = Some("video-01".into());
    let r = avc::provider::create_video("yu_minimax", &pc);
    assert!(r.is_ok());
}
```

跑 `rtk proxy cargo test --test integration minimax_factory` 验通过。

- [ ] **Step 6: cargo fmt + clippy + test**

```bash
cd /home/ubuntu/avcore
rtk proxy cargo fmt --all
rtk proxy cargo clippy --all-targets -- -D warnings 2>&1 | tail -3
rtk proxy cargo test --all-targets 2>&1 | tail -3
```

Expected: 0 errors，187+ tests pass（3 个新 video）。

- [ ] **Step 7: 提交**

```bash
cd /home/ubuntu/avcore
git add src/provider/minimax.rs src/provider/mod.rs tests/integration.rs
git commit -m "feat(provider): MiniMax video provider (3-step async)"
```

---

### Task 4: 真实 API 集成测试（`#[ignore]`）

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: 加 3 个 `#[ignore]` 集成测试**

```rust
#[tokio::test]
#[ignore = "requires MINIMAX_API_KEY env; uses 18B token quota"]
async fn minimax_real_api_avatar_creates_image() {
    use avc::provider::minimax::MiniMaxCompatAvatarProvider;
    use avc::provider::AvatarProvider;
    use avc::provider::AvatarSpec;
    use avc::config::ProviderCfg;

    let api_key = match std::env::var("MINIMAX_API_KEY") {
        Ok(k) => k,
        Err(_) => return,  // skip
    };
    let mut cfg = ProviderCfg::default();
    cfg.api_key = Some(api_key);
    cfg.model = Some("image-01".into());
    let p = MiniMaxCompatAvatarProvider::new("real_minimax".into(), cfg).unwrap();
    let avatar = p.create(&AvatarSpec {
        prompt: "a tiny red apple".into(),
        style: None,
        ref_image_paths: vec![],
    }).await.unwrap();
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(&avatar.primary_png_b64).unwrap();
    assert!(bytes.starts_with(b"\x89PNG"));
    assert!(bytes.len() > 100, "expected real PNG, got {} bytes", bytes.len());
}

#[tokio::test]
#[ignore = "requires MINIMAX_API_KEY env; uses 18B token quota"]
async fn minimax_real_api_voice_synth_decodes_mp3() {
    use avc::provider::minimax::MiniMaxCompatVoiceProvider;
    use avc::provider::VoiceProvider;
    use avc::provider::Voice;
    use avc::config::ProviderCfg;
    use base64::Engine;

    let api_key = match std::env::var("MINIMAX_API_KEY") {
        Ok(k) => k,
        Err(_) => return,
    };
    let mut cfg = ProviderCfg::default();
    cfg.api_key = Some(api_key);
    cfg.model = Some("speech-01-turbo".into());
    let p = MiniMaxCompatVoiceProvider::new("real_minimax_voice".into(), cfg).unwrap();
    let voice = Voice {
        provider: "test".into(),
        provider_version: "v1".into(),
        voice_id_remote: Some("male-qn-qingse".into()),
        sample_wav_b64: String::new(),
        transcript: None,
        embed_b64: None,
        embed_dim: None,
    };
    let audio = p.synth(&voice, "你好世界").await.unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(&audio.wav_b64).unwrap();
    assert!(bytes.len() > 1000, "expected real MP3, got {} bytes", bytes.len());
    // ID3 header at start (MP3)
    assert_eq!(&bytes[..3], b"ID3");
}

#[tokio::test]
#[ignore = "requires MINIMAX_API_KEY env; uses 18B token quota; takes ~80-110s"]
async fn minimax_real_api_video_generates_mp4() {
    use avc::provider::minimax::MiniMaxCompatVideoProvider;
    use avc::provider::VideoProvider;
    use avc::config::ProviderCfg;
    use std::time::Duration;

    let api_key = match std::env::var("MINIMAX_API_KEY") {
        Ok(k) => k,
        Err(_) => return,
    };
    let mut cfg = ProviderCfg::default();
    cfg.api_key = Some(api_key);
    cfg.model = Some("video-01".into());
    let mut p = MiniMaxCompatVideoProvider::new("real_minimax_video".into(), cfg).unwrap();
    // 测试模式：短超时
    p.poll_interval = Duration::from_secs(5);
    p.timeout = Duration::from_secs(180);
    let task_id = p.submit("a cat walking slowly", &[], &[]).await.unwrap();
    let tmp = std::env::temp_dir().join("minimax_real_test.mp4");
    p.fetch(&task_id, &tmp).await.unwrap();
    let bytes = std::fs::read(&tmp).unwrap();
    // MP4 magic: "ftyp" at byte 4
    assert_eq!(&bytes[4..8], b"ftyp");
    assert!(bytes.len() > 1000, "expected real MP4, got {} bytes", bytes.len());
    let _ = std::fs::remove_file(&tmp);
}
```

- [ ] **Step 2: 跑 `cargo build`（不跑 ignored 测试），确保编译过**

```bash
cd /home/ubuntu/avcore && rtk proxy cargo test --test integration --no-run
```

Expected: 编译成功。

- [ ] **Step 3: 提交**

```bash
cd /home/ubuntu/avcore
git add tests/integration.rs
git commit -m "test(provider): MiniMax real API integration tests (#[ignore])"
```

- [ ] **Step 4: 手动跑 ignored 测试（用户自行 verify，需 MINIMAX_API_KEY）**

```bash
MINIMAX_API_KEY=sk-cp-... rtk proxy cargo test --test integration minimax_real -- --ignored --nocapture
```

Expected：3 个都通过（avatar ~3s / voice ~5s / video ~80-110s）。**CI 不跑 ignored**。

---

### Task 5: 文档 + CHANGELOG

**Files:**
- Modify: `docs/cli.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: 改 `docs/cli.md`**

加一节说明 MiniMax 配置：

```markdown
## MiniMax Provider（`minimaxi.com`）

支持 MiniMax 文本/图像/语音/视频专有 API（不暴露 OpenAI 兼容端点）。配置段名后缀固定 `_minimax`：

```toml
[provider.avatar_minimax.yu]
api_key = "sk-cp-..."                  # MiniMax 控制台
model = "image-01"                    # 或 image-01-live
base_url = "https://api.minimaxi.com"

[provider.voice_minimax.yu]
api_key = "sk-cp-..."
model = "speech-01-turbo"             # 或 speech-01

[provider.video_minimax.yu]
api_key = "sk-cp-..."
model = "video-01"                    # 或 T2V-01；I2V-01 (需 first_frame_image，v1 暂不支持)
```

`avc render run --avatar-provider yu_minimax ...` 走 MiniMax 适配器。
- audio 字段是 HEX 编码（不是 base64）—— 适配器自动解码
- 视频异步：submit → poll（5s/次，5min 超时）→ 文件 retrieve
- 视频每日 3 条配额：撞到 429 报 `AvcError::RateLimited`（exit 10）
- voice_id 写死 `male-qn-qingse`（v1 不可选声线）
```

放在 `docs/cli.md` 现有 `provider` 相关节之后。

- [ ] **Step 2: 改 `CHANGELOG.md`**

`[Unreleased]` 段加：

```markdown
### Added
- MiniMax 多模态 Provider 适配（avatar / voice / video）
  - 端点：`api.minimaxi.com` 专有 API（非 OpenAI 兼容）
  - 配置：`[provider.<dim>_minimax.<n>]` 段（后缀固定 `_minimax` 走 MiniMax 适配器）
  - image-01 / speech-01-turbo / video-01 实测通过
  - 视频异步 3 段式（submit → poll → retrieve）
  - 真实 API 集成测试用 `#[ignore]` 标记（需 `MINIMAX_API_KEY` env）
- 3 个新单元测试 + 3 个 `#[ignore]` 集成测试
```

- [ ] **Step 3: cargo fmt + clippy + test 验证全过**

```bash
cd /home/ubuntu/avcore
rtk proxy cargo fmt --all
rtk proxy cargo clippy --all-targets -- -D warnings 2>&1 | tail -3
rtk proxy cargo test --all-targets 2>&1 | tail -5
```

Expected: 0 errors，190+ tests pass。

- [ ] **Step 4: 提交并 push**

```bash
cd /home/ubuntu/avcore
git add docs/cli.md CHANGELOG.md
git commit -m "docs: MiniMax provider config + CHANGELOG"
git push origin main
```

---

## Self-Review

**Spec 覆盖检查**：

| Spec 节 | 对应 task |
|---|---|
| §2 配置 schema | Task 5（docs/cli.md + avc.toml 例子）|
| §3 文件组织（minimax.rs 新建）| Task 1-3 |
| §4 公共 helper（auth_header / handle_response / decode_hex_audio / wait_video_done）| Task 1 (helper) + Task 3 (wait_video_done) |
| §5.1 MiniMaxCompatAvatarProvider | Task 1 |
| §5.2 MiniMaxCompatVoiceProvider | Task 2 |
| §5.3 MiniMaxCompatVideoProvider | Task 3 |
| §5.4 factory 路由 | Task 1-3 各自的 Step 5/7 |
| §6 错误翻译 | Task 1 helper `handle_response` |
| §7 测试策略 | Task 1-3（mock 单元）+ Task 4（`#[ignore]` 真实 API）|
| §9 实施 checklist | Task 1-5 |
| §10 用法示例 | Task 1 测试 + Task 5 doc |
| §12 out-of-scope | Task 5 标记 "voice_id 写死 / I2V 不支持" |

**Placeholder 扫描**：无 TBD / TODO / 后续再说（除了 §12 明确列的 out-of-scope）。
**类型一致性**：
- `MiniMaxCompatAvatarProvider` / `Voice` / `Video` 3 个 struct 都用 `(name: String, cfg: ProviderCfg)` 字段
- 都 impl 现有 trait：AvatarProvider / VoiceProvider / VideoProvider
- 公共 helper 4 个签名在 §4 定义，Task 1-3 全部用一致签名

**自评通过**。

---

## 实施顺序（3 commit + 1 doc commit）

```
1. feat(provider): MiniMax avatar provider + factory routing     (Task 1)
2. feat(provider): MiniMax voice provider + factory routing      (Task 2)
3. feat(provider): MiniMax video provider (3-step async)         (Task 3)
4. test(provider): MiniMax real API integration tests (#[ignore]) (Task 4)
5. docs: MiniMax provider config + CHANGELOG                     (Task 5)
```

每个 task 末跑 `cargo test --all-targets` 验证。每个 commit 末 `cargo fmt --all` + `cargo clippy -D warnings`。

预计总代码：~1000 行（含测试和 doc），1-2 小时。
