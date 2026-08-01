//! `avc provider <verb>`

use crate::AvcError;
use crate::AvcResult;
use crate::config::Config;
use crate::output::{print, OutputMode};
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
            for (k, _) in cfg.provider.avatar.iter() { all.insert(format!("avatar.{}", k), json!({})); }
            for (k, _) in cfg.provider.voice.iter() { all.insert(format!("voice.{}", k), json!({})); }
            for (k, _) in cfg.provider.llm.iter() { all.insert(format!("llm.{}", k), json!({})); }
            for (k, _) in cfg.provider.video.iter() { all.insert(format!("video.{}", k), json!({})); }
            for (k, _) in cfg.provider.embed.iter() { all.insert(format!("embed.{}", k), json!({})); }
            print(mode, &all)?;
        }
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
                other => {
                    return Err(AvcError::Arg(format!(
                        "provider test.{}: not yet implemented (Phase 1+ scope)",
                        other
                    )));
                }
            }
        }
        _ => return Err(AvcError::Arg(format!("provider: unknown verb '{}'", argv[0]))),
    }
    Ok(())
}

