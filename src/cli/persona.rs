//! `avc persona <verb>` 子命令

use std::path::PathBuf;

use crate::db::Db;
use crate::output::{print, OutputMode};
use crate::svc::persona as svc;
use crate::AvcError;
use crate::AvcResult;
use serde_json::json;

pub fn dispatch(argv: &[String]) -> AvcResult<()> {
    if argv.is_empty() {
        return Err(AvcError::Arg(
            "persona create|list|show|versions|attach-*|set-*|commit|promote|demote|archive|delete|current|inspect|dump|onboard|finetune ..."
                .into(),
        ));
    }
    let json = argv.iter().any(|a| a == "--json");
    let quiet = argv.iter().any(|a| a == "--quiet");
    let mode = OutputMode::from_flags(json, quiet);

    let argv: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    let verb = argv[0];

    let db = Db::open_default()?;

    match verb {
        "list" => {
            let status = argv.iter().skip(1).find(|a| !a.starts_with("--")).copied();
            let ps = svc::list_personas(&db, status)?;
            print(mode, &ps)?;
        }
        "show" => {
            if argv.len() < 2 {
                return Err(AvcError::Arg("persona show <name>".into()));
            }
            let p = svc::get_persona(&db, argv[1])?;
            print(mode, &p)?;
        }
        "versions" => {
            if argv.len() < 2 {
                return Err(AvcError::Arg("persona versions <name>".into()));
            }
            let vs = svc::list_versions(&db, argv[1])?;
            print(mode, &vs)?;
        }
        "current" => {
            // persona current <name> [--set <v>]
            if argv.len() < 2 {
                return Err(AvcError::Arg("persona current <name> [--set <v>]".into()));
            }
            let name = argv[1];
            if let Some(idx) = argv.iter().position(|a| *a == "--set") {
                let v_str = argv
                    .get(idx + 1)
                    .ok_or_else(|| AvcError::Arg("--set 缺值".into()))?;
                let v: i64 = v_str
                    .parse()
                    .map_err(|_| AvcError::Arg(format!("--set 值 '{}' 不是整数", v_str)))?;
                svc::promote(&db, name, v)?;
                print(mode, &json!({ "persona": name, "current_version": v }))?;
            } else {
                let v = svc::current_version(&db, name)?;
                print(mode, &json!({ "persona": name, "current_version": v }))?;
            }
        }
        "create" => {
            let mut name: Option<&str> = None;
            let mut archetype: Option<&str> = None;
            let mut description: Option<&str> = None;
            let mut i = 1;
            while i < argv.len() {
                match argv[i] {
                    "--archetype" => {
                        archetype = argv.get(i + 1).copied();
                        i += 2;
                    }
                    "--description" => {
                        description = argv.get(i + 1).copied();
                        i += 2;
                    }
                    "--name" => {
                        name = argv.get(i + 1).copied();
                        i += 2;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            let name = name.ok_or_else(|| AvcError::Arg("--name <n> 必填".into()))?;
            let p = svc::create(&db, name, archetype, description)?;
            print(mode, &p)?;
        }
        "attach-avatar" => {
            // persona attach-avatar <name> --version <v> --ref <path> [--provider <name>]
            let name = argv.get(1).copied().ok_or_else(|| {
                AvcError::Arg("persona attach-avatar <name> --version <v> --ref <path>".into())
            })?;
            let mut version: Option<i64> = None;
            let mut ref_path: Option<String> = None;
            let mut provider: Option<String> = None;
            let mut i = 2;
            while i < argv.len() {
                match argv[i] {
                    "--version" => {
                        version = argv.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--ref" => {
                        ref_path = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--provider" => {
                        provider = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            let version = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            let ref_path = ref_path.ok_or_else(|| AvcError::Arg("--ref <path> 必填".into()))?;
            svc::attach_avatar(
                &db,
                name,
                version,
                &PathBuf::from(&ref_path),
                provider.as_deref(),
            )?;
            print(
                mode,
                &json!({ "persona": name, "version": version, "ref": ref_path, "kind": "avatar" }),
            )?;
        }
        "attach-voice" => {
            let name = argv.get(1).copied().ok_or_else(|| {
                AvcError::Arg("persona attach-voice <name> --version <v> --ref <path>".into())
            })?;
            let mut version: Option<i64> = None;
            let mut ref_path: Option<String> = None;
            let mut provider: Option<String> = None;
            let mut i = 2;
            while i < argv.len() {
                match argv[i] {
                    "--version" => {
                        version = argv.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--ref" => {
                        ref_path = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--provider" => {
                        provider = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            let version = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            let ref_path = ref_path.ok_or_else(|| AvcError::Arg("--ref <path> 必填".into()))?;
            svc::attach_voice(
                &db,
                name,
                version,
                &PathBuf::from(&ref_path),
                provider.as_deref(),
            )?;
            print(
                mode,
                &json!({ "persona": name, "version": version, "ref": ref_path, "kind": "voice" }),
            )?;
        }
        "attach-persona" => {
            // persona attach-persona <name> --version <v> --descriptor <json>
            let name = argv.get(1).copied().ok_or_else(|| {
                AvcError::Arg(
                    "persona attach-persona <name> --version <v> --descriptor <json>".into(),
                )
            })?;
            let mut version: Option<i64> = None;
            let mut descriptor: Option<String> = None;
            let mut i = 2;
            while i < argv.len() {
                match argv[i] {
                    "--version" => {
                        version = argv.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--descriptor" => {
                        descriptor = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            let version = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            let descriptor =
                descriptor.ok_or_else(|| AvcError::Arg("--descriptor <json> 必填".into()))?;
            let v: serde_json::Value = serde_json::from_str(&descriptor)
                .map_err(|e| AvcError::Arg(format!("--descriptor 不是合法 JSON: {}", e)))?;
            svc::attach_persona(&db, name, version, &v)?;
            print(
                mode,
                &json!({ "persona": name, "version": version, "kind": "persona" }),
            )?;
        }
        "attach-knowledge" => {
            // persona attach-knowledge <name> --version <v> --corpus <id>
            let name = argv.get(1).copied().ok_or_else(|| {
                AvcError::Arg("persona attach-knowledge <name> --version <v> --corpus <id>".into())
            })?;
            let mut version: Option<i64> = None;
            let mut corpus_id: Option<String> = None;
            let mut i = 2;
            while i < argv.len() {
                match argv[i] {
                    "--version" => {
                        version = argv.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--corpus" => {
                        corpus_id = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            let version = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            let corpus_id = corpus_id.ok_or_else(|| AvcError::Arg("--corpus 必填".into()))?;
            svc::attach_knowledge(&db, name, version, &corpus_id)?;
            print(
                mode,
                &json!({ "persona": name, "version": version, "corpus": corpus_id, "kind": "knowledge" }),
            )?;
        }
        "set-traits" => {
            // persona set-traits <name> --version <v> --traits <csv>
            let name = argv.get(1).copied().ok_or_else(|| {
                AvcError::Arg("persona set-traits <name> --version <v> --traits <csv>".into())
            })?;
            let mut version: Option<i64> = None;
            let mut traits_csv: Option<String> = None;
            let mut i = 2;
            while i < argv.len() {
                match argv[i] {
                    "--version" => {
                        version = argv.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--traits" => {
                        traits_csv = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            let version = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            let traits_csv =
                traits_csv.ok_or_else(|| AvcError::Arg("--traits <csv> 必填".into()))?;
            let traits: Vec<String> = traits_csv
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            svc::set_traits(&db, name, version, &traits)?;
            print(
                mode,
                &json!({ "persona": name, "version": version, "traits": traits }),
            )?;
        }
        "set-catchphrase" => {
            // persona set-catchphrase <name> --version <v> [--add <s>]... [--remove <s>]...
            let name = argv.get(1).copied().ok_or_else(|| {
                AvcError::Arg(
                    "persona set-catchphrase <name> --version <v> [--add <s>] [--remove <s>]"
                        .into(),
                )
            })?;
            let mut version: Option<i64> = None;
            let mut add: Vec<String> = Vec::new();
            let mut remove: Vec<String> = Vec::new();
            let mut i = 2;
            while i < argv.len() {
                match argv[i] {
                    "--version" => {
                        version = argv.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--add" => {
                        if let Some(s) = argv.get(i + 1) {
                            add.push(s.to_string());
                        }
                        i += 2;
                    }
                    "--remove" => {
                        if let Some(s) = argv.get(i + 1) {
                            remove.push(s.to_string());
                        }
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            let version = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            svc::set_catchphrase(&db, name, version, &add, &remove)?;
            print(
                mode,
                &json!({ "persona": name, "version": version, "added": add, "removed": remove }),
            )?;
        }
        "set-render" => {
            // persona set-render <name> --version <v> --<key> <value>...（任意 key=value 写到 render_options）
            let name = argv.get(1).copied().ok_or_else(|| {
                AvcError::Arg("persona set-render <name> --version <v> --<key> <value>".into())
            })?;
            let mut version: Option<i64> = None;
            let mut kvs: Vec<(String, String)> = Vec::new();
            let mut i = 2;
            while i < argv.len() {
                if argv[i] == "--version" {
                    version = argv.get(i + 1).and_then(|s| s.parse().ok());
                    i += 2;
                } else if argv[i].starts_with("--") {
                    if let Some(v) = argv.get(i + 1) {
                        kvs.push((argv[i][2..].to_string(), v.to_string()));
                        i += 2;
                    } else {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            let version = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            if kvs.is_empty() {
                return Err(AvcError::Arg("至少传一个 --<key> <value>".into()));
            }
            for (k, v) in &kvs {
                svc::set_render_option(&db, name, version, k, v)?;
            }
            print(
                mode,
                &json!({ "persona": name, "version": version, "render_options": kvs.iter().cloned().collect::<std::collections::BTreeMap<_,_>>() }),
            )?;
        }
        "commit" => {
            // persona commit <name> --version <v>
            let name = argv
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("persona commit <name> --version <v>".into()))?;
            let mut version: Option<i64> = None;
            let mut i = 2;
            while i < argv.len() {
                if argv[i] == "--version" {
                    version = argv.get(i + 1).and_then(|s| s.parse().ok());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let version = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            svc::commit(&db, name, version)?;
            print(
                mode,
                &json!({ "persona": name, "version": version, "status": "ready" }),
            )?;
        }
        "promote" => {
            // persona promote <name> --to <v>
            let name = argv
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("persona promote <name> --to <v>".into()))?;
            let mut to: Option<i64> = None;
            let mut i = 2;
            while i < argv.len() {
                if argv[i] == "--to" {
                    to = argv.get(i + 1).and_then(|s| s.parse().ok());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let to = to.ok_or_else(|| AvcError::Arg("--to 必填".into()))?;
            svc::promote(&db, name, to)?;
            print(mode, &json!({ "persona": name, "current_version": to }))?;
        }
        "demote" => {
            // persona demote <name> --version <v>
            let name = argv
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("persona demote <name> --version <v>".into()))?;
            let mut version: Option<i64> = None;
            let mut i = 2;
            while i < argv.len() {
                if argv[i] == "--version" {
                    version = argv.get(i + 1).and_then(|s| s.parse().ok());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let version = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            svc::demote(&db, name, version)?;
            print(
                mode,
                &json!({ "persona": name, "version": version, "status": "deprecated" }),
            )?;
        }
        "archive" => {
            // persona archive <name>
            if argv.len() < 2 {
                return Err(AvcError::Arg("persona archive <name>".into()));
            }
            svc::archive(&db, argv[1])?;
            print(mode, &json!({ "persona": argv[1], "status": "archived" }))?;
        }
        "delete" => {
            // persona delete <name> --confirm
            if argv.len() < 2 {
                return Err(AvcError::Arg("persona delete <name> --confirm".into()));
            }
            let confirm = argv.contains(&"--confirm");
            svc::delete(&db, argv[1], confirm)?;
            print(mode, &json!({ "persona": argv[1], "deleted": true }))?;
        }
        "inspect" => {
            // persona inspect <name> --version <v>
            let name = argv
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("persona inspect <name> --version <v>".into()))?;
            let mut version: Option<i64> = None;
            let mut i = 2;
            while i < argv.len() {
                if argv[i] == "--version" {
                    version = argv.get(i + 1).and_then(|s| s.parse().ok());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let version = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            let p = svc::get_persona(&db, name)?;
            let v = svc::get_version(&db, name, version)?;
            let merged = json!({
                "persona": p,
                "version": v,
            });
            print(mode, &merged)?;
        }
        "dump" => {
            // persona dump <name> --version <v> --out <dir>
            let name = argv.get(1).copied().ok_or_else(|| {
                AvcError::Arg("persona dump <name> --version <v> --out <dir>".into())
            })?;
            let mut version: Option<i64> = None;
            let mut out_dir: Option<String> = None;
            let mut i = 2;
            while i < argv.len() {
                match argv[i] {
                    "--version" => {
                        version = argv.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--out" => {
                        out_dir = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            let version = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            let out_dir = out_dir.ok_or_else(|| AvcError::Arg("--out <dir> 必填".into()))?;
            svc::dump(&db, name, version, &PathBuf::from(&out_dir))?;
            print(
                mode,
                &json!({ "persona": name, "version": version, "out_dir": out_dir }),
            )?;
        }
        "onboard" => {
            // persona onboard --name <n> --archetype <a> --avatar <p> --voice <p> --descriptor <json> [--corpus <id>] [--description <s>]
            let mut name: Option<&str> = None;
            let mut archetype: Option<String> = None;
            let mut description: Option<String> = None;
            let mut avatar_ref: Option<PathBuf> = None;
            let mut avatar_provider: Option<String> = None;
            let mut voice_ref: Option<PathBuf> = None;
            let mut voice_provider: Option<String> = None;
            let mut descriptor: Option<serde_json::Value> = None;
            let mut corpus_id: Option<String> = None;
            let mut i = 1;
            while i < argv.len() {
                match argv[i] {
                    "--name" => {
                        name = argv.get(i + 1).copied();
                        i += 2;
                    }
                    "--archetype" => {
                        archetype = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--description" => {
                        description = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--avatar" => {
                        avatar_ref = argv.get(i + 1).map(PathBuf::from);
                        i += 2;
                    }
                    "--avatar-provider" => {
                        avatar_provider = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--voice" => {
                        voice_ref = argv.get(i + 1).map(PathBuf::from);
                        i += 2;
                    }
                    "--voice-provider" => {
                        voice_provider = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--descriptor" => {
                        let s = argv
                            .get(i + 1)
                            .ok_or_else(|| AvcError::Arg("--descriptor 缺值".into()))?;
                        descriptor = Some(serde_json::from_str(s).map_err(|e| {
                            AvcError::Arg(format!("--descriptor 不是合法 JSON: {}", e))
                        })?);
                        i += 2;
                    }
                    "--corpus" => {
                        corpus_id = argv.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            let name = name.ok_or_else(|| AvcError::Arg("--name <n> 必填".into()))?;
            let spec = svc::OnboardSpec {
                archetype,
                description,
                avatar_ref,
                avatar_provider,
                voice_ref,
                voice_provider,
                descriptor,
                corpus_id,
            };
            svc::onboard(&db, name, spec)?;
            print(
                mode,
                &json!({ "persona": name, "onboarded": true, "version": 1 }),
            )?;
        }
        _ => return Err(AvcError::Arg(format!("persona: unknown verb '{}'", verb))),
    }
    Ok(())
}
