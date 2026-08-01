# Phase 1 · Embed Provider 实施计划

> 继续 Phase 1 第一刀（同模板）。新增 `OpenAiCompatEmbedProvider` 真实现 + `provider test embed.<name>` 探针。
>
> `drift_eval` 真用 embed 算留待独立 plan（触动 `svc/finetune::publish` 现有手 mock 语义）。

**Goal:** 让 `provider test embed.<name>` 真打到 OpenAI 兼容 `/embeddings` 端点，证明
embed Provider 路径走通；为 corpus 切 chunk + drift 真算提供底层调用。

**Architecture:**

- 仿照 `OpenAiCompatLlmProvider` 的模式：`OpenAiCompatEmbedProvider`
  - reqwest POST `{base_url}/embeddings`
  - body: `{ "input": [...], "model": "..." }`
  - response: `{ "data": [{ "embedding": [...] }, ...] }`
- `factory::make_embed(&Config, name)` —— 与 `make_llm` 对称
- 同样支持 `base_url` + `extra_headers`（Anthropic 兼容 proxy 没 `/embeddings`，但 OpenAI 兼容就行）

**Tech Stack:** 同前一刀，零新依赖。

**非范围：**

- 不动 `svc/finetune::publish`（已有 `--passed/--failed` 手 mock 行为，文档明确说要 Phase 1+ 替换）
- 不实现 corpus 切 chunk（Phase 1.4 独立 plan）
- 不实现 DriftEvaluator（独立 plan）

---

## 任务拆分

### Task 1: OpenAiCompatEmbedProvider 实现

**Files:**
- Modify: `src/provider/real.rs`

**Step 1: 加 Provider struct + impl + factory + 单测**

在 real.rs 末尾（unit tests 后或前），加：

```rust
pub struct OpenAiCompatEmbedProvider {
    pub name: String,
    pub cfg: ProviderCfg,
    client: Client,
}

impl OpenAiCompatEmbedProvider {
    pub fn new(name: String, cfg: ProviderCfg) -> AvcResult<Self> {
        // 同 OpenAiCompatLlmProvider 构造逻辑
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(api_key) = cfg.api_key.as_ref() {
            if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", api_key))
            {
                headers.insert(reqwest::header::AUTHORIZATION, v);
            }
        }
        for (k, v) in &cfg.extra_headers {
            if let (Ok(hname), Ok(hval)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                headers.insert(hname, hval);
            }
        }
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| AvcError::Internal(format!("reqwest builder: {}", e)))?;
        Ok(Self { name, cfg, client })
    }

    fn base_url(&self) -> &str {
        self.cfg
            .base_url
            .as_deref()
            .or(self.cfg.endpoint.as_deref())
            .unwrap_or("https://api.openai.com/v1")
    }
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: Vec<&'a str>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedDatum>,
}

#[derive(Deserialize)]
struct EmbedDatum {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbedProvider for OpenAiCompatEmbedProvider {
    fn name(&self) -> &str { &self.name }

    async fn embed(&self, texts: &[&str]) -> AvcResult<Vec<Vec<f32>>> {
        let model = self.cfg.model.as_deref().unwrap_or("text-embedding-3-small");
        let url = format!("{}/embeddings", self.base_url().trim_end_matches('/'));
        let body = EmbedRequest { model, input: texts.to_vec() };
        let resp = self.client.post(&url).json(&body).send().await
            .map_err(|e| AvcError::ProviderTimeout(format!("embed {} POST {}: {}", self.name, url, e)))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN {
            return Err(AvcError::TokenAuth(format!("embed.{}: HTTP {}", self.name, status)));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(AvcError::RateLimited(format!("embed.{}: HTTP 429", self.name)));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AvcError::ProviderUpstream(format!(
                "embed.{}: HTTP {} body={}", self.name, status, body
            )));
        }
        let parsed: EmbedResponse = resp.json().await
            .map_err(|e| AvcError::ProviderUpstream(format!("embed.{}: bad json: {}", self.name, e)))?;
        Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
    }
}

pub fn make_embed(cfg: &Config, name: &str) -> AvcResult<Arc<dyn EmbedProvider>> {
    let pc = cfg.provider.embed.get(name).ok_or_else(|| {
        AvcError::NotFound(format!("provider.embed.{}", name))
    })?;
    Ok(Arc::new(OpenAiCompatEmbedProvider::new(name.to_string(), pc.clone())?))
}
```

加 use:
```rust
use super::{ChatMessage, EmbedProvider, LlmProvider};
```

**Step 2: 单测覆盖工厂 + 错误路径**

在 real.rs 的 `mod tests` 里加：

