//! 集成测试：CLI 三入口路由 + 基础 CRUD + refine 数据层

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_avc"))
}

#[test]
fn version_and_help() {
    let out = bin().arg("version").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("avc"));
    assert!(out.status.success());

    let out = bin().arg("--help").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("USAGE"));
}

#[test]
fn init_idempotent_guard() {
    // init 第二次应报错（已存在）
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let r = bin()
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .arg("init")
        .output()
        .unwrap();
    assert!(r.status.success());

    let r = bin()
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .arg("init")
        .output()
        .unwrap();
    assert!(!r.status.success());
    assert_eq!(r.status.code(), Some(4)); // Conflict
}

#[test]
fn persona_lifecycle_json() {
    let dir = tempfile::tempdir().unwrap();
    bin()
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .arg("init")
        .output()
        .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .args(["persona", "create", "--name", "yu", "--archetype", "db_kernel_expert"])
        .output()
        .unwrap();
    assert!(r.status.success(), "create: {}", String::from_utf8_lossy(&r.stderr));
    let stdout = String::from_utf8_lossy(&r.stdout);
    assert!(stdout.contains("db_kernel_expert"));

    let r = bin()
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .args(["persona", "list", "--json"])
        .output()
        .unwrap();
    assert!(r.status.success());
    assert!(String::from_utf8_lossy(&r.stdout).contains("yu"));
}

#[test]
fn refine_changes_persist() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "iterate", "apply", "yu",
            "--version", "1",
            "--set-persona", r#"{"traits":["严谨","务实"],"catchphrase":"直接看源码"}"#,
        ])
        .output()
        .unwrap();

    // 通过 avc 看是否落库：写一个 inspect helper 或者查库
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let json: String = db.query_row(
        "SELECT persona_descriptor_json FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ?",
        ["yu"],
        |r| r.get(0),
    ).unwrap();
    assert!(json.contains("严谨"));
    assert!(json.contains("直接看源码"));
}

#[test]
fn finetune_creates_v2_then_publish() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();
    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).args(["persona","create","--name","yu"]).output().unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config)
        .args(["finetune","start","yu","--base-version","1","--scope","voice","--threshold","0.85"])
        .output().unwrap();
    assert!(r.status.success());
    let stdout = String::from_utf8_lossy(&r.stdout);
    let fj_id: String = serde_json::from_str::<serde_json::Value>(&stdout)
        .unwrap()["finetune_job_id"].as_str().unwrap().to_string();

    // publish --passed：v2 应 ready
    bin()
        .env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config)
        .args(["finetune","publish",&fj_id,"--passed"])
        .output().unwrap();

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ? AND pv.version = 2 AND pv.status = 'ready'",
        ["yu"], |r| r.get(0)).unwrap();
    assert_eq!(count, 1, "v2 should be ready after publish");
}

#[test]
fn finetune_publish_failed_drifts_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();
    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).args(["persona","create","--name","yu"]).output().unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config)
        .args(["finetune","start","yu","--base-version","1"])
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&r.stdout);
    let fj_id: String = serde_json::from_str::<serde_json::Value>(&stdout)
        .unwrap()["finetune_job_id"].as_str().unwrap().to_string();

    // publish 默认 failed（无 --passed）
    bin()
        .env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config)
        .args(["finetune","publish",&fj_id])
        .output().unwrap();

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ? AND pv.version = 2",
        ["yu"], |r| r.get(0)).unwrap();
    assert_eq!(count, 0, "v2 should be rolled back when drift fails");
}

#[test]
fn ask_without_llm_errors() {
    let dir = tempfile::tempdir().unwrap();
    bin()
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .arg("init").output().unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .args(["ask", "列出所有角色"])
        .output().unwrap();
    assert_eq!(r.status.code(), Some(6)); // token missing
}
