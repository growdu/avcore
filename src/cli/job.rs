//! `avc job <verb>`

use crate::AvcError;
use crate::AvcResult;
use crate::db::Db;
use crate::output::{print, OutputMode};

pub fn dispatch(argv: &[String]) -> AvcResult<()> {
    let mode = OutputMode::from_flags(
        argv.iter().any(|a| a == "--json"),
        argv.iter().any(|a| a == "--quiet"),
    );

    if argv.is_empty() {
        return Err(AvcError::Arg("job list|show|wait|export ...".into()));
    }

    let db = Db::open_default()?;
    match argv[0].as_str() {
        "list" => {
            let name = argv.get(1).ok_or_else(|| AvcError::Arg("job list <persona>".into()))?;
            let ids = crate::svc::render::list_jobs(&db, name)?;
            print(mode, &ids)?;
        }
        "show" => {
            let id = argv.get(1).ok_or_else(|| AvcError::Arg("job show <job_id>".into()))?;
            let status = crate::svc::render::get_job(&db, id)?;
            print(mode, &serde_json::json!({"id": id, "status": status}))?;
        }
        _ => return Err(AvcError::Arg(format!("job: unknown verb '{}'", argv[0]))),
    }
    Ok(())
}
