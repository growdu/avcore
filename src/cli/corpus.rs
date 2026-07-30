//! `avc corpus <verb>`

use crate::AvcError;
use crate::AvcResult;
use crate::db::Db;
use crate::output::{print, OutputMode};
use serde_json::json;

pub fn dispatch(argv: &[String]) -> AvcResult<()> {
    let mode = OutputMode::from_flags(
        argv.iter().any(|a| a == "--json"),
        argv.iter().any(|a| a == "--quiet"),
    );

    if argv.is_empty() {
        return Err(AvcError::Arg("corpus create|search|...".into()));
    }

    let _db = Db::open_default()?;
    match argv[0].as_str() {
        "list" => { print(mode, &json!([]))?; }
        _ => return Err(AvcError::Arg(format!("corpus: unknown verb '{}'", argv[0]))),
    }
    Ok(())
}
