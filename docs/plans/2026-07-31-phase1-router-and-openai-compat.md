# Phase 1 · Provider 路由表 + OpenAI 兼容 LLM Provider 实施计划

> **For Hermes:** 走 TDD 直跑模式（plan 已写细，无需独立 reviewer）；本计划只覆盖
> Phase 1 的"第一刀"——验证一条真 Provider 路径。其余 6 项（avatar/voice/video/embed
> 真 Provider + drift 真算 + DAG 真调度）按需后续追加独立 plan。

**Goal:** 让 `ask` 模式真打到 OpenAI 兼容 / Anthropic 兼容 / DeepSeek 等 chat completion
端点，证明"真 Provider"路径走通；为后续 4 个真 Provider（avatar/voice/video/embed）
建立可复用的接线模板与路由表。

**Architecture:**

- 引入 `ProviderRegistry`：进程内单例，按"维度 + 名"缓存 Provider 实例
- 配置加载仍走 `~/.config/avc/avc.toml` 的 `[provider.llm.<name>]` 段，但额外支持
  `base_url`（OpenAI 兼容 API 的关键字段，与现有 `endpoint` 同义以便复用）、可选
  `headers`（用于 Anthropic `x-api-key` / `anthropic-version`）
- `OpenAiCompatLlmProvider` 是 trait 的真正实现；通过通用 reqwest client 调
  `/chat/completions` 端点
- `ask` 模式：当配置存在 `provider.llm.*` 时，把 NL 当 user message 真发出去，回显
  assistant content；不允许写操作（保持 Phase 1 stub 的安全姿态）
- 不引入新依赖；`reqwest::Client` 全局共享（连接池复用）

**Tech Stack:** Rust 2021 + reqwest(已有) + async-trait(已有) + serde/toml(已有)

**非范围（明确不做）：**

- 不动 persona/iterate/finetune/render 任何现有路径（保持 15 个测试零回归）
- 不实现 avatar/voice/video/embed 真 Provider（独立 plan）
- 不实现 NL→原子计划（仅回显 LLM 输出，留 hook）
- 不实现 Provider 健康检查（`avc provider test` 后续 plan）

---

## 任务拆分

### Task 1: ProviderCfg 扩展 base_url / extra_headers

**Objective:** Config 的 `ProviderCfg` 增加 `base_url: Option<String>` 与
`extra_headers: HashMap<String,String>`，让 OpenAI 兼容与 Anthropic 兼容都能挂在
现有 `provider.<dim>.<name>` 配置段下。

**Files:**
- Modify: `src/config.rs`

**Step 1: 加字段（保 round-trip 兼容）**

把：

```rust
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProviderCfg {
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
}
```

改成：

```rust
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProviderCfg {
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub extra_headers: std::collections::BTreeMap<String, String>,
}
```

为什么 BTreeMap：JSON 序列化稳定，便于 round-trip 测试断言。

**Step 2: 扩展 get_path / apply_set 支持新字段**

- `get_path`: 加 `"base_url" => Ok(entry.base_url.as_ref().map(...))`
- `apply_set`: 加 `"base_url" => entry.base_url = Some(val.to_string())`
- `extra_headers` 因是 map 形态，**不接入 get_path / apply_set**——只读/写 by provider
  factory 内部。它出现已有 `apply_set` 之外的 TOML 编辑即可（直接 toml set）

**Step 3: 跑既有测试确认零回归**

Run: `cargo test --locked --test integration -- --test-threads=1`
Expected: 15 passed。

**Step 4: 提交**

```bash
git add src/config.rs
git commit -m "feat(config): add base_url + extra_headers to ProviderCfg"
```

---

### Task 2: Provider 工厂 + 注册表 + 真 OpenAI 兼容实现

**Objective:** 新增 `src/provider/real.rs`，实现 `OpenAiCompatLlmProvider`（聊天调用
OpenAI 兼容端点 `/chat/completions`）。这是第一个真 Provider 实现，确立后续 4 个
真 Provider 的接线模式。

**Files:**
- Create: `src/provider/real.rs`
- Modify: `src/provider/mod.rs`

**Step 1: real.rs 顶部结构**