```rust
#[test]
fn embed_factory_returns_404_for_unknown_name() {
    let cfg = Config::default();
    let r = make_embed(&cfg, "ghost");
    assert!(matches!(r, Err(AvcError::NotFound(_))));
}

#[test]
fn embed_factory_succeeds_with_api_key() {
    let mut cfg = Config::default();
    cfg.provider.embed.insert(
        "openai".into(),
        ProviderCfg {
            api_key: Some("sk-test".into()),
            model: Some("text-embedding-3-small".into()),
            ..Default::default()
        },
    );
    let p = make_embed(&cfg, "openai").expect("ok");
    assert_eq!(p.name(), "openai");
}
```

**Step 3: 跑 `cargo test --locked --lib provider::real`**

Expected：5 unit tests pass（原 3 个 llm + 新增 2 个 embed）。

**Step 4: 跑既有集成测试**

Run: `cargo test --locked --test integration -- --test-threads=1`
Expected: 18 passed, 零回归。

**Step 5: 提交**

```bash
git add src/provider/real.rs
git commit -m "feat(provider): add OpenAiCompatEmbedProvider + factory"
```

---

### Task 2: provider test 加 embed 探针 + 集成测试

**Files:**
- Modify: `src/cli/provider.rs`
- Modify: `tests/integration.rs`

**Step 1: 在 provider.rs test verb 内嵌 embed 分支**

```rust
"embed" => {
    let cfg = Config::load(&Config::default_config_path()?)?;
    let provider = crate::provider::real::make_embed(&cfg, name)?;
    let sample = vec!["hello", "world"];
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AvcError::Internal(e.to_string()))?;
    let vectors = rt.block_on(provider.embed(&sample))?;
    let payload = json!({
        "provider": target,
        "ok": true,
        "count": vectors.len(),
        "dim": vectors.first().map(|v| v.len()).unwrap_or(0),
    });
    print(mode, &payload)?;
}
```

**Step 2: 加集成测试 `provider_test_embed_unknown`**

```rust
#[test]
fn provider_test_embed_unknown() {
    // 复用 provider_test_unknown_llm_name_says_not_configured 模式
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["provider", "test", "embed.ghost"])
        .output()
        .unwrap();
    assert!(!r.status.success(), "应非 0；stderr={}", String::from_utf8_lossy(&r.stderr));
}
```

**Step 3: 跑全部测试**

Run: `cargo test --locked --all-targets -- --test-threads=1`
Expected: 5 unit + 19 integration = 24 pass, 零回归。

**Step 4: 提交**

```bash
git add src/cli/provider.rs tests/integration.rs
git commit -m "feat(provider): provider test embed.<name> 探针 + 单测"
```

---

### Task 3: 文档同步

**Files:**
- Modify: `docs/status.md`

**Step 1: 把 Phase 1.1 的 avatar / voice / video / embed 一行 ⬜ 拆**
```diff
- | avatar / voice / video / embed 真 Provider | ⬜ | Phase 1.1 续；按 `src/provider/real.rs` 模式复制 trait 实现 |
+ | embed 真 Provider（`openai_compat`）             | ✅ | `src/provider/real.rs::OpenAiCompatEmbedProvider`；同 LLM 模板，复用 base_url + extra_headers 走 OpenAI 兼容 `/embeddings` |
+ | avatar / voice / video 真 Provider               | ⬜ | Phase 1.1 续；按 `src/provider/real.rs` 模式复制 trait 实现 |
```

**Step 2: 测试矩阵加新行**

```
├── embed_factory_returns_404_for_unknown_name           [新增] Phase 1.1: embed factory NotFound
├── embed_factory_succeeds_with_api_key                 [新增] Phase 1.1: embed factory OK
└── provider_test_embed_unknown                         [新增] Phase 1.1: provider test embed.<name>
```

更新计数 18 → 19 + 3 → 5 unit。

**Step 3: api/README.md embed 行小订正**

把 `embed | openai_embed ...` 那行加注 ✅ 真实现已落地（Phase 1.1），说明同 LLM 段。

**Step 4: 提交**

```bash
git add docs/status.md docs/api/README.md
git commit -m "docs: Phase 1 Embed Provider 交付"
```

---

## 验收

* `cargo test --locked --all-targets -- --test-threads=1` 全绿（5 unit + 19 integration = 24）
* `cargo build --locked` 无新 warning
* 手验：

  ```bash
  # 起一个最小 HTTP mock：返 fake embeddings
  ./avc provider test embed.<name>
  ```

---

## 不做的明确清单（后续 plan）

* DriftEvaluator 真算（接 embed.voice + 现有 face/style）
* `svc/finetune::publish` 增 `--drift-eval` 调用真 embed
* avatar / voice / video 真 Provider
* corpus create + embed 真切 chunk + write
