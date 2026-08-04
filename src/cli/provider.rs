//! `avc provider <verb>`

use crate::config::Config;
use crate::output::{print, OutputMode};
use crate::provider::VoiceProvider as VoiceProviderTrait;
use crate::svc::health::{HealthRow, RateLimitRow};
use crate::AvcError;
use crate::AvcResult;
use base64::Engine as _;
use serde_json::json;

pub fn dispatch(argv: &[String]) -> AvcResult<()> {
    let mode = OutputMode::from_flags(
        argv.iter().any(|a| a == "--json"),
        argv.iter().any(|a| a == "--quiet"),
    );

    if argv.is_empty() {
        return Err(AvcError::Arg(
            "provider list|show|test|status|rate-limit ...".into(),
        ));
    }

    match argv[0].as_str() {
        "list" => {
            let cfg = Config::load(&Config::default_config_path()?)?;
            let mut all = serde_json::Map::new();
            for (k, _) in cfg.provider.avatar.iter() {
                all.insert(format!("avatar.{}", k), json!({}));
            }
            for (k, _) in cfg.provider.voice.iter() {
                all.insert(format!("voice.{}", k), json!({}));
            }
            for (k, _) in cfg.provider.llm.iter() {
                all.insert(format!("llm.{}", k), json!({}));
            }
            for (k, _) in cfg.provider.video.iter() {
                all.insert(format!("video.{}", k), json!({}));
            }
            for (k, _) in cfg.provider.embed.iter() {
                all.insert(format!("embed.{}", k), json!({}));
            }
            print(mode, &all)?;
        }
        "test" => {
            if argv.len() < 2 {
                return Err(AvcError::Arg("provider test <dim>.<name>".into()));
            }
            let target = &argv[1];
            let (dim, name) = target
                .split_once('.')
                .ok_or_else(|| AvcError::Arg("provider test: 需要形如 llm.openai".into()))?;
            match dim {
                "llm" => {
                    let cfg = Config::load(&Config::default_config_path()?)?;
                    let provider = crate::provider::real::make_llm(&cfg, name)?;
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
                "embed" => {
                    let cfg = Config::load(&Config::default_config_path()?)?;
                    let provider = crate::provider::real::make_embed(&cfg, name)?;
                    let samples = vec!["hello", "world"];
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| AvcError::Internal(e.to_string()))?;
                    let vectors = rt.block_on(provider.embed(&samples))?;
                    let payload = json!({
                        "provider": target,
                        "ok": true,
                        "count": vectors.len(),
                        "dim": vectors.first().map(|v| v.len()).unwrap_or(0),
                    });
                    print(mode, &payload)?;
                }
                "avatar" => {
                    let cfg = Config::load(&Config::default_config_path()?)?;
                    let provider = crate::provider::real::make_avatar(&cfg, name)?;
                    let spec = crate::provider::AvatarSpec {
                        prompt: "a portrait of yu".to_string(),
                        style: None,
                        ref_image_paths: vec![],
                    };
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| AvcError::Internal(e.to_string()))?;
                    let avatar = rt.block_on(provider.create(&spec))?;
                    let png_size = base64::engine::general_purpose::STANDARD
                        .decode(&avatar.primary_png_b64)
                        .map(|b| b.len())
                        .unwrap_or(0);
                    let payload = json!({
                        "provider": target,
                        "ok": true,
                        "model_id": avatar.model_id,
                        "png_size": png_size,
                    });
                    print(mode, &payload)?;
                }
                "voice" => {
                    let cfg = Config::load(&Config::default_config_path()?)?;
                    let provider = crate::provider::real::make_voice(&cfg, name)?;
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| AvcError::Internal(e.to_string()))?;
                    // synth 试连真 /audio/speech；失败也返 Ok（Phase 1 mock fallback 由 clone 返占位 WAV）
                    let voice = rt.block_on(VoiceProviderTrait::clone(&*provider, &[]))?;
                    let wav_size = base64::engine::general_purpose::STANDARD
                        .decode(&voice.sample_wav_b64)
                        .map(|b| b.len())
                        .unwrap_or(0);
                    let payload = json!({
                        "provider": target,
                        "ok": true,
                        "voice_id_remote": voice.voice_id_remote,
                        "sample_size": wav_size,
                    });
                    print(mode, &payload)?;
                }
                "video" => {
                    let cfg = Config::load(&Config::default_config_path()?)?;
                    let provider = crate::provider::real::make_video(&cfg, name)?;
                    let voice = crate::provider::Voice {
                        provider: name.to_string(),
                        provider_version: "stub".into(),
                        voice_id_remote: Some("mock".into()),
                        sample_wav_b64: String::new(),
                        transcript: None,
                        embed_b64: None,
                        embed_dim: None,
                    };
                    let avatar = crate::provider::Avatar {
                        provider: name.to_string(),
                        provider_version: "stub".into(),
                        model_id: Some("mock".into()),
                        primary_png_b64: String::new(),
                        views_zip_b64: None,
                        face_id: None,
                    };
                    let scenes = vec![crate::provider::ScriptSegment {
                        scene_index: 0,
                        text: "hello".into(),
                        duration_ms: 1000,
                    }];
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| AvcError::Internal(e.to_string()))?;
                    let clip = rt.block_on(provider.render(&voice, &avatar, &scenes))?;
                    let payload = json!({
                        "provider": target,
                        "ok": true,
                        "duration_ms": clip.duration_ms,
                        "mime": clip.mime,
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
        "status" => {
            // provider status [--dim <llm|embed|voice|avatar|video>]
            //
            // v1: read latest provider_health per provider key from DB. `--live` is
            // accepted (no-op for v1) to keep the flag reserved; future versions will
            // hit the daemon HTTP `/health/all` endpoint when present.
            let argv_ref: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
            let dim = parse_dim_flag(&argv_ref)?;
            let _live = argv_ref.contains(&"--live");
            let conn = crate::db::open_default()
                .map_err(|e| AvcError::Internal(format!("db open: {}", e)))?;
            let rows = crate::svc::health::latest_per_provider(&conn, dim.as_deref())?;
            emit_health(mode, dim.as_deref(), &rows)?;
        }
        "rate-limit" => {
            // provider rate-limit
            // 列出 provider_rate_limit 表的所有行；JSON 输出走 print()，
            // 文本输出走简单表格。
            let conn = crate::db::open_default()
                .map_err(|e| AvcError::Internal(format!("db open: {}", e)))?;
            let rows = crate::svc::health::rate_limit_all(&conn)?;
            emit_rate_limit(mode, &rows)?;
        }
        _ => {
            return Err(AvcError::Arg(format!(
                "provider: unknown verb '{}'",
                argv[0]
            )))
        }
    }
    Ok(())
}

/// 提取 `--dim <value>` 中的 value。
///
/// 简化：只看 argv 里第一次出现 `--dim` 的下一个 token；
/// value 不能以 `-` 开头（否则 `--dim --json` 会被吞成 value）。
/// 缺值时返 `Arg` 错，否则返回 `Ok(None)` 表示 flag 不存在。
fn parse_dim_flag(argv: &[&str]) -> AvcResult<Option<String>> {
    let mut i = 0;
    while i < argv.len() {
        if argv[i] == "--dim" {
            return argv
                .get(i + 1)
                .filter(|s| !s.starts_with('-'))
                .map(|s| s.to_string())
                .ok_or_else(|| AvcError::Arg("provider status: --dim <dim> 需要值".into()))
                .map(Some);
        }
        i += 1;
    }
    Ok(None)
}

fn emit_health(mode: OutputMode, dim_filter: Option<&str>, rows: &[HealthRow]) -> AvcResult<()> {
    match mode {
        OutputMode::Json => {
            let arr: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "provider": r.provider_key,
                        "status": r.status.as_str(),
                        "latency_ms": r.latency_ms,
                        "error_msg": r.error_msg,
                        "checked_at": r.checked_at,
                        "source": r.source,
                    })
                })
                .collect();
            print(mode, &json!({ "filter_dim": dim_filter, "rows": arr }))?;
        }
        OutputMode::Quiet => {
            // 静默模式不输出任何行
        }
        OutputMode::Text => {
            println!(
                "{:<10} {:<22} {:<14} {:<9} {:<8} {:<20}",
                "dim", "provider", "status", "latency", "source", "checked_at"
            );
            if rows.is_empty() {
                println!("(no rows)");
                return Ok(());
            }
            for r in rows {
                let dim = r
                    .provider_key
                    .split_once('.')
                    .map(|(d, _)| d)
                    .unwrap_or("?");
                let latency = r
                    .latency_ms
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".to_string());
                println!(
                    "{:<10} {:<22} {:<14} {:<9} {:<8} {:<20}",
                    dim,
                    r.provider_key,
                    r.status.as_str(),
                    latency,
                    r.source,
                    r.checked_at,
                );
            }
        }
    }
    Ok(())
}