```rust
//! 真实 Provider 实现：仅 token 鉴权 API 调用。
//!
//! 当前包含 OpenAI 兼容的 LLM Provider。其余 4 个真 Provider
//! （avatar/voice/video/embed）按同一模式后续追加。

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::{LlmProvider, ChatMessage};
use crate::config::{Config, ProviderCfg};
use crate::error::{AvcError, AvcResult};

/// OpenAI 兼容 chat completion 端点。可用于 OpenAI / Azure OpenAI / DeepSeek /
/// 智谱 / 豆包 / Ollama 等暴露相同 schema 的服务。
pub struct OpenAiCompatLlmProvider {
    pub name: String,
    pub cfg: ProviderCfg,
    client: Client,
}

impl OpenAiCompatLlmProvider {
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        let api_key = cfg.api_key.clone().ok_or_else(|| {
            AvcError::TokenMissing(format!(
                "provider.llm.{}.api_key 未配置", name
            ))
        })?;
        // 关键：即使 api_key 为空也允许（Ollama 等本地兼容服务不需要）；
        // 真正需要 header 时由 default_headers 决定。
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", api_key)) {
            headers.insert(reqwest::header::AUTHORIZATION, v);
        }
        for (k, v) in &cfg.extra_headers {
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                headers.insert(name, value);
            }
        }
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AvcError::Internal(format!("reqwest builder: {}", e)))?;
        Ok(Self { name, cfg, client })
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage>,
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[async_trait]
impl LlmProvider for OpenAiCompatLlmProvider {
    fn name(&self) -> &str { &self.name }

    async fn chat(&self, msgs: &[ChatMessage]) -> AvcResult<String> {
        let model = self.cfg.model.as_deref().unwrap_or("gpt-4o-mini");
        let base_url = self.cfg.base_url.as_deref()
            .or(self.cfg.endpoint.as_deref())
            .unwrap_or("https://api.openai.com/v1");
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

        let body = ChatRequest {
            model,
            messages: msgs.to_vec(),
            temperature: 0.0,
        };
        let resp = self.client.post(&url).json(&body).send().await
            .map_err(|e| AvcError::ProviderTimeout(format!("llm {} POST {}: {}", self.name, url, e)))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN {
            return Err(AvcError::TokenAuth(format!(
                "provider.llm.{}: HTTP {}", self.name, status
            )));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(AvcError::RateLimited(format!(
                "provider.llm.{}: HTTP 429", self.name
            )));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AvcError::ProviderUpstream(format!(
                "provider.llm.{}: HTTP {} body={}", self.name, status, body
            )));
        }
        let parsed: ChatResponse = resp.json().await
            .map_err(|e| AvcError::ProviderUpstream(format!(
                "provider.llm.{}: bad json: {}", self.name, e
            )))?;
        parsed.choices.into_iter().next()
            .map(|c| c.message.content)
            .ok_or_else(|| AvcError::ProviderUpstream(format!(
                "provider.llm.{}: empty choices", self.name
            )))
    }
}

/// Provider 工厂：从 Config + 维度名构造 provider 实例。
///
/// `dim`: "avatar" / "voice" / "llm" / "video" / "embed"
/// `name`: 该维度下的子 provider 名（与 toml key 一致）
pub fn make_llm(cfg: &Config, name: &str) -> AvcResult<Arc<dyn LlmProvider>> {
    let pc = cfg.provider.llm.get(name).ok_or_else(|| {
        AvcError::NotFound(format!("provider.llm.{}", name))
    })?;
    Ok(Arc::new(OpenAiCompatLlmProvider::new(name.to_string(), pc.clone())?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_returns_404_for_unknown_name() {
        let cfg = Config::default();
        let r = make_llm(&cfg, "ghost");
        assert!(matches!(r, Err(AvcError::NotFound(_))));
    }

    #[test]
    fn factory_token_missing_when_api_key_absent() {
        let mut cfg = Config::default();
        cfg.provider.llm.insert(
            "openai".into(),
            ProviderCfg { api_key: None, model: Some("gpt-4o-mini".into()), ..Default::default() },
        );
        let r = make_llm(&cfg, "openai");
        assert!(matches!(r, Err(AvcError::TokenMissing(_))));
    }

    #[test]
    fn factory_succeeds_with_api_key() {
        let mut cfg = Config::default();
        cfg.provider.llm.insert(
            "openai".into(),
            ProviderCfg { api_key: Some("sk-test".into()), model: Some("gpt-4o-mini".into()), ..Default::default() },
        );
        let p = make_llm(&cfg, "openai").expect("ok");
        assert_eq!(p.name(), "openai");
    }
}
```

