//! `avc sample <verb>`

use crate::db::Db;
use crate::output::{print, OutputMode};
use crate::svc::sample as svc;
use crate::AvcError;
use crate::AvcResult;
use serde_json::json;

pub fn dispatch(argv: &[String]) -> AvcResult<()> {
    let mode = OutputMode::from_flags(
        argv.iter().any(|a| a == "--json"),
        argv.iter().any(|a| a == "--quiet"),
    );

    if argv.is_empty() {
        return Err(AvcError::Arg("sample add|list|show|remove ...".into()));
    }

    let db = Db::open_default()?;
    let argv_ref: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

    match argv_ref[0] {
        "add" => {
            // sample add <persona> --kind <image|audio|behavior_text|feedback>
            //   [--uri <path> | --text <s>] [--consent <path>] [--source <s>]
            let persona = argv_ref.get(1).copied().ok_or_else(|| {
                AvcError::Arg("sample add <persona> --kind <k> [--uri|--text]".into())
            })?;
            let mut kind: Option<&str> = None;
            let mut uri: Option<String> = None;
            let mut text: Option<String> = None;
            let mut source: Option<String> = None;
            let mut consent: Option<String> = None;
            let mut i = 2;
            while i < argv_ref.len() {
                match argv_ref[i] {
                    "--kind" => {
                        kind = argv_ref.get(i + 1).copied();
                        i += 2;
                    }
                    "--uri" => {
                        uri = argv_ref.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--text" => {
                        text = argv_ref.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--source" => {
                        source = argv_ref.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    "--consent" => {
                        consent = argv_ref.get(i + 1).map(|s| s.to_string());
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            let kind = kind.ok_or_else(|| AvcError::Arg("--kind <k> 必填".into()))?;
            let source = source.unwrap_or_else(|| "user".to_string());
            let id = svc::add(
                &db,
                persona,
                kind,
                uri.as_deref().map(std::path::Path::new),
                text.as_deref(),
                &source,
                consent.as_deref().map(std::path::Path::new),
            )?;
            print(
                mode,
                &json!({ "sample_id": id, "persona": persona, "kind": kind }),
            )?;
        }
        "list" => {
            // sample list <persona> [--kind <k>]
            let persona = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("sample list <persona> [--kind <k>]".into()))?;
            let mut kind: Option<&str> = None;
            let mut i = 2;
            while i < argv_ref.len() {
                if argv_ref[i] == "--kind" {
                    kind = argv_ref.get(i + 1).copied();
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let rows = svc::list(&db, persona, kind)?;
            print(mode, &rows)?;
        }
        "show" => {
            let sample_id = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("sample show <sample_id>".into()))?;
            let s = svc::show(&db, sample_id)?;
            print(mode, &s)?;
        }
        "remove" => {
            let sample_id = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("sample remove <sample_id>".into()))?;
            svc::remove(&db, sample_id)?;
            print(mode, &json!({ "sample_id": sample_id, "removed": true }))?;
        }
        _ => {
            return Err(AvcError::Arg(format!(
                "sample: unknown verb '{}'",
                argv_ref[0]
            )))
        }
    }
    Ok(())
}
