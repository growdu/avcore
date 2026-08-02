//! 根命令：init / version / doctor / config

use crate::config::Config;
use crate::db::Db;
use crate::AvcError;
use crate::AvcResult;

pub fn cmd_init() -> AvcResult<()> {
    let db_path = Config::default_db_path()?;
    let cfg_path = Config::default_config_path()?;

    if db_path.exists() {
        return Err(AvcError::Conflict(format!(
            "数据库已存在: {}，先备份再操作",
            db_path.display()
        )));
    }

    let _db = Db::open(&db_path)?;
    println!("✓ 已初始化数据库: {}", db_path.display());

    if !cfg_path.exists() {
        let cfg = Config::default();
        cfg.save(&cfg_path)?;
        println!("✓ 已生成默认配置: {}", cfg_path.display());
    } else {
        println!("• 配置已存在: {}", cfg_path.display());
    }

    Ok(())
}

pub fn cmd_doctor() -> AvcResult<()> {
    let db_path = Config::default_db_path()?;
    let cfg_path = Config::default_config_path()?;

    let mut ok = true;
    if !db_path.exists() {
        println!("✗ 数据库缺失: {}", db_path.display());
        ok = false;
    } else {
        println!("✓ 数据库: {}", db_path.display());
    }

    if !cfg_path.exists() {
        println!("✗ 配置缺失: {}", cfg_path.display());
        ok = false;
    } else {
        println!("✓ 配置: {}", cfg_path.display());
    }

    if ok {
        println!(
            "
doc: avc init  # 若以上缺失"
        );
    }
    Ok(())
}

pub fn cmd_config(argv: &[String]) -> AvcResult<()> {
    if argv.is_empty() {
        return Err(AvcError::Arg("config get|set <key> <val>".into()));
    }
    let path = Config::default_config_path()?;
    let mut cfg = Config::load(&path)?;

    match argv[0].as_str() {
        "get" => {
            if argv.len() < 2 {
                return Err(AvcError::Arg("config get <key>".into()));
            }
            let key = &argv[1];
            match cfg.get_path(key)? {
                Some(v) => println!("{} = {}", key, v),
                None => println!("(unset) {}", key),
            }
        }
        "set" => {
            if argv.len() < 3 {
                return Err(AvcError::Arg("config set <key> <val>".into()));
            }
            // 简化：仅支持 provider.<dim>.<name>.api_key / model
            let key = &argv[1];
            let val = &argv[2];
            apply_set(&mut cfg, key, val)?;
            cfg.save(&path)?;
            println!("✓ set {} = {}", key, val);
        }
        _ => return Err(AvcError::Arg(format!("unknown config verb: {}", argv[0]))),
    }
    Ok(())
}

/// 极简 setter：仅支持 `provider.<dim>.<name>.{api_key|model|endpoint}`
fn apply_set(cfg: &mut Config, key: &str, val: &str) -> AvcResult<()> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.len() != 4 || parts[0] != "provider" {
        return Err(AvcError::Arg(format!("暂不支持此 key: {}", key)));
    }
    let dim = parts[1];
    let name = parts[2];
    let field = parts[3];
    if dim.is_empty() || name.is_empty() || field.is_empty() {
        return Err(AvcError::Arg(format!("暂不支持此 key: {}", key)));
    }

    let map = match dim {
        "avatar" => &mut cfg.provider.avatar,
        "voice" => &mut cfg.provider.voice,
        "llm" => &mut cfg.provider.llm,
        "video" => &mut cfg.provider.video,
        "embed" => &mut cfg.provider.embed,
        _ => return Err(AvcError::Arg(format!("未知维度: {}", dim))),
    };

    let entry = map.entry(name.to_string()).or_default();
    match field {
        "api_key" => entry.api_key = Some(val.to_string()),
        "model" => entry.model = Some(val.to_string()),
        "endpoint" => entry.endpoint = Some(val.to_string()),
        "base_url" => entry.base_url = Some(val.to_string()),
        _ => return Err(AvcError::Arg(format!("未知字段: {}", field))),
    }
    Ok(())
}
