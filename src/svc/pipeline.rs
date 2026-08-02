//! pipeline-svc：DAG 引擎
//!
//! 节点种类: avatar / voice / llm / video / embed / compose / gate / branch
//! 每个节点可声明 `input_from` 依赖前节点输出（同名 JSON 字段）。
//! 调度: 拓扑排序 → 按顺序执行 → 节点结果落 `job_steps` (status, attempt, outputs_json, error_json, duration_ms)。
//! 节点产物落 `artifacts` (kind, name, content BLOB, mime, byte_size, sha256)。
//!
//! Phase 1.2 起节点 handler 内置：script_gen (LLM) / tts (voice) / img_gen (avatar) /
//! i2v (video) / compose (聚合 BLOB 为 mp4-like 占位)。Phase 1 仅调用 Mock Provider 填
//! 真实 BLOB；Wave C 再切到 LLM/voice/avatar/video 真 Provider trait impl。

use std::time::Instant;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::db::Db;
use crate::error::{AvcError, AvcResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub input_from: Vec<String>,
    #[serde(default)]
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagSpec {
    pub nodes: Vec<NodeSpec>,
}

/// 节点的输出（落 job_steps.outputs_json）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOutput {
    pub kind: String,
    /// base64-encoded bytes；为 None 时节点无 BLOB 产物
    #[serde(default)]
    pub blob: Option<String>,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub meta: serde_json::Value,
    #[serde(default)]
    pub artifact_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

impl NodeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeStatus::Pending => "pending",
            NodeStatus::Running => "running",
            NodeStatus::Succeeded => "succeeded",
            NodeStatus::Failed => "failed",
            NodeStatus::Skipped => "skipped",
        }
    }
}

/// 内置渲染 spec: 5 节点
pub fn render_publishment_spec() -> DagSpec {
    DagSpec {
        nodes: vec![
            NodeSpec {
                id: "script_gen".into(),
                kind: "llm".into(),
                when: None,
                input_from: vec![],
                config: serde_json::json!({"duration": 30, "llm_provider": "mock"}),
            },
            NodeSpec {
                id: "tts".into(),
                kind: "voice".into(),
                when: None,
                input_from: vec!["script_gen".into()],
                config: serde_json::json!({}),
            },
            NodeSpec {
                id: "img_gen".into(),
                kind: "avatar".into(),
                when: None,
                input_from: vec!["script_gen".into()],
                config: serde_json::json!({}),
            },
            NodeSpec {
                id: "i2v".into(),
                kind: "video".into(),
                when: None,
                // i2v 依赖 tts（audio WAV）+ img_gen（PNG）+ script_gen（脚本文本 / 时长）。
                // script_gen 是显式 DAG 依赖：避免从 audio/image 反推（也不准）。
                input_from: vec!["tts".into(), "img_gen".into(), "script_gen".into()],
                config: serde_json::json!({}),
            },
            NodeSpec {
                id: "compose".into(),
                kind: "compose".into(),
                when: None,
                input_from: vec!["i2v".into()],
                config: serde_json::json!({}),
            },
        ],
    }
}

/// 拓扑排序：按 input_from 顺序入图，检测 cycle。
/// 拓扑排序：按 input_from 顺序入图，检测 cycle。
fn topo_sort(spec: &DagSpec) -> AvcResult<Vec<String>> {
    // deps[node] = n 的前驱列表（先于 n 执行）
    let mut deps: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for n in &spec.nodes {
        deps.insert(&n.id, vec![]);
    }
    for n in &spec.nodes {
        for dep in &n.input_from {
            deps.entry(n.id.as_str()).or_default().push(dep.as_str());
        }
    }
    // nodes_to_drain: 尚未处理（前驱都已 push 完成）的节点集合
    let mut remaining: std::collections::HashMap<&str, usize> =
        deps.iter().map(|(k, v)| (*k, v.len())).collect();
    let mut order = Vec::new();
    // Kahn 算法：重复 "找无前驱 (deps 全部已 drained) 节点"
    while !remaining.is_empty() {
        // Keep execution deterministic by selecting the first ready node in spec order.
        // This also ensures tts failure stops the render before sibling/downstream work.
        let next = spec
            .nodes
            .iter()
            .find(|node| remaining.get(node.id.as_str()) == Some(&0))
            .map(|node| node.id.as_str());
        let Some(next) = next else {
            return Err(AvcError::Internal("DAG cycle detected".into()));
        };
        remaining.remove(next);
        order.push(next.to_string());
        // next 的执行减少它的"被依赖者"的前驱计数
        for (k, v) in deps.iter() {
            if v.contains(&next) {
                if let Some(c) = remaining.get_mut(k) {
                    *c -= 1;
                }
            }
        }
    }
    if order.len() != spec.nodes.len() {
        return Err(AvcError::Internal("DAG topo sort failed".into()));
    }
    Ok(order)
}

/// 真调度：在 db connection 上同步执行，节点结果落 job_steps，
/// 节点产物 BLOB 落 artifacts 表。
pub fn run(db: &Db, job_id: &str, spec: &DagSpec, topic: &str) -> AvcResult<()> {
    run_with_overrides(db, job_id, spec, topic, None, None, None, None)
}

pub fn run_with_llm_provider(
    db: &Db,
    job_id: &str,
    spec: &DagSpec,
    topic: &str,
    llm_override: Option<std::sync::Arc<dyn crate::provider::LlmProvider>>,
) -> AvcResult<()> {
    run_with_overrides(db, job_id, spec, topic, llm_override, None, None, None)
}

pub fn run_with_voice_provider(
    db: &Db,
    job_id: &str,
    spec: &DagSpec,
    topic: &str,
    voice_override: Option<std::sync::Arc<dyn crate::provider::VoiceProvider>>,
) -> AvcResult<()> {
    run_with_overrides(db, job_id, spec, topic, None, voice_override, None, None)
}

pub fn run_with_avatar_provider(
    db: &Db,
    job_id: &str,
    spec: &DagSpec,
    topic: &str,
    avatar_override: Option<std::sync::Arc<dyn crate::provider::AvatarProvider>>,
) -> AvcResult<()> {
    run_with_overrides(db, job_id, spec, topic, None, None, avatar_override, None)
}

pub fn run_with_video_provider(
    db: &Db,
    job_id: &str,
    spec: &DagSpec,
    topic: &str,
    video_override: Option<std::sync::Arc<dyn crate::provider::VideoProvider>>,
) -> AvcResult<()> {
    run_with_overrides(db, job_id, spec, topic, None, None, None, video_override)
}

