//! `avc corpus <verb>`

use crate::AvcError;
use crate::AvcResult;
use crate::config::Config;
use crate::db::Db;
use crate::output::{print, OutputMode};
use serde_json::json;
use std::path::PathBuf;

pub fn dispatch(argv: &[String]) -> AvcResult<()> {
    let mode = OutputMode::from_flags(
        argv.iter().any(|a| a == "--json"),
        argv.iter().any(|a| a == "--quiet"),
    );

    if argv.is_empty() {
        return Err(AvcError::Arg(
            "corpus create|chunks|search|attach|detach|delete ...".into(),
        ));
    }

    let db = Db::open_default()?;
    let argv_ref: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

    match argv_ref[0] {
        "create" => {
            // corpus create --name <name> --source <path> --embed <name> [--lang <lang>]
            let mut name: Option<&str> = None;
            let mut source: Option<PathBuf> = None;
            let mut embed: Option<&str> = None;
            let mut lang: &str = "zh";
            let mut i = 1;
            while i < argv_ref.len() {
                match argv_ref[i] {
                    "--name" => {
                        name = argv_ref.get(i + 1).copied();
                        i += 2;
                    }
                    "--source" => {
                        source = argv_ref.get(i + 1).map(PathBuf::from);
                        i += 2;
                    }
                    "--embed" => {
                        embed = argv_ref.get(i + 1).copied();
                        i += 2;
                    }
                    "--lang" => {
                        lang = argv_ref.get(i + 1).copied().unwrap_or(lang);
                        i += 2;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            let name = name.ok_or_else(|| AvcError::Arg("--name 必填".into()))?;
            let source = source.ok_or_else(|| AvcError::Arg("--source <path> 必填".into()))?;
            let embed_name = embed.ok_or_else(|| AvcError::Arg("--embed <name> 必填".into()))?;
            let cfg = Config::load(&Config::default_config_path()?)?;
            let id = crate::svc::corpus::create_from_file(
                &db,
                &cfg,
                embed_name,
                name,
                "upload",
                lang,
                &source,
            )?;
            print(
                mode,
                &json!({
                    "corpus_id": id,
                    "name": name,
                    "embed": embed_name,
                    "source": source.display().to_string(),
                    "lang": lang,
                }),
            )?;
        }
        "chunks" => {
            let corpus_id = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("corpus chunks <id>".into()))?;
            let conn = db.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, ordinal, content, embed_dim FROM corpus_chunks
                 WHERE corpus_id = ? AND deprecated = 0
                 ORDER BY ordinal ASC",
            )?;
            let rows = stmt.query_map([corpus_id], |r| {
                Ok(json!({
                    "id": r.get::<_, String>(0)?,
                    "ordinal": r.get::<_, i64>(1)?,
                    "content": r.get::<_, String>(2)?,
                    "embed_dim": r.get::<_, Option<i64>>(3)?,
                }))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            print(mode, &out)?;
        }
        "search" => {
            // corpus search <corpus_id> --query <q> --embed <name> [--topk 5]
            let corpus_id = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("corpus search <id> --query <q>".into()))?;
            let mut query: Option<&str> = None;
            let mut embed: Option<&str> = None;
            let mut topk: usize = 5;
            let mut i = 2;
            while i < argv_ref.len() {
                match argv_ref[i] {
                    "--query" => {
                        query = argv_ref.get(i + 1).copied();
                        i += 2;
                    }
                    "--embed" => {
                        embed = argv_ref.get(i + 1).copied();
                        i += 2;
                    }
                    "--topk" => {
                        topk = argv_ref
                            .get(i + 1)
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(topk);
                        i += 2;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            let query = query.ok_or_else(|| AvcError::Arg("--query 必填".into()))?;
            let embed_name = embed.ok_or_else(|| AvcError::Arg("--embed <name> 必填".into()))?;
            let cfg = Config::load(&Config::default_config_path()?)?;
            let hits = crate::svc::corpus::search(&db, &cfg, embed_name, corpus_id, query, topk)?;
            print(mode, &hits)?;
        }
        "list" => {
            let conn = db.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, source_type, language, chunk_count, created_at
                 FROM knowledge_corpora ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(json!({
                    "id": r.get::<_, String>(0)?,
                    "name": r.get::<_, String>(1)?,
                    "source_type": r.get::<_, String>(2)?,
                    "language": r.get::<_, String>(3)?,
                    "chunk_count": r.get::<_, i64>(4)?,
                    "created_at": r.get::<_, String>(5)?,
                }))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            print(mode, &out)?;
        }
        "attach" => {
            // corpus attach <persona> --version <v> --corpus <id>
            let name = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("corpus attach <persona>".into()))?;
            let mut version: Option<i64> = None;
            let mut corpus_id: Option<&str> = None;
            let mut i = 2;
            while i < argv_ref.len() {
                match argv_ref[i] {
                    "--version" => {
                        version = argv_ref.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    "--corpus" => {
                        corpus_id = argv_ref.get(i + 1).copied();
                        i += 2;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            let v = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            let cid = corpus_id.ok_or_else(|| AvcError::Arg("--corpus 必填".into()))?;
            let p = crate::svc::persona::get_persona(&db, name)?;
            let conn = db.conn.lock().unwrap();
            let binding = serde_json::json!({
                "corpora": [{
                    "corpus_id": cid,
                    "language": "zh",
                    "scope": "persona",
                }]
            });
            let binding_str = binding.to_string();
            let changed = conn.execute(
                "UPDATE persona_versions
                 SET knowledge_binding_json = ?1
                 WHERE persona_model_id = ?2 AND version = ?3
                   AND (knowledge_binding_json IS NULL OR knowledge_binding_json = 'null')",
                rusqlite::params![&binding_str, &p.id, v],
            )?;
            if changed == 0 {
                conn.execute(
                    "UPDATE persona_versions
                     SET knowledge_binding_json = ?1
                     WHERE persona_model_id = ?2 AND version = ?3",
                    rusqlite::params![&binding_str, &p.id, v],
                )?;
            }
            print(
                mode,
                &json!({"persona": name, "version": v, "corpus_id": cid, "attached": true}),
            )?;
        }
        "detach" => {
            let name = argv_ref
                .get(1)
                .copied()
                .ok_or_else(|| AvcError::Arg("corpus detach <persona>".into()))?;
            let mut version: Option<i64> = None;
            let mut i = 2;
            while i < argv_ref.len() {
                match argv_ref[i] {
                    "--version" => {
                        version = argv_ref.get(i + 1).and_then(|s| s.parse().ok());
                        i += 2;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            let v = version.ok_or_else(|| AvcError::Arg("--version 必填".into()))?;
            let p = crate::svc::persona::get_persona(&db, name)?;
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE persona_versions SET knowledge_binding_json = NULL
                 WHERE persona_model_id = ?1 AND version = ?2",
                rusqlite::params![&p.id, v],
            )?;
            print(
                mode,
                &json!({"persona": name, "version": v, "detached": true}),
            )?;
        }
        other => {
            return Err(AvcError::Arg(format!("corpus: unknown verb '{}'", other)));
        }
    }
    Ok(())
}