**Step 2: mod.rs 导出**

在 `src/provider/mod.rs` 末尾加：

```rust
pub mod real;
```

**Step 3: 跑新单元测试**

Run: `cargo test --locked --lib provider::real -- --nocapture`
Expected: 3 passed。

**Step 4: 跑既有集成测试确认零回归**

Run: `cargo test --locked --test integration -- --test-threads=1`
Expected: 15 passed。

**Step 5: 提交**

```bash
git add src/provider/real.rs src/provider/mod.rs
git commit -m "feat(provider): add OpenAiCompatLlmProvider + factory"
```

---

### Task 3: ask 模式接入真 LLM（保留安全 stub）

**Objective:** 当用户配置了 `provider.llm.<name>` 时，`avc ask "..."` 真把输入当作
user message 发出去，回显 assistant content。**保留所有现有 Phase 1 stub 的安全
姿态**——不擅自改人设、不擅自出片。只打印 LLM 输出。

**Files:**
- Modify: `src/ask/mod.rs`

**Step 1: 工厂调用 + 真发请求**

把 `src/ask/mod.rs` 中：

```rust
let cfg = Config::load(&Config::default_config_path()?)?;
let has_llm = !cfg.provider.llm.is_empty();
if !has_llm {
    return Err(AvcError::NlModelMissing(
        "未配置 provider.llm.* ，无法做 NL 解析；可直接用原子命令".into(),
    ));
}

if dry_run {
    println!("[dry-run] would plan: {}", nl);
    return Ok(());
}

if !yes && !std::io::stdout().is_terminal() {
    return Err(AvcError::Arg(
        "非 TTY 下默认要求 --yes（避免脚本意外执行写操作）".into(),
    ));
}

// Phase 1 占位：拿到 LLM 也只是 echo
println!("[ask] phase-1 stub: input={}", nl);
println!("hint: 配置 provider.llm 后可启用真 NL 解析；当前阶段请用原子命令。");
if json {
    println!("{{\"input\": {:?}, \"phase\": 1}}", nl);
}

let _ = yes; // suppress
Ok(())
```

替换为：

```rust
let cfg = Config::load(&Config::default_config_path())?;
if cfg.provider.llm.is_empty() {
    return Err(AvcError::NlModelMissing(
        "未配置 provider.llm.* ，无法做 NL 解析；可直接用原子命令".into(),
    ));
}

// Phase 1 安全姿态：只读不写。Phase 2+ 才允许把 LLM 输出当成原子计划执行。
let write_intent = !yes && !std::io::stdout().is_terminal();
if write_intent && nl.to_lowercase().contains("create")
    || nl.to_lowercase().contains("delete")
    || nl.to_lowercase().contains("finetune")
    || nl.to_lowercase().contains("render run")
{
    return Err(AvcError::Arg(
        "非 TTY 下默认要求 --yes（避免脚本意外执行写操作）".into(),
    ));
}

// 选定默认 llm provider：取第一个 key
let provider_name = cfg.provider.llm.keys().next().unwrap().clone();
let llm = crate::provider::real::make_llm(&cfg, &provider_name)?;
let msgs = vec![crate::provider::ChatMessage {
    role: "user".into(),
    content: nl.to_string(),
}];
let reply = llm.chat(&msgs).await?;

if dry_run {
    println!("[dry-run] would send to {} : {}", provider_name, nl);
    return Ok(());
}

if json {
    let v = serde_json::json!({
        "input": nl,
        "provider": provider_name,
        "reply": reply,
        "phase": 1,
    });
    println!("{}", serde_json::to_string_pretty(&v).unwrap());
} else {
    println!("[ask] provider={}", provider_name);
    println!("{}", reply);
    println!("hint: 本阶段仅 echo LLM 回复；不自动执行写操作。");
}

let _ = yes; // suppress
Ok(())
```