/// 同时接受 LLM + Voice + Avatar + Video override；单元/集成测试 + 未来 CLI 注入统一入口。
#[allow(clippy::too_many_arguments)]
pub fn run_with_overrides(
    db: &Db,
    job_id: &str,
    spec: &DagSpec,
    topic: &str,
    llm_override: Option<std::sync::Arc<dyn crate::provider::LlmProvider>>,
    voice_override: Option<std::sync::Arc<dyn crate::provider::VoiceProvider>>,
    avatar_override: Option<std::sync::Arc<dyn crate::provider::AvatarProvider>>,
    video_override: Option<std::sync::Arc<dyn crate::provider::VideoProvider>>,
) -> AvcResult<()> {
    let order = topo_sort(spec)?;
    let cfg = Config::load(&Config::default_config_path()?)?;
    let nodes: std::collections::HashMap<&str, &NodeSpec> =
        spec.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut outputs: std::collections::HashMap<String, NodeOutput> =
        std::collections::HashMap::new();
    let now = crate::svc::now_iso();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status='running', current_step='pending' WHERE id=?",
            [job_id],
        )?;
    }
    for node_id in order {
        let node = nodes
            .get(node_id.as_str())
            .copied()
            .ok_or_else(|| AvcError::Internal(format!("node '{}' missing", node_id)))?;
        let started_at = now.clone();
        let started = Instant::now();
        // 写 step pending → running
        {
            let conn = db.conn.lock().unwrap();
            let step_id = crate::svc::new_id("stp");
            conn.execute(
                "INSERT INTO job_steps
                    (id, job_id, node_id, status, attempt, outputs_json, error_json,
                     started_at, finished_at, duration_ms)
                 VALUES (?1, ?2, ?3, 'running', 1, NULL, NULL, ?4, NULL, NULL)",
                rusqlite::params![&step_id, job_id, &node.id, &started_at],
            )?;
        }
        // 执行
        let exec = execute_node(
            node,
            &outputs,
            job_id,
            topic,
            &cfg,
            llm_override.clone(),
            voice_override.clone(),
            avatar_override.clone(),
            video_override.clone(),
        );
        let duration_ms = started.elapsed().as_millis() as i64;
        let finished_at = crate::svc::now_iso();
        match exec {
            Ok(out) => {
                // 写 step succeeded + artifact BLOB
                let artifact_id = if let Some(blob_b64) = &out.blob {
                    let conn = db.conn.lock().unwrap();
                    let id = crate::svc::new_id("art");
                    let mime = out
                        .mime
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".to_string());
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(blob_b64)
                        .map_err(|e: base64::DecodeError| {
                            AvcError::Internal(format!("b64: {}", e))
                        })?;
                    let byte_size = bytes.len() as i64;
                    let mut h = Sha256::new();
                    h.update(&bytes);
                    let sha = hex::encode(h.finalize());
                    conn.execute(
                        "INSERT INTO artifacts
                            (id, job_id, kind, name, content, mime, byte_size, sha256, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        rusqlite::params![
                            &id,
                            job_id,
                            &node.id,
                            &node.id,
                            &bytes,
                            &mime,
                            byte_size,
                            &sha,
                            &finished_at,
                        ],
                    )?;
                    let mut out_clone = out.clone();
                    out_clone.artifact_id = Some(id.clone());
                    outputs.insert(node.id.clone(), out_clone);
                    Some(id)
                } else {
                    outputs.insert(node.id.clone(), out);
                    None
                };
                let conn = db.conn.lock().unwrap();
                let step_id: String = conn.query_row(
                    "SELECT id FROM job_steps WHERE job_id = ?1 AND node_id = ?2 AND status='running' ORDER BY started_at DESC LIMIT 1",
                    rusqlite::params![job_id, &node.id],
                    |r| r.get(0),
                )?;
                let outputs_json =
                    serde_json::to_string(outputs.get(&node.id).unwrap_or(&NodeOutput {
                        kind: node.kind.clone(),
                        blob: None,
                        mime: None,
                        meta: serde_json::json!({}),
                        artifact_id: artifact_id.clone(),
                    }))?;
                conn.execute(
                    "UPDATE job_steps SET status='succeeded', outputs_json=?1,
                        finished_at=?2, duration_ms=?3
                     WHERE id=?4",
                    rusqlite::params![&outputs_json, &finished_at, duration_ms, &step_id],
                )?;
                let _ = artifact_id;
            }
            Err(e) => {
                let conn = db.conn.lock().unwrap();
                conn.execute(
                    "UPDATE job_steps SET status='failed', error_json=?1, finished_at=?2,
                        duration_ms=?3
                     WHERE job_id=?4 AND node_id=?5 AND status='running'",
                    rusqlite::params![e.to_string(), &finished_at, duration_ms, job_id, &node.id],
                )?;
                conn.execute(
                    "UPDATE jobs SET status='failed', error_json=?1, current_step=?2,
                        finished_at=?3 WHERE id=?4",
                    rusqlite::params![e.to_string(), &node.id, &finished_at, job_id],
                )?;
                return Err(e);
            }
        }
    }
    // 全部成功
    let finished_at_all = crate::svc::now_iso();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "UPDATE jobs SET status='succeeded', current_step='done', finished_at=?1
         WHERE id=?2",
        rusqlite::params![&finished_at_all, job_id],
    )?;
    Ok(())
}

