//! `avc provider <verb>`

use crate::config::Config;
use crate::output::{print, OutputMode};
use crate::provider::VoiceProvider as VoiceProviderTrait;
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
        return Err(AvcError::Arg("provider list|show|test|config ...".into()));
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
        _ => {
            return Err(AvcError::Arg(format!(
                "provider: unknown verb '{}'",
                argv[0]
            )))
        }
    }
    Ok(())
}