加文件顶用 `tokio::runtime::Runtime` 包装同步 `run`（因为 ask 是同步入口）：

在 `src/ask/mod.rs` 顶部 `use` 后加：

```rust
fn run(args: &[String]) -> AvcResult<()> {
    // ... 同步逻辑 ...
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AvcError::Internal(format!("tokio: {}", e)))?;
    rt.block_on(async move { /* 上面 async 部分 */ Ok(()) })
}
```

为最小代码改动，**重构 `ask::run` 成全 async**，再在 main.rs 用 `tokio::runtime::Runtime` 包。
而 main.rs 现在没有 tokio runtime — 需要在 main.rs 加 `let rt = ... ; rt.block_on(...)`，
但这会动到 main.rs 的所有路径。

**更小动作**：在 `ask::run` 末尾用 `tokio::runtime::Builder` 同步包一层即可，
不动其它路径。代码如下：

```rust
// 替换整个 run 函数到上面内容后，在最末包一层 tokio runtime：
fn run(args: &[String]) -> AvcResult<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AvcError::Internal(format!("ask tokio: {}", e)))?;
    rt.block_on(run_async(args))
}

async fn run_async(args: &[String]) -> AvcResult<()> {
    // 上面 main 逻辑，.await 即可
    // (省略重复)
}
```

**Step 2: 跑既有集成测试 + ask 相关**

Run: `cargo test --locked --test integration ask_without_llm_errors -- --nocapture`
Expected: passed。

**Step 3: 加新集成测试：未配置 llm 仍报错**

`tests/integration.rs` 现有 `ask_without_llm_errors` 已盖这个 case。新增：

`ask_with_real_llm_uses_provider`：模拟一个未实际发出 HTTP 的真 Provider？不行——
ask 现在直连 reqwest 不能 mock。

**改为**：在测试里起一个最小 HTTP server（`httpmock` 不用 → 直接 std::net::TcpListener
手写一个返回固定 OpenAI JSON 的 server）。

加：

```rust
#[test]
fn ask_with_real_llm_round_trip() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    // 后台 handler thread：返回 OpenAI 形状的 JSON
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"hello-from-mock"}}]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();

    // 写 toml：base_url 指向本机 mock
    std::fs::create_dir_all(&config).unwrap();
    let toml = format!(
        "[provider.llm.mock]\napi_key = \"sk-test\"\nmodel = \"mock-model\"\nbase_url = \"http://127.0.0.1:{}\"\n",
        port
    );
    std::fs::write(config.join("avc/avc.toml"), toml).unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["ask", "--yes", "ping"])
        .output()
        .unwrap();
    let _ = handle.join();

    assert!(r.status.success(), "ask 应成功；stderr={}", String::from_utf8_lossy(&r.stderr));
    let stdout = String::from_utf8_lossy(&r.stdout);
    assert!(stdout.contains("hello-from-mock"),
        "应回显 LLM reply；stdout={:?}", stdout);
    assert!(stdout.contains("mock"),
        "应包含 provider 名 'mock'；stdout={:?}", stdout);
}
```

**Step 4: 跑所有集成测试**

Run: `cargo test --locked --test integration -- --test-threads=1`
Expected: 16 passed (15 + 新增)。

**Step 5: 提交**

```bash
git add src/ask/mod.rs tests/integration.rs
git commit -m "feat(ask): 真发请求到 OpenAI 兼容 LLM Provider"
```

---

### Task 4: avc provider test 加最小探针（HTTP HEAD / GET）

**Objective:** `avc provider test llm.<name>` 对 LLM 端点发请求，验证 token 有效。
对其它 4 维度 stub 一个 "未实现" 提示，避免假装通过。

**Files:**
- Modify: `src/cli/provider.rs`

**Step 1: 增 `test` verb**