/// 单节点执行（同步；生产应 async）。Phase 1 用 mock 数据生成占位 BLOB。
#[allow(clippy::too_many_arguments)]
fn execute_node(
    node: &NodeSpec,
    inputs: &std::collections::HashMap<String, NodeOutput>,
    _job_id: &str,
    topic: &str,
    cfg: &Config,
    llm_override: Option<std::sync::Arc<dyn crate::provider::LlmProvider>>,
    voice_override: Option<std::sync::Arc<dyn crate::provider::VoiceProvider>>,
    avatar_override: Option<std::sync::Arc<dyn crate::provider::AvatarProvider>>,
    video_override: Option<std::sync::Arc<dyn crate::provider::VideoProvider>>,
) -> AvcResult<NodeOutput> {
    match node.kind.as_str() {
        "llm" => {
            let script = generate_script(node, topic, cfg, llm_override)?;
            Ok(NodeOutput {
                kind: "script".into(),
                blob: Some(base64::engine::general_purpose::STANDARD.encode(script.as_bytes())),
                mime: Some("text/plain; charset=utf-8".into()),
                meta: serde_json::json!({"duration_ms": duration_ms(node), "provider": script_provider_name(node, cfg)}),
                artifact_id: None,
            })
        }
        "voice" => {
            let text = required_text_input(node, inputs)?;
            let voice_name = voice_provider_name(node);
            let provider = resolve_voice_provider(cfg, voice_override.clone(), &voice_name)?;
            // 真 Provider（包括 mock）走 synth(voice, text)。WAV BLOB 由 provider 决定；
            // 不再用"MOCK_TTS:..."占位。失败直接冒泡 → job 状态 failed + 后续节点不跑。
            let voice_ref = crate::provider::Voice {
                provider: provider.name().to_string(),
                provider_version: "openai_compat".into(),
                voice_id_remote: None,
                sample_wav_b64: String::new(),
                transcript: None,
                embed_b64: None,
                embed_dim: None,
            };
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| AvcError::Internal(format!("tokio runtime: {e}")))?;
            let audio = rt.block_on(provider.synth(&voice_ref, &text))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&audio.wav_b64)
                .map_err(|e| AvcError::Internal(format!("voice wav_b64 decode: {e}")))?;
            Ok(NodeOutput {
                kind: "audio".into(),
                blob: Some(audio.wav_b64),
                mime: Some(audio.mime),
                meta: serde_json::json!({
                    "provider": provider.name(),
                    "bytes": bytes.len(),
                    "input_text": text,
                }),
                artifact_id: None,
            })
        }
        "avatar" => {
            // Wave C: 真调 avatar provider；exact script text → prompt → primary PNG。
            // - 优先注入的 avatar_override；mock → 内置 MockAvatarProvider（离线默认）；
            //   其它 → make_avatar(cfg, name)；不存在 → NotFound，节点失败冒泡。
            // - 失败（429 / Upstream / TokenAuth / Timeout）直接冒泡，job 状态 failed，
            //   下游 i2v/compose 不再跑。无占位 fallback。
            // - 持久化精确 base64 解码后的 primary PNG bytes；mime = image/png；
            //   meta 携带 provider / model_id / bytes / prompt。
            let script = required_text_input(node, inputs)?;
            let avatar_name = avatar_provider_name(node);
            let provider = resolve_avatar_provider(cfg, avatar_override.clone(), &avatar_name)?;
            let spec = crate::provider::AvatarSpec {
                prompt: script.clone(),
                style: None,
                ref_image_paths: Vec::new(),
            };
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| AvcError::Internal(format!("tokio runtime: {e}")))?;
            let avatar = rt.block_on(provider.create(&spec))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&avatar.primary_png_b64)
                .map_err(|e| AvcError::Internal(format!("avatar primary_png_b64 decode: {e}")))?;
            Ok(NodeOutput {
                kind: "image".into(),
                blob: Some(avatar.primary_png_b64),
                mime: Some("image/png".into()),
                meta: serde_json::json!({
                    "provider": provider.name(),
                    "model_id": avatar.model_id,
                    "bytes": bytes.len(),
                    "prompt": script,
                }),
                artifact_id: None,
            })
        }
        "video" => {
            // Wave C: 真调 video provider；exact script_text + wav_b64 + png_b64 → render()。
            // - 依赖项：必须从 inputs 取 script_gen (script) / tts (audio) / img_gen (image)
            //   三个 exact NodeOutput；任意缺失 / kind 不符 / blob 缺失 → 节点失败冒泡。
            // - 注入优先 video_override；mock → 内置 MockVideoProvider（离线默认）；
            //   其它 → make_video(cfg, name)；不存在 → NotFound，节点失败冒泡（无占位 fallback）。
            // - 失败（429 / Upstream / Timeout）直接冒泡，job 状态 failed，
            //   下游 compose 不再跑。
            // - 持久化精确 base64 解码后的 mp4 bytes；mime = video/mp4；
            //   meta 携带 provider / duration_ms / bytes / prompt(脚本 text)。
            let script_text = required_script_text_input(node, inputs, "script_gen")?;
            let wav_bytes = required_audio_blob(node, inputs, "tts")?;
            let png_bytes = required_image_blob(node, inputs, "img_gen")?;
            let video_name = video_provider_name(node);
            let provider = resolve_video_provider(cfg, video_override.clone(), &video_name)?;
            let voice = crate::provider::Voice {
                provider: provider.name().to_string(),
                provider_version: "pipeline".into(),
                voice_id_remote: None,
                sample_wav_b64: base64::engine::general_purpose::STANDARD.encode(&wav_bytes),
                transcript: None,
                embed_b64: None,
                embed_dim: None,
            };
            let avatar = crate::provider::Avatar {
                provider: "pipeline".into(),
                provider_version: "pipeline".into(),
                model_id: None,
                primary_png_b64: base64::engine::general_purpose::STANDARD.encode(&png_bytes),
                views_zip_b64: None,
                face_id: None,
            };
            let scenes = vec![crate::provider::ScriptSegment {
                scene_index: 0,
                text: script_text.clone(),
                duration_ms: duration_ms(node),
            }];
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| AvcError::Internal(format!("tokio runtime: {e}")))?;
            let clip = rt.block_on(provider.render(&voice, &avatar, &scenes))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&clip.mp4_b64)
                .map_err(|e| AvcError::Internal(format!("video mp4_b64 decode: {e}")))?;
            Ok(NodeOutput {
                kind: "clip".into(),
                blob: Some(clip.mp4_b64),
                mime: Some(clip.mime),
                meta: serde_json::json!({
                    "provider": provider.name(),
                    "duration_ms": clip.duration_ms,
                    "bytes": bytes.len(),
                    "prompt": script_text,
                }),
                artifact_id: None,
            })
        }
        "compose" => Ok(NodeOutput {
            kind: "final_video".into(),
            blob: Some(
                base64::engine::general_purpose::STANDARD
                    .encode(b"\x00\x00\x00\x18ftypMOCK_FINAL_MP4"),
            ),
            mime: Some("video/mp4".into()),
            meta: serde_json::json!({"duration_ms": 30000}),
            artifact_id: None,
        }),
        other => Err(AvcError::Internal(format!(
            "unsupported node kind: {other}"
        ))),
    }
}

fn duration_ms(node: &NodeSpec) -> i64 {
    node.config
        .get("duration")
        .and_then(|v| v.as_i64())
        .unwrap_or(30)
        * 1000
}

fn script_provider_name(node: &NodeSpec, _cfg: &Config) -> String {
    node.config
        .get("llm_provider")
        .and_then(|v| v.as_str())
        .unwrap_or("mock")
        .to_string()
}

/// 解析 voice 节点 `node.config["voice_provider"]`；缺省 / 非字符串 → "mock"（保留离线默认）。
fn voice_provider_name(node: &NodeSpec) -> String {
    node.config
        .get("voice_provider")
        .and_then(|v| v.as_str())
        .unwrap_or("mock")
        .to_string()
}

/// 决定 avatar 节点用哪个 `AvatarProvider`：
/// - 优先用注入的 `avatar_override`（测试 / 显式 Provider 用）
/// - 否则 name == "mock" 走内置 `MockAvatarProvider`（不读 cfg，离线默认）
/// - 否则从 cfg.provider.avatar 取真 provider；不存在 → `NotFound`
///
/// 出错（NotFound / RateLimited / ProviderUpstream 等）直接返 Err，节点失败冒泡。
fn resolve_avatar_provider(
    cfg: &Config,
    avatar_override: Option<std::sync::Arc<dyn crate::provider::AvatarProvider>>,
    name: &str,
) -> AvcResult<std::sync::Arc<dyn crate::provider::AvatarProvider>> {
    if let Some(p) = avatar_override {
        return Ok(p);
    }
    if name == "mock" || name.is_empty() {
        return Ok(std::sync::Arc::new(
            crate::provider::mock::MockAvatarProvider {
                name: "mock".into(),
            },
        ));
    }
    crate::provider::real::make_avatar(cfg, name)
}