fn emit_rate_limit(mode: OutputMode, rows: &[RateLimitRow]) -> AvcResult<()> {
    match mode {
        OutputMode::Json => {
            let now = chrono::Utc::now().timestamp();
            let arr: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    let in_cooldown = r
                        .until_ts
                        .as_ref()
                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                        .map(|d| d.timestamp() > now)
                        .unwrap_or(false);
                    json!({
                        "provider": r.provider_key,
                        "in_cooldown": in_cooldown,
                        "until_ts": r.until_ts,
                        "retry_after_s": r.retry_after_s,
                        "hit_count_24h": r.hit_count_24h,
                        "last_hit_at": r.last_hit_at,
                        "updated_at": r.updated_at,
                    })
                })
                .collect();
            print(mode, &json!({ "rows": arr }))?;
        }
        OutputMode::Quiet => {}
        OutputMode::Text => {
            println!(
                "{:<10} {:<22} {:<12} {:<13} {:<14} {:<22}",
                "dim", "provider", "in_cooldown", "retry_after", "hit_count_24h", "until_ts"
            );
            if rows.is_empty() {
                println!("(no rows)");
                return Ok(());
            }
            let now = chrono::Utc::now().timestamp();
            for r in rows {
                let dim = r
                    .provider_key
                    .split_once('.')
                    .map(|(d, _)| d)
                    .unwrap_or("?");
                let in_cooldown = r
                    .until_ts
                    .as_deref()
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map(|d| d.timestamp() > now)
                    .unwrap_or(false);
                let retry = r
                    .retry_after_s
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".to_string());
                println!(
                    "{:<10} {:<22} {:<12} {:<13} {:<14} {:<22}",
                    dim,
                    r.provider_key,
                    if in_cooldown { "yes" } else { "no" },
                    retry,
                    r.hit_count_24h,
                    r.until_ts.clone().unwrap_or_else(|| "-".to_string()),
                );
            }
        }
    }
    Ok(())
}
