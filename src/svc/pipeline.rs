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
                config: serde_json::json!({"duration": 30}),
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
                input_from: vec!["tts".into(), "img_gen".into()],
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
        let next: Vec<&str> = remaining
            .iter()
            .filter(|(_, c)| **c == 0)
            .map(|(k, _)| *k)
            .collect();
        if next.is_empty() {
            return Err(AvcError::Internal("DAG cycle detected".into()));
        }
        let next = next[0];
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
pub fn run(db: &Db, job_id: &str, spec: &DagSpec, _topic: &str) -> AvcResult<()> {
    let order = topo_sort(spec)?;
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
        let exec = execute_node(node, &outputs, job_id);
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
fn execute_node(
    node: &NodeSpec,
    _inputs: &std::collections::HashMap<String, NodeOutput>,
    _job_id: &str,
) -> AvcResult<NodeOutput> {
    match node.kind.as_str() {
        "llm" => Ok(NodeOutput {
            kind: "script".into(),
            blob: Some(
                base64::engine::general_purpose::STANDARD
                    .encode(format!("MOCK_SCRIPT:{}", node.id).as_bytes()),
            ),
            mime: Some("text/plain".into()),
            meta: serde_json::json!({"duration_ms": 30000}),
            artifact_id: None,
        }),
        "voice" => Ok(NodeOutput {
            kind: "audio".into(),
            blob: Some(base64::engine::general_purpose::STANDARD.encode(b"RIFF....MOCK_TTS")),
            mime: Some("audio/wav".into()),
            meta: serde_json::json!({"duration_ms": 30000}),
            artifact_id: None,
        }),
        "avatar" => Ok(NodeOutput {
            kind: "image".into(),
            blob: Some(
                base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nMOCK_IMG"),
            ),
            mime: Some("image/png".into()),
            meta: serde_json::json!({"resolution": "1080p"}),
            artifact_id: None,
        }),
        "video" => {
            // Phase 2: 真调 video provider（如 kling-cli 通过 binary 三段式）。
            // 步骤：load Config → make_video("mock") → 构造 fake Voice/Avatar/Scenes → render().await 同步阻塞。
            // 若 provider.video.<name> 没配 → 走占位 BLOB（与 Phase 1 兼容）。
            // XDG-aware 路径：XDG_CONFIG_HOME/avc/avc.toml
            let cfg_path = std::env::var("XDG_CONFIG_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_default()
                .join("avc")
                .join("avc.toml");
            let cfg = crate::config::Config::load(&cfg_path).unwrap_or_default();
            let video_name = node
                .config
                .get("video_provider")
                .and_then(|v| v.as_str())
                .unwrap_or("mock");
            let provider = match crate::provider::real::make_video(&cfg, video_name) {
                Ok(p) => p,
                Err(_) => {
                    // 没配 → 用占位 BLOB（Phase 1 行为）
                    return Ok(NodeOutput {
                        kind: "clip".into(),
                        blob: Some(
                            base64::engine::general_purpose::STANDARD
                                .encode(b"\x00\x00\x00\x18ftypMOCK_VIDEO"),
                        ),
                        mime: Some("video/mp4".into()),
                        meta: serde_json::json!({"duration_ms": 30000}),
                        artifact_id: None,
                    });
                }
            };
            let voice = crate::provider::Voice {
                provider: "mock".into(),
                provider_version: "stub".into(),
                voice_id_remote: None,
                sample_wav_b64: String::new(),
                transcript: None,
                embed_b64: None,
                embed_dim: None,
            };
            let avatar = crate::provider::Avatar {
                provider: "mock".into(),
                provider_version: "stub".into(),
                model_id: None,
                primary_png_b64: String::new(),
                views_zip_b64: None,
                face_id: None,
            };
            let scenes = vec![crate::provider::ScriptSegment {
                scene_index: 0,
                text: format!("scene from job {}", _job_id),
                duration_ms: 30000,
            }];
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let clip = rt.block_on(provider.render(&voice, &avatar, &scenes))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&clip.mp4_b64)
                .map_err(|e| AvcError::Internal(format!("b64: {}", e)))?;
            Ok(NodeOutput {
                kind: "clip".into(),
                blob: Some(clip.mp4_b64),
                mime: Some(clip.mime),
                meta: serde_json::json!({"duration_ms": clip.duration_ms, "bytes": bytes.len()}),
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

/// 旧 stub 保留兼容外部 import
pub fn execute(dag: &DagSpec) -> AvcResult<()> {
    let _ = dag;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn render_spec_has_five_nodes_in_order() {
        let spec = render_publishment_spec();
        assert_eq!(spec.nodes.len(), 5);
        let order = topo_sort(&spec).unwrap();
        assert_eq!(order.len(), 5);
        // script_gen 是根；compose 是终；tts/img_gen 都依赖 script_gen 互不依赖；
        // i2v 依赖 tts 与 img_gen。Kahn 算法保证依赖次序，不保证并列节点的选取。
        assert_eq!(order.first().unwrap(), "script_gen");
        assert_eq!(order.last().unwrap(), "compose");
        // i2v 必须在 tts 与 img_gen 之后
        let i2v = order.iter().position(|n| n == "i2v").unwrap();
        assert!(order.iter().position(|n| n == "tts").unwrap() < i2v);
        assert!(order.iter().position(|n| n == "img_gen").unwrap() < i2v);
        // script_gen 必须在 tts 与 img_gen 之前
        assert!(
            order.iter().position(|n| n == "script_gen").unwrap()
                < order.iter().position(|n| n == "tts").unwrap()
        );
        assert!(
            order.iter().position(|n| n == "script_gen").unwrap()
                < order.iter().position(|n| n == "img_gen").unwrap()
        );
        // i2v 必须在 compose 之前
        assert!(i2v < order.iter().position(|n| n == "compose").unwrap());
    }
}