/// 解析 avatar 节点 `node.config["avatar_provider"]`；缺省 / 非字符串 → "mock"（保留离线默认）。
fn avatar_provider_name(node: &NodeSpec) -> String {
    node.config
        .get("avatar_provider")
        .and_then(|v| v.as_str())
        .unwrap_or("mock")
        .to_string()
}

/// 解析 video 节点 `node.config["video_provider"]`；缺省 / 非字符串 → "mock"（保留离线默认）。
fn video_provider_name(node: &NodeSpec) -> String {
    node.config
        .get("video_provider")
        .and_then(|v| v.as_str())
        .unwrap_or("mock")
        .to_string()
}

/// 决定 video 节点用哪个 `VideoProvider`：
/// - 优先用注入的 `video_override`（测试 / 显式 Provider 用）
/// - 否则 name == "mock" 走内置 `MockVideoProvider`（不读 cfg，离线默认）
/// - 否则从 cfg.provider.video 取真 provider；不存在 → `NotFound`
///
/// 出错（NotFound / ProviderUpstream 等）直接返 Err，节点失败冒泡。
fn resolve_video_provider(
    cfg: &Config,
    video_override: Option<std::sync::Arc<dyn crate::provider::VideoProvider>>,
    name: &str,
) -> AvcResult<std::sync::Arc<dyn crate::provider::VideoProvider>> {
    if let Some(p) = video_override {
        return Ok(p);
    }
    if name.is_empty() || (name == "mock" && !cfg.provider.video.contains_key(name)) {
        return Ok(std::sync::Arc::new(
            crate::provider::mock::MockVideoProvider {
                name: "mock".into(),
            },
        ));
    }
    crate::provider::real::make_video(cfg, name)
}

/// 决定 tts 节点用哪个 `VoiceProvider`：
/// - 优先用注入的 `voice_override`（测试 / 显式 Provider 用）
/// - 否则 name == "mock" 走内置 `MockVoiceProvider`（不读 cfg，离线默认）
/// - 否则从 cfg.provider.voice 取真 provider；不存在 → `NotFound`
///
/// 出错（NotFound / ProviderUpstream 等）直接返 Err，节点失败冒泡。
fn resolve_voice_provider(
    cfg: &Config,
    voice_override: Option<std::sync::Arc<dyn crate::provider::VoiceProvider>>,
    name: &str,
) -> AvcResult<std::sync::Arc<dyn crate::provider::VoiceProvider>> {
    if let Some(p) = voice_override {
        return Ok(p);
    }
    if name == "mock" || name.is_empty() {
        return Ok(std::sync::Arc::new(
            crate::provider::mock::MockVoiceProvider {
                name: "mock".into(),
            },
        ));
    }
    crate::provider::real::make_voice(cfg, name)
}

fn generate_script(
    node: &NodeSpec,
    topic: &str,
    cfg: &Config,
    override_provider: Option<std::sync::Arc<dyn crate::provider::LlmProvider>>,
) -> AvcResult<String> {
    let provider_name = script_provider_name(node, cfg);
    let provider = if provider_name == "mock" {
        std::sync::Arc::new(crate::provider::mock::MockLlmProvider {
            name: "mock".into(),
        }) as std::sync::Arc<dyn crate::provider::LlmProvider>
    } else if let Some(provider) = override_provider {
        provider
    } else {
        crate::provider::real::make_llm(cfg, &provider_name)?
    };
    let duration = duration_ms(node) / 1000;
    let messages = vec![
        crate::provider::ChatMessage {
            role: "system".into(),
            content: "You generate a concise spoken video script. Return only the script text."
                .into(),
        },
        crate::provider::ChatMessage {
            role: "user".into(),
            content: format!("Topic: {topic}\nDuration: {duration} seconds"),
        },
    ];
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AvcError::Internal(format!("tokio runtime: {e}")))?;
    let reply = rt.block_on(provider.chat(&messages))?;
    if reply.trim().is_empty() {
        return Err(AvcError::ProviderUpstream(
            "LLM returned an empty script".into(),
        ));
    }
    Ok(reply)
}

fn required_text_input(
    node: &NodeSpec,
    inputs: &std::collections::HashMap<String, NodeOutput>,
) -> AvcResult<String> {
    let dep = node.input_from.first().ok_or_else(|| {
        AvcError::Internal(format!(
            "node '{}' requires a text input dependency",
            node.id
        ))
    })?;
    let output = inputs.get(dep).ok_or_else(|| {
        AvcError::Internal(format!(
            "node '{}' missing dependency output '{}'",
            node.id, dep
        ))
    })?;
    if output.kind != "script" {
        return Err(AvcError::Internal(format!(
            "dependency '{}' is not script output",
            dep
        )));
    }
    let blob = output
        .blob
        .as_deref()
        .ok_or_else(|| AvcError::Internal(format!("dependency '{}' has no blob", dep)))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(blob)
        .map_err(|e| AvcError::Internal(format!("dependency '{}' invalid base64: {}", dep, e)))?;
    String::from_utf8(bytes)
        .map_err(|e| AvcError::Internal(format!("dependency '{}' invalid UTF-8: {}", dep, e)))
}

/// 强类型：从指定 named 依赖取 script text（kind == "script"，blob 是 base64 文本）。
/// 用于 video 节点：依赖 script_gen 必须存在且内容完整。
fn required_script_text_input(
    node: &NodeSpec,
    inputs: &std::collections::HashMap<String, NodeOutput>,
    dep: &str,
) -> AvcResult<String> {
    if !node.input_from.iter().any(|d| d == dep) {
        return Err(AvcError::Internal(format!(
            "node '{}' missing required DAG dep '{}'",
            node.id, dep
        )));
    }
    let output = inputs.get(dep).ok_or_else(|| {
        AvcError::Internal(format!(
            "node '{}' missing dependency output '{}'",
            node.id, dep
        ))
    })?;
    if output.kind != "script" {
        return Err(AvcError::Internal(format!(
            "dependency '{}' is not script output (kind={})",
            dep, output.kind
        )));
    }
    let blob = output.blob.as_deref().ok_or_else(|| {
        AvcError::Internal(format!("dependency '{}' has no blob (text required)", dep))
    })?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(blob)
        .map_err(|e| AvcError::Internal(format!("dependency '{}' invalid base64: {}", dep, e)))?;
    String::from_utf8(bytes)
        .map_err(|e| AvcError::Internal(format!("dependency '{}' invalid UTF-8: {}", dep, e)))
}

