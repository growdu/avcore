//! `avc persona <verb>` 子命令

use crate::AvcError;
use crate::AvcResult;
use crate::db::Db;
use crate::output::{print, OutputMode};
use crate::svc::persona as svc;

pub fn dispatch(argv: &[String]) -> AvcResult<()> {
    if argv.is_empty() {
        return Err(AvcError::Arg("persona <verb> ...".into()));
    }
    let json = argv.iter().any(|a| a == "--json");
    let quiet = argv.iter().any(|a| a == "--quiet");
    let mode = OutputMode::from_flags(json, quiet);

    let argv: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    let verb = argv[0];

    let db = Db::open_default()?;

    match verb {
        "list" => {
            let status = argv.iter()
                .skip(1)
                .find(|a| !a.starts_with("--"))
                .copied();
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
        "create" => {
            let mut name: Option<&str> = None;
            let mut archetype: Option<&str> = None;
            let mut description: Option<&str> = None;
            let mut i = 1;
            while i < argv.len() {
                match argv[i] {
                    "--archetype" => { archetype = argv.get(i+1).copied(); i += 2; }
                    "--description" => { description = argv.get(i+1).copied(); i += 2; }
                    "--name" => { name = argv.get(i+1).copied(); i += 2; }
                    _ => { i += 1; }
                }
            }
            let name = name.ok_or_else(|| AvcError::Arg("--name <n> 必填".into()))?;
            let p = svc::create(&db, name, archetype, description)?;
            print(mode, &p)?;
        }
        _ => return Err(AvcError::Arg(format!("persona: unknown verb '{}'", verb))),
    }
    Ok(())
}