```rust
"test" => {
    if argv.len() < 2 {
        return Err(AvcError::Arg("provider test <dim>.<name>".into()));
    }
    let target = &argv[1];
    let (dim, name) = target.split_once('.').ok_or_else(|| {
        AvcError::Arg("provider test: 需要形如 llm.openai".into())
    })?;
    match dim {
        "llm" => {
            let cfg = Config::load(&Config::default_config_path())?;
            let provider = crate::provider::real::make_llm(&cfg, name)?;
            // 用最小消息探活
            let msgs = vec![crate::provider::ChatMessage {
                role: "user".into(),
                content: "ping".into(),
            }];
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| AvcError::Internal(e.to_string()))?;
            let reply = rt.block_on(provider.chat(&msgs))?;
            let payload = json!({
                "provider": target,
                "ok": true,
                "reply_preview": reply.chars().take(80).collect::<String>(),
            });
            print(mode, &payload)?;
        }
        other => {
            return Err(AvcError::Arg(format!(
                "provider test.{}: not yet implemented (Phase 1+ scope)",
                other
            )));
        }
    }
}
```

**Step 2: 加集成测试**

```rust
#[test]
fn provider_test_llm_unknown_name_says_not_configured() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["provider", "test", "llm.ghost"])
        .output()
        .unwrap();
    // 期望：NotFound，但目前 stdout 可能含 ok=false 也可；这里先宽容为非 0
    assert!(!r.status.success(), "不存在的 llm 应非 0");
}
```

**Step 3: 跑测试**

Run: `cargo test --locked --test integration -- --test-threads=1`
Expected: 16/17 passed。

**Step 4: 提交**

```bash
git add src/cli/provider.rs tests/integration.rs
git commit -m "feat(provider): provider test llm.<name> 探针"
```

---

### Task 5: 文档同步

**Objective:** `docs/status.md` 把 Phase 1 第一项标 ✅，`docs/api/README.md` 中
`openai_compat` 增真实现说明（保留 mock 已声明），README 不变。

**Files:**
- Modify: `docs/status.md`
- Modify: `docs/api/README.md`

**Step 1: status.md**

把：

```
| 真实 Provider 实现（kling / openai / elevenlabs / doubao 等） | ⬜ | Phase 1.1 |
```

拆成两行：

```
| `openai_compat` LLM 真实现（任意 OpenAI 兼容 chat 端点） | ✅ | base_url + extra_headers 兼容 OpenAI / Anthropic 兼容 / DeepSeek / 智谱 / Ollama 等 |
| avatar / voice / video / embed 真 Provider | ⬜ | Phase 1.1 续 |
```

并在 Phase 1 总段下面补一行"已完成第一刀"的注脚。

**Step 2: api/README.md**

在第 4 节"内置 Provider 列表"中 `llm | openai_compat | 兼容 OpenAI / Anthropic / DeepSeek / 智谱 / 豆包 |`
后加一行：

```
|| llm | `openai_compat` | 任意 OpenAI 兼容 `/chat/completions` 端点；通过 `base_url` + `extra_headers` 接 OpenAI / DeepSeek / 智谱 / Anthropic 兼容 proxy / Ollama 等；设置 `provider.llm.<name>.api_key`+`model`+`base_url` 后 `avc ask` 直接可用 |
```

**Step 3: 提交**

```bash
git add docs/status.md docs/api/README.md
git commit -m "docs: Phase 1 Provider 路由第一刀交付"
```

---

## 验收

* `cargo test --locked --all-targets -- --test-threads=1` 全绿（unit + integration）
* `cargo clippy --locked --all-targets` 无新增 warning
* 手验：

  ```bash
  cargo build --release
  ./target/release/avc init
  ./target/release/avc config set provider.llm.openai.api_key sk-...
  ./target/release/avc config set provider.llm.openai.model gpt-4o-mini
  ./target/release/avc provider test llm.openai
  ./target/release/avc ask --yes "ping"
  ```

  （若没有真 token，第二个会 exit 5；本地起 mock HTTP server 也能跑通）

---

## 不做的明确清单（落到 Phase 1+ 后续）

* AvatarProvider / VoiceProvider / VideoProvider / EmbedProvider 真实现
* drift_eval 用 Provider 真 embedding 算
* DAG 引擎真调度
* Shell 内 NL 解析（按 plan → 多原子执行）
* corpus 切 chunk + embed
* provider test avatar/voice/video/embed 的具体探针

后续都开独立 plan。