/// 强类型：从指定 named 依赖取 audio 字节（kind == "audio"，blob 是 base64 WAV）。
/// 用于 video 节点：依赖 tts 必须存在且 blob 可解码为音频字节。
fn required_audio_blob(
    node: &NodeSpec,
    inputs: &std::collections::HashMap<String, NodeOutput>,
    dep: &str,
) -> AvcResult<Vec<u8>> {
    if !node.input_from.iter().any(|d| d == dep) {
        return Err(AvcError::Internal(format!(
            "node '{}' missing required DAG dep '{}'",
            node.id, dep
        )));
    }
    let output = inputs.get(dep).ok_or_else(|| {
        AvcError::Internal(format!(
            "node '{}' missing dependency output '{}'",
            node.id, dep
        ))
    })?;
    if output.kind != "audio" {
        return Err(AvcError::Internal(format!(
            "dependency '{}' is not audio output (kind={})",
            dep, output.kind
        )));
    }
    let blob = output.blob.as_deref().ok_or_else(|| {
        AvcError::Internal(format!("dependency '{}' has no blob (audio required)", dep))
    })?;
    base64::engine::general_purpose::STANDARD
        .decode(blob)
        .map_err(|e| AvcError::Internal(format!("dependency '{}' invalid base64: {}", dep, e)))
}

/// 强类型：从指定 named 依赖取 image 字节（kind == "image"，blob 是 base64 PNG）。
/// 用于 video 节点：依赖 img_gen 必须存在且 blob 可解码为图片字节。
fn required_image_blob(
    node: &NodeSpec,
    inputs: &std::collections::HashMap<String, NodeOutput>,
    dep: &str,
) -> AvcResult<Vec<u8>> {
    if !node.input_from.iter().any(|d| d == dep) {
        return Err(AvcError::Internal(format!(
            "node '{}' missing required DAG dep '{}'",
            node.id, dep
        )));
    }
    let output = inputs.get(dep).ok_or_else(|| {
        AvcError::Internal(format!(
            "node '{}' missing dependency output '{}'",
            node.id, dep
        ))
    })?;
    if output.kind != "image" {
        return Err(AvcError::Internal(format!(
            "dependency '{}' is not image output (kind={})",
            dep, output.kind
        )));
    }
    let blob = output.blob.as_deref().ok_or_else(|| {
        AvcError::Internal(format!("dependency '{}' has no blob (image required)", dep))
    })?;
    base64::engine::general_purpose::STANDARD
        .decode(blob)
        .map_err(|e| AvcError::Internal(format!("dependency '{}' invalid base64: {}", dep, e)))
}

