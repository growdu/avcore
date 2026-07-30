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
        _ => return Err(AvcError::Arg(format!("provider: unknown verb '{}'", argv[0]))),
    }
    Ok(())
}