/// 旧 stub 保留兼容外部 import
pub fn execute(dag: &DagSpec) -> AvcResult<()> {
    let _ = dag;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_WAV: &[u8] = b"RIFF\x10\x00\x00\x00WAVEdeterministic";

    fn voice_node(config: serde_json::Value) -> NodeSpec {
        NodeSpec {
            id: "tts".into(),
            kind: "voice".into(),
            when: None,
            input_from: vec!["script_gen".into()],
            config,
        }
    }

    fn script_inputs(text: &str) -> std::collections::HashMap<String, NodeOutput> {
        let mut inputs = std::collections::HashMap::new();
        inputs.insert(
            "script_gen".into(),
            NodeOutput {
                kind: "script".into(),
                blob: Some(base64::engine::general_purpose::STANDARD.encode(text.as_bytes())),
                mime: Some("text/plain; charset=utf-8".into()),
                meta: serde_json::json!({}),
                artifact_id: None,
            },
        );
        inputs
    }

    struct DeterministicVoiceProvider;

    #[async_trait]
    impl crate::provider::VoiceProvider for DeterministicVoiceProvider {
        fn name(&self) -> &str {
            "injected"
        }

        async fn clone(&self, _paths: &[String]) -> AvcResult<crate::provider::Voice> {
            unreachable!("clone is not used by the render pipeline")
        }

        async fn synth(
            &self,
            _voice: &crate::provider::Voice,
            text: &str,
        ) -> AvcResult<crate::provider::Audio> {
            assert_eq!(text, "exact script text");
            Ok(crate::provider::Audio {
                wav_b64: base64::engine::general_purpose::STANDARD.encode(TEST_WAV),
                mime: "audio/x-test-wav".into(),
            })
        }

        async fn finetune(
            &self,
            _base: &crate::provider::Voice,
            _paths: &[String],
            _cfg: &crate::provider::FinetuneConfig,
        ) -> AvcResult<crate::provider::Voice> {
            unreachable!("finetune is not used by the render pipeline")
        }
    }

    struct FailingVoiceProvider;

    #[async_trait]
    impl crate::provider::VoiceProvider for FailingVoiceProvider {
        fn name(&self) -> &str {
            "failing"
        }

        async fn clone(&self, _paths: &[String]) -> AvcResult<crate::provider::Voice> {
            unreachable!()
        }

        async fn synth(
            &self,
            _voice: &crate::provider::Voice,
            _text: &str,
        ) -> AvcResult<crate::provider::Audio> {
            Err(AvcError::RateLimited("deterministic 429".into()))
        }

        async fn finetune(
            &self,
            _base: &crate::provider::Voice,
            _paths: &[String],
            _cfg: &crate::provider::FinetuneConfig,
        ) -> AvcResult<crate::provider::Voice> {
            unreachable!()
        }
    }

    #[test]
    fn voice_provider_name_supports_default_and_explicit_provider() {
        assert_eq!(
            voice_provider_name(&voice_node(serde_json::json!({}))),
            "mock"
        );
        assert_eq!(
            voice_provider_name(&voice_node(serde_json::json!({"voice_provider": "local"}))),
            "local"
        );
    }

    #[test]
    fn voice_node_preserves_exact_injected_wav_mime_and_metadata() {
        let provider: std::sync::Arc<dyn crate::provider::VoiceProvider> =
            std::sync::Arc::new(DeterministicVoiceProvider);
        let output = execute_node(
            &voice_node(serde_json::json!({"voice_provider": "explicit"})),
            &script_inputs("exact script text"),
            "job-test",
            "topic",
            &Config::default(),
            None,
            Some(provider),
            None,
            None,
        )
        .unwrap();

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(output.blob.unwrap())
            .unwrap();
        assert_eq!(bytes, TEST_WAV);
        assert_eq!(output.mime.as_deref(), Some("audio/x-test-wav"));
        assert_eq!(output.meta["provider"], "injected");
        assert_eq!(output.meta["bytes"], TEST_WAV.len());
        assert_eq!(output.meta["input_text"], "exact script text");
    }

    #[test]
    fn voice_node_propagates_provider_error() {
        let provider: std::sync::Arc<dyn crate::provider::VoiceProvider> =
            std::sync::Arc::new(FailingVoiceProvider);
        let result = execute_node(
            &voice_node(serde_json::json!({})),
            &script_inputs("exact script text"),
            "job-test",
            "topic",
            &Config::default(),
            None,
            Some(provider),
            None,
            None,
        );
        assert!(
            matches!(result, Err(AvcError::RateLimited(message)) if message == "deterministic 429")
        );
    }

    #[test]
    fn topo_orders_simple_chain() {
        let spec = DagSpec {
            nodes: vec![
                NodeSpec {
                    id: "a".into(),
                    kind: "llm".into(),
                    when: None,
                    input_from: vec![],
                    config: serde_json::json!({}),
                },
                NodeSpec {
                    id: "b".into(),
                    kind: "voice".into(),
                    when: None,
                    input_from: vec!["a".into()],
                    config: serde_json::json!({}),
                },
                NodeSpec {
                    id: "c".into(),
                    kind: "compose".into(),
                    when: None,
                    input_from: vec!["b".into()],
                    config: serde_json::json!({}),
                },
            ],
        };
        let order = topo_sort(&spec).unwrap();
        let pa = order.iter().position(|n| n == "a").unwrap();
        let pb = order.iter().position(|n| n == "b").unwrap();
        let pc = order.iter().position(|n| n == "c").unwrap();
        assert!(pa < pb && pb < pc);
    }

    #[test]
    fn topo_detects_cycle() {
        let spec = DagSpec {
            nodes: vec![
                NodeSpec {
                    id: "a".into(),
                    kind: "llm".into(),
                    when: None,
                    input_from: vec!["b".into()],
                    config: serde_json::json!({}),
                },
                NodeSpec {
                    id: "b".into(),
                    kind: "voice".into(),
                    when: None,
                    input_from: vec!["a".into()],
                    config: serde_json::json!({}),
                },
            ],
        };
        assert!(topo_sort(&spec).is_err());
    }

    #[test]
    fn required_text_input_decodes_exact_utf8_script() {
        let text = "exact UTF-8 漢字\nline";
        let node = NodeSpec {
            id: "tts".into(),
            kind: "voice".into(),
            when: None,
            input_from: vec!["script_gen".into()],
            config: serde_json::json!({}),
        };
        let mut inputs = std::collections::HashMap::new();
        inputs.insert(
            "script_gen".into(),
            NodeOutput {
                kind: "script".into(),
                blob: Some(base64::engine::general_purpose::STANDARD.encode(text.as_bytes())),
                mime: Some("text/plain; charset=utf-8".into()),
                meta: serde_json::json!({}),
                artifact_id: None,
            },
        );
        assert_eq!(required_text_input(&node, &inputs).unwrap(), text);
    }

    #[test]
    fn required_text_input_rejects_missing_and_invalid_dependencies() {
        let node = NodeSpec {
            id: "tts".into(),
            kind: "voice".into(),
            when: None,
            input_from: vec!["script_gen".into()],
            config: serde_json::json!({}),
        };
        let inputs = std::collections::HashMap::new();
        assert!(required_text_input(&node, &inputs).is_err());

        let mut inputs = std::collections::HashMap::new();
        inputs.insert(
            "script_gen".into(),
            NodeOutput {
                kind: "script".into(),
                blob: Some("not base64".into()),
                mime: Some("text/plain; charset=utf-8".into()),
                meta: serde_json::json!({}),
                artifact_id: None,
            },
        );
        assert!(required_text_input(&node, &inputs).is_err());
    }

    #[test]
    fn script_generation_uses_stable_prompt_and_mock_reply() {
        let node = render_publishment_spec().nodes[0].clone();
        let reply = generate_script(&node, "topic exact", &Config::default(), None).unwrap();
        assert!(reply.contains("topic exact"));
        assert!(reply.contains("30"));
    }

    #[test]
    fn script_generation_rejects_empty_reply() {
        let mut node = render_publishment_spec().nodes[0].clone();
        node.config["llm_provider"] = serde_json::Value::String("custom".into());
        let provider: std::sync::Arc<dyn crate::provider::LlmProvider> =
            std::sync::Arc::new(EmptyLlmProvider);
        let result = generate_script(&node, "topic", &Config::default(), Some(provider));
        assert!(matches!(result, Err(AvcError::ProviderUpstream(_))));
    }

    fn avatar_node(config: serde_json::Value) -> NodeSpec {
        NodeSpec {
            id: "img_gen".into(),
            kind: "avatar".into(),
            when: None,
            input_from: vec!["script_gen".into()],
            config,
        }
    }

    const TEST_PNG: &[u8] = b"\x89PNG\r\n\x1a\navatar-deterministic-payload";

    struct DeterministicAvatarProvider;

    #[async_trait]
    impl crate::provider::AvatarProvider for DeterministicAvatarProvider {
        fn name(&self) -> &str {
            "injected-avatar"
        }

        async fn create(
            &self,
            spec: &crate::provider::AvatarSpec,
        ) -> AvcResult<crate::provider::Avatar> {
            assert_eq!(spec.prompt, "exact script text");
            Ok(crate::provider::Avatar {
                provider: "injected-avatar".into(),
                provider_version: "openai_compat".into(),
                model_id: Some("dall-e-3".into()),
                primary_png_b64: base64::engine::general_purpose::STANDARD.encode(TEST_PNG),
                views_zip_b64: None,
                face_id: None,
            })
        }

        async fn finetune(
            &self,
            _base: &crate::provider::Avatar,
            _ref_images: &[String],
            _cfg: &crate::provider::FinetuneConfig,
        ) -> AvcResult<crate::provider::Avatar> {
            unreachable!("finetune is not used by the render pipeline")
        }
    }

    struct FailingAvatarProvider;

    #[async_trait]
    impl crate::provider::AvatarProvider for FailingAvatarProvider {
        fn name(&self) -> &str {
            "failing-avatar"
        }

        async fn create(
            &self,
            _spec: &crate::provider::AvatarSpec,
        ) -> AvcResult<crate::provider::Avatar> {
            Err(AvcError::RateLimited("deterministic avatar 429".into()))
        }

        async fn finetune(
            &self,
            _base: &crate::provider::Avatar,
            _ref_images: &[String],
            _cfg: &crate::provider::FinetuneConfig,
        ) -> AvcResult<crate::provider::Avatar> {
            unreachable!()
        }
    }

    #[test]
    fn avatar_provider_name_supports_default_and_explicit_provider() {
        assert_eq!(
            avatar_provider_name(&avatar_node(serde_json::json!({}))),
            "mock"
        );
        assert_eq!(
            avatar_provider_name(&avatar_node(
                serde_json::json!({"avatar_provider": "local"})
            )),
            "local"
        );
    }

    #[test]
    fn avatar_node_default_uses_mock_provider_and_persists_png_payload() {
        // 不传 override + config 中无 avatar_provider → 内置 MockAvatarProvider。
        // - mime = image/png
        // - blob 是 base64 解码后 = MockAvatarProvider 的 primary PNG bytes
        // - meta.provider = "mock"
        // - meta.bytes = 解码后长度
        // - meta.prompt = 注入的脚本文本
        // - meta.model_id = Some(...)
        let output = execute_node(
            &avatar_node(serde_json::json!({})),
            &script_inputs("exact script text"),
            "job-test",
            "topic",
            &Config::default(),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(output.blob.unwrap())
            .unwrap();
        assert_eq!(output.mime.as_deref(), Some("image/png"));
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(output.meta["provider"], "mock");
        assert_eq!(output.meta["bytes"], bytes.len());
        assert_eq!(output.meta["prompt"], "exact script text");
        assert!(
            output.meta["model_id"].is_string(),
            "mock model_id should be present: {:?}",
            output.meta["model_id"]
        );
    }

    #[test]
    fn avatar_node_explicit_injected_provider_preserves_exact_png_payload() {
        // 显式 override：provider 来自注入；不是 mock；与 mock 输出无关。
        // - 持久化精确 primary PNG bytes（与 override 返回完全一致）
        // - mime = image/png
        // - meta = { provider: "injected-avatar", model_id, bytes, prompt: "exact script text" }
        let provider: std::sync::Arc<dyn crate::provider::AvatarProvider> =
            std::sync::Arc::new(DeterministicAvatarProvider);
        let output = execute_node(
            &avatar_node(serde_json::json!({"avatar_provider": "explicit"})),
            &script_inputs("exact script text"),
            "job-test",
            "topic",
            &Config::default(),
            None,
            None,
            Some(provider),
            None,
        )
        .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(output.blob.unwrap())
            .unwrap();
        assert_eq!(bytes, TEST_PNG);
        assert_eq!(output.mime.as_deref(), Some("image/png"));
        assert_eq!(output.meta["provider"], "injected-avatar");
        assert_eq!(output.meta["model_id"], "dall-e-3");
        assert_eq!(output.meta["bytes"], TEST_PNG.len());
        assert_eq!(output.meta["prompt"], "exact script text");
    }

    #[test]
    fn avatar_node_propagates_provider_error() {
        // provider 报 RateLimited → 节点失败冒泡；下游 i2v/compose 不跑。
        let provider: std::sync::Arc<dyn crate::provider::AvatarProvider> =
            std::sync::Arc::new(FailingAvatarProvider);
        let result = execute_node(
            &avatar_node(serde_json::json!({})),
            &script_inputs("exact script text"),
            "job-test",
            "topic",
            &Config::default(),
            None,
            None,
            Some(provider),
            None,
        );
        assert!(
            matches!(result, Err(AvcError::RateLimited(message)) if message == "deterministic avatar 429")
        );
    }

    use async_trait::async_trait;

    struct EmptyLlmProvider;

    #[async_trait]
    impl crate::provider::LlmProvider for EmptyLlmProvider {
        fn name(&self) -> &str {
            "empty-test"
        }

        async fn chat(&self, _msgs: &[crate::provider::ChatMessage]) -> AvcResult<String> {
            Ok(String::new())
        }
    }

    // ─── Video provider 单元测试 ──────────────────────────────────

    const TEST_MP4: &[u8] =
        b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00mp42isomvideo-deterministic-payload";
    const TEST_MP4_MIME: &str = "video/mp4";
    const TEST_VIDEO_DURATION_MS: i64 = 30_000;

    /// 通用 fixture：构造 video node + 三依赖 inputs（script/tts/img_gen）+ cfg。
    fn video_inputs(
        script_text: &str,
        wav: &[u8],
        png: &[u8],
    ) -> std::collections::HashMap<String, NodeOutput> {
        let mut inputs = std::collections::HashMap::new();
        inputs.insert(
            "script_gen".into(),
            NodeOutput {
                kind: "script".into(),
                blob: Some(
                    base64::engine::general_purpose::STANDARD.encode(script_text.as_bytes()),
                ),
                mime: Some("text/plain; charset=utf-8".into()),
                meta: serde_json::json!({"duration_ms": 30000}),
                artifact_id: None,
            },
        );
        inputs.insert(
            "tts".into(),
            NodeOutput {
                kind: "audio".into(),
                blob: Some(base64::engine::general_purpose::STANDARD.encode(wav)),
                mime: Some("audio/wav".into()),
                meta: serde_json::json!({"provider": "mock"}),
                artifact_id: None,
            },
        );
        inputs.insert(
            "img_gen".into(),
            NodeOutput {
                kind: "image".into(),
                blob: Some(base64::engine::general_purpose::STANDARD.encode(png)),
                mime: Some("image/png".into()),
                meta: serde_json::json!({"provider": "mock"}),
                artifact_id: None,
            },
        );
        inputs
    }

    fn video_node(config: serde_json::Value) -> NodeSpec {
        NodeSpec {
            id: "i2v".into(),
            kind: "video".into(),
            when: None,
            input_from: vec!["tts".into(), "img_gen".into(), "script_gen".into()],
            config,
        }
    }

    struct DeterministicVideoProvider;

    #[async_trait]
    impl crate::provider::VideoProvider for DeterministicVideoProvider {
        fn name(&self) -> &str {
            "injected-video"
        }

        async fn render(
            &self,
            voice: &crate::provider::Voice,
            avatar: &crate::provider::Avatar,
            scenes: &[crate::provider::ScriptSegment],
        ) -> AvcResult<crate::provider::Clip> {
            // 必须收到 exact 上游 wav bytes（解码 → 再 base64 → 与 provider 收的相等）
            let wav_in = base64::engine::general_purpose::STANDARD
                .decode(&voice.sample_wav_b64)
                .expect("wav b64");
            assert_eq!(
                wav_in, TEST_WAV,
                "video provider 必须收到 exact 上游 wav bytes"
            );
            let png_in = base64::engine::general_purpose::STANDARD
                .decode(&avatar.primary_png_b64)
                .expect("png b64");
            assert_eq!(
                png_in, TEST_PNG,
                "video provider 必须收到 exact 上游 png bytes"
            );
            // scenes 必须是单段且 text=exact script
            assert_eq!(scenes.len(), 1);
            assert_eq!(scenes[0].scene_index, 0);
            assert_eq!(scenes[0].text, "exact script text");
            // duration 来自 node.config.duration=30 → 30000 ms（脚本 generation 节点默认 30s）
            assert_eq!(scenes[0].duration_ms, 30_000);
            Ok(crate::provider::Clip {
                mp4_b64: base64::engine::general_purpose::STANDARD.encode(TEST_MP4),
                mime: TEST_MP4_MIME.into(),
                duration_ms: TEST_VIDEO_DURATION_MS,
            })
        }
    }

    struct FailingVideoProvider;

    #[async_trait]
    impl crate::provider::VideoProvider for FailingVideoProvider {
        fn name(&self) -> &str {
            "failing-video"
        }

        async fn render(
            &self,
            _voice: &crate::provider::Voice,
            _avatar: &crate::provider::Avatar,
            _scenes: &[crate::provider::ScriptSegment],
        ) -> AvcResult<crate::provider::Clip> {
            Err(AvcError::ProviderUpstream("deterministic video 502".into()))
        }
    }

    #[test]
    fn video_provider_name_supports_default_and_explicit_provider() {
        assert_eq!(
            video_provider_name(&video_node(serde_json::json!({}))),
            "mock"
        );
        assert_eq!(
            video_provider_name(&video_node(serde_json::json!({"video_provider": "kling"}))),
            "kling"
        );
    }

    #[test]
    fn video_node_injected_provider_receives_exact_inputs_and_persists_mp4() {
        // 注入 video provider：必须收到 exact 上游 wav/png/script text；
        // 持久化 exact MP4 bytes + mime + duration_ms + provider + prompt。
        let provider: std::sync::Arc<dyn crate::provider::VideoProvider> =
            std::sync::Arc::new(DeterministicVideoProvider);
        let output = execute_node(
            &video_node(serde_json::json!({"video_provider": "explicit"})),
            &video_inputs("exact script text", TEST_WAV, TEST_PNG),
            "job-test",
            "topic",
            &Config::default(),
            None,
            None,
            None,
            Some(provider),
        )
        .expect("ok");

        // 持久化 = provider 返的 exact mp4 bytes
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(output.blob.expect("blob"))
            .unwrap();
        assert_eq!(
            bytes, TEST_MP4,
            "video BLOB = provider 返的 exact mp4 bytes"
        );
        assert_eq!(output.kind, "clip");
        assert_eq!(output.mime.as_deref(), Some(TEST_MP4_MIME));
        assert_eq!(output.meta["provider"], "injected-video");
        assert_eq!(output.meta["duration_ms"], TEST_VIDEO_DURATION_MS);
        assert_eq!(output.meta["bytes"], TEST_MP4.len() as i64);
        assert_eq!(output.meta["prompt"], "exact script text");
    }

    #[test]
    fn video_node_default_uses_mock_provider_when_no_override_and_persists_mp4() {
        // 不传 override + config.video_provider="mock" → 内置 MockVideoProvider
        // mock 看 scenes 算 duration_ms = 30000 + mp4 magic bytes
        let output = execute_node(
            &video_node(serde_json::json!({})),
            &video_inputs("exact script text", TEST_WAV, TEST_PNG),
            "job-test",
            "topic",
            &Config::default(),
            None,
            None,
            None,
            None,
        )
        .expect("ok");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(output.blob.expect("blob"))
            .unwrap();
        // mock 返回固定的 mp4 占位（见 provider::mock::MockVideoProvider）
        assert!(
            bytes.starts_with(b"\x00\x00\x00\x18ftypmock"),
            "mock video 应以 ftyp mock 开头；actual={:?}",
            &bytes[..bytes.len().min(16)]
        );
        assert_eq!(output.meta["provider"], "mock");
        assert_eq!(output.meta["duration_ms"], 30_000);
        assert_eq!(output.meta["prompt"], "exact script text");
    }

    #[test]
    fn video_node_propagates_provider_error_and_no_fallback() {
        // provider 报 ProviderUpstream → 节点失败冒泡；不返占位 mp4。
        let provider: std::sync::Arc<dyn crate::provider::VideoProvider> =
            std::sync::Arc::new(FailingVideoProvider);
        let result = execute_node(
            &video_node(serde_json::json!({"video_provider": "explicit"})),
            &video_inputs("exact script text", TEST_WAV, TEST_PNG),
            "job-test",
            "topic",
            &Config::default(),
            None,
            None,
            None,
            Some(provider),
        );
        assert!(
            matches!(result, Err(AvcError::ProviderUpstream(ref message)) if message == "deterministic video 502"),
            "provider error 应原样冒泡；actual={:?}",
            result
        );
    }

    #[test]
    fn video_node_rejects_unknown_provider_name_without_fallback() {
        // 显式 video_provider="ghost" + 没 override → cfg 没配 → NotFound；
        // 不再走占位 mp4 路径（Wave C 要求：no named fallback）。
        let cfg = Config::default();
        // 确保 cfg 里没有 ghost 这个 video provider
        assert!(!cfg.provider.video.contains_key("ghost"));
        let result = execute_node(
            &video_node(serde_json::json!({"video_provider": "ghost"})),
            &video_inputs("exact script text", TEST_WAV, TEST_PNG),
            "job-test",
            "topic",
            &cfg,
            None,
            None,
            None,
            None,
        );
        assert!(
            matches!(result, Err(AvcError::NotFound(_))),
            "未注册的 video provider 名应 NotFound；actual={:?}",
            result
        );
    }

    #[test]
    fn video_node_rejects_missing_dependency_outputs() {
        // 缺任意依赖（script_gen / tts / img_gen）→ 节点失败冒泡
        let base = video_inputs("exact script text", TEST_WAV, TEST_PNG);
        for missing in ["script_gen", "tts", "img_gen"] {
            let mut inputs = base.clone();
            inputs.remove(missing);
            let result = execute_node(
                &video_node(serde_json::json!({})),
                &inputs,
                "job-test",
                "topic",
                &Config::default(),
                None,
                None,
                None,
                None,
            );
            assert!(
                result.is_err(),
                "缺 {} 依赖应报错；actual={:?}",
                missing,
                result
            );
        }
    }

    #[test]
    fn video_node_rejects_dependency_with_wrong_kind() {
        // 依赖 kind 不符（script_gen 不是 script；tts 不是 audio；img_gen 不是 image）
        // → 节点失败冒泡
        let mut inputs = video_inputs("exact script text", TEST_WAV, TEST_PNG);
        // 把 script_gen.kind 改成 audio → 应当 fail
        inputs.get_mut("script_gen").unwrap().kind = "audio".into();
        let result = execute_node(
            &video_node(serde_json::json!({})),
            &inputs,
            "job-test",
            "topic",
            &Config::default(),
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err(), "script_gen kind 错应报错");
    }

    #[test]
    fn video_node_rejects_node_without_required_dag_dep() {
        // video 节点 input_from 缺 script_gen → 应报错（不能从 audio/image 反推）
        let node = NodeSpec {
            id: "i2v".into(),
            kind: "video".into(),
            when: None,
            input_from: vec!["tts".into(), "img_gen".into()],
            config: serde_json::json!({}),
        };
        let result = execute_node(
            &node,
            &video_inputs("exact script text", TEST_WAV, TEST_PNG),
            "job-test",
            "topic",
            &Config::default(),
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err(), "video 缺 script_gen DAG dep 应报错");
    }

    #[test]
    fn video_node_injected_override_overrides_explicit_name() {
        // 注入 video_override 时，无论 node.config.video_provider 是什么，
        // 都用注入的 provider（"injected-video"）。
        let provider: std::sync::Arc<dyn crate::provider::VideoProvider> =
            std::sync::Arc::new(DeterministicVideoProvider);
        let output = execute_node(
            &video_node(serde_json::json!({"video_provider": "ignored-when-overridden"})),
            &video_inputs("exact script text", TEST_WAV, TEST_PNG),
            "job-test",
            "topic",
            &Config::default(),
            None,
            None,
            None,
            Some(provider),
        )
        .expect("ok");
        assert_eq!(
            output.meta["provider"], "injected-video",
            "override 应覆盖 config 里的 video_provider 名"
        );
    }
}
