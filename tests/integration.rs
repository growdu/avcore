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
fn finetune_rejects_missing_base_version() {
    // Task 2 / Step 1: persona 只有 v1，从不存在的 v99 分叉应被拒绝。
    // 期望：exit 3 (NotFound)；DB 中无 v100、无 finetune_jobs。
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();
    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"]).output().unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "start", "yu", "--base-version", "99"])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(3),
        "missing base version 应 exit 3 (NotFound); stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    // DB 中 v100 应不存在
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let v100_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ? AND pv.version = 100",
        ["yu"], |r| r.get(0)).unwrap();
    assert_eq!(v100_count, 0, "v100 不应被创建");

    // finetune_jobs 应为空
    let job_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM finetune_jobs fj
         JOIN persona_models pm ON pm.id = fj.persona_model_id
         WHERE pm.name = ?",
        ["yu"], |r| r.get(0)).unwrap();
    assert_eq!(job_count, 0, "finetune_jobs 不应有任何条目");
}

#[test]
fn finetune_rejects_non_ready_base_version() {
    // Task 2 / Step 1: 先 start base 1 创建 building v2（不 publish），
    // 再以 base 2 start，应被拒绝（exit 4 Conflict），且无 v3。
    // finetune_jobs 只保留第一次那条 1 条。
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();
    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"]).output().unwrap();

    // 第一次 start base 1：应成功，v2 进入 building 状态（不 publish）。
    let r1 = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "start", "yu", "--base-version", "1"])
        .output()
        .unwrap();
    assert!(
        r1.status.success(),
        "第一次 start base 1 应成功；stderr={}",
        String::from_utf8_lossy(&r1.stderr)
    );

    // 第二次 start base 2：v2 处于 building，应被拒绝（exit 4 Conflict）。
    let r2 = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "start", "yu", "--base-version", "2"])
        .output()
        .unwrap();
    assert_eq!(
        r2.status.code(),
        Some(4),
        "non-ready base version 应 exit 4 (Conflict); stderr={}",
        String::from_utf8_lossy(&r2.stderr)
    );

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();

    // v3 不应被创建
    let v3_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ? AND pv.version = 3",
        ["yu"], |r| r.get(0)).unwrap();
    assert_eq!(v3_count, 0, "v3 不应被创建");

    // finetune_jobs 应只有第一次的 1 条
    let job_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM finetune_jobs fj
         JOIN persona_models pm ON pm.id = fj.persona_model_id
         WHERE pm.name = ?",
        ["yu"], |r| r.get(0)).unwrap();
    assert_eq!(job_count, 1, "finetune_jobs 应只有 1 条");
}

#[test]
fn finetune_rejects_duplicate_target_version() {
    // Task 1 / Step 1: 同一 persona + 同一 base_version 重复 finetune start，
    // 第二次必须 exit 4 (Conflict)；DB 中 persona_versions 恰好 1 行 v2、
    // finetune_jobs 恰好 1 行（v2 行 + job 行原子，拒绝时必须回滚）。
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();
    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"]).output().unwrap();

    // 第一次 start base 1：应成功，创建 v2 building + 1 条 finetune_jobs。
    let r1 = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "start", "yu", "--base-version", "1"])
        .output()
        .unwrap();
    assert!(
        r1.status.success(),
        "第一次 finetune start 应成功；stderr={}",
        String::from_utf8_lossy(&r1.stderr)
    );

    // 第二次同一命令：应被拒绝 (exit 4 Conflict)。
    let r2 = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "start", "yu", "--base-version", "1"])
        .output()
        .unwrap();
    assert_eq!(
        r2.status.code(),
        Some(4),
        "重复 target version 应 exit 4 (Conflict); stderr={}",
        String::from_utf8_lossy(&r2.stderr)
    );
    let stderr2 = String::from_utf8_lossy(&r2.stderr);
    // 错误信息须可定位：含 persona 名、target version 字样、冲突关键词之一
    assert!(
        stderr2.contains("yu") && stderr2.contains("2") && (
            stderr2.contains("target") || stderr2.contains("conflict")
                || stderr2.contains("冲突") || stderr2.contains("存在")
        ),
        "stderr 应包含 persona 名 'yu' + target version '2' + 冲突定位关键词，实际: {:?}",
        stderr2
    );

    // DB 断言：persona_versions 中 yu 的 v2 恰好 1 行；finetune_jobs 恰好 1 行。
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let v2_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ? AND pv.version = 2",
        ["yu"], |r| r.get(0)).unwrap();
    assert_eq!(v2_count, 1, "persona_versions v2 应恰好 1 行");

    let job_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM finetune_jobs fj
         JOIN persona_models pm ON pm.id = fj.persona_model_id
         WHERE pm.name = ?",
        ["yu"], |r| r.get(0)).unwrap();
    assert_eq!(job_count, 1, "finetune_jobs 应恰好 1 行（重复请求应被回滚）");
}

#[test]
fn finetune_concurrent_starts_are_conflicts() {
    // TDD: 跨进程并发 finetune start 必须有且仅有一个成功，
    // 其余全部 exit 4 (Conflict)；绝不能出现 exit 20 (Db / SQLITE_BUSY)。
    // 复现路径：N 个独立 CLI 同时 start，Deferred 事务升级写锁时会随机触发 BUSY。
    // 修复后：所有非胜者都应被 target-version 已存在拒绝。
    //
    // 设计：3 轮 × 8 进程并发，使用独立临时 XDG，确保在原 deferred 实现下
    // 至少 1 轮触发 exit20（独立探针 10/10 中 9 次命中 1 轮），从而 RED 稳定；
    // 修复后 3 轮全部 GREEN。
    use std::sync::{Arc, Barrier};
    use std::thread;

    const N: usize = 8;
    const ROUNDS: usize = 3;

    for round_idx in 0..ROUNDS {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let config = dir.path().join("config");

        bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();
        bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config)
            .args(["persona", "create", "--name", "yu"]).output().unwrap();

        let barrier = Arc::new(Barrier::new(N));
        let mut handles = Vec::with_capacity(N);
        for _ in 0..N {
            let data = data.clone();
            let config = config.clone();
            let b = barrier.clone();
            handles.push(thread::spawn(move || {
                // 等齐后同时发起，最大化抢锁竞争
                b.wait();
                bin()
                    .env("XDG_DATA_HOME", &data)
                    .env("XDG_CONFIG_HOME", &config)
                    .args(["finetune", "start", "yu", "--base-version", "1"])
                    .output()
                    .unwrap()
            }));
        }

        let mut ok = 0usize;
        let mut conflicts = 0usize;
        let mut busy = 0usize; // exit 20 = Db(SQLITE_BUSY) — 严禁出现
        let mut other = 0usize;
        let mut busy_stderr = String::new();
        for h in handles {
            let out = h.join().unwrap();
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            match out.status.code() {
                Some(0) => ok += 1,
                Some(4) => conflicts += 1,
                Some(20) => {
                    busy += 1;
                    if busy_stderr.len() < 512 {
                        busy_stderr.push_str(&stderr);
                    }
                }
                _ => {
                    other += 1;
                    if busy_stderr.len() < 512 {
                        busy_stderr.push_str(&format!("code={:?} ", out.status.code()));
                        busy_stderr.push_str(&stderr);
                    }
                }
            }
        }

        assert_eq!(
            ok, 1,
            "round {round_idx}: 恰好 1 个成功；实测 ok={ok} conflicts={conflicts} busy={busy} other={other}\n\
             busy_stderr={busy_stderr:?}"
        );
        assert_eq!(
            conflicts,
            N - 1,
            "round {round_idx}: 其余 {n} 个应全 exit 4 (Conflict)；实测 ok={ok} conflicts={conflicts} busy={busy} other={other}",
            n = N - 1
        );
        assert_eq!(
            busy, 0,
            "round {round_idx}: 严禁出现 exit 20 (SQLITE_BUSY)；实测 busy={busy}\n\
             busy_stderr={busy_stderr:?}"
        );
        assert_eq!(other, 0, "round {round_idx}: 不应出现其它 exit 码；other={other}");

        // 终态：v2 恰好 1 行、finetune_jobs 恰好 1 行（被拒绝的全部回滚）。
        let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
        let v2_count: i64 = db.query_row(
            "SELECT COUNT(*) FROM persona_versions pv
             JOIN persona_models pm ON pm.id = pv.persona_model_id
             WHERE pm.name = ? AND pv.version = 2",
            ["yu"], |r| r.get(0)).unwrap();
        assert_eq!(v2_count, 1, "round {round_idx}: persona_versions v2 应恰好 1 行");

        let job_count: i64 = db.query_row(
            "SELECT COUNT(*) FROM finetune_jobs fj
             JOIN persona_models pm ON pm.id = fj.persona_model_id
             WHERE pm.name = ?",
            ["yu"], |r| r.get(0)).unwrap();
        assert_eq!(job_count, 1, "round {round_idx}: finetune_jobs 应恰好 1 行");
    }
}

#[test]
fn config_set_get_round_trip() {
    // 临时 XDG init，再 set provider.llm.openai.api_key sk-test，再 get
    // 断言 exit 0、stdout 含完整 key 与 sk-test、且不含 `(unset)`
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    assert!(r.status.success(), "init: {}", String::from_utf8_lossy(&r.stderr));

    let key = "provider.llm.openai.api_key";
    let val = "sk-test";

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["config", "set", key, val])
        .output()
        .unwrap();
    assert!(r.status.success(), "set: {}", String::from_utf8_lossy(&r.stderr));

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["config", "get", key])
        .output()
        .unwrap();
    assert!(r.status.success(), "get should exit 0: {}", String::from_utf8_lossy(&r.stderr));
    let stdout = String::from_utf8_lossy(&r.stdout);
    assert!(stdout.contains(key), "stdout 应含完整 key，实际: {:?}", stdout);
    assert!(stdout.contains(val), "stdout 应含 sk-test，实际: {:?}", stdout);
    assert!(!stdout.contains("(unset)"), "不应出现 (unset)，实际: {:?}", stdout);
}

#[test]
fn config_rejects_empty_provider_name() {
    // 契约：<name> 段必须非空；空 name 的 key 应作为参数错误被拒绝（exit 2）。
    // get 与 set 对称：均不可接受 `provider.<dim>..<field>` 形式。
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    assert!(r.status.success(), "init: {}", String::from_utf8_lossy(&r.stderr));

    // GET 空 name
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["config", "get", "provider.llm..api_key"])
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(2), "empty-name get 应 exit 2 (Arg)");
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert!(
        stderr.contains("参数") || stderr.contains("不支持"),
        "stderr 应提示参数/不支持 key，实际: {:?}",
        stderr
    );

    // SET 空 name：亦应被拒绝
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["config", "set", "provider.llm..api_key", "sk-test"])
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(2), "empty-name set 应 exit 2 (Arg)");
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert!(
        stderr.contains("参数") || stderr.contains("不支持"),
        "stderr 应提示参数/不支持 key，实际: {:?}",
        stderr
    );
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

#[test]
fn ask_with_real_llm_round_trip() {
    // Phase 1 第一刀：起一个最小 HTTP mock 充当 OpenAI 兼容端点，
    // 验证 ask 真发出请求并回显 reply。
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    // 后台 handler：所有请求都返回固定 OpenAI 形状 JSON。
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 8192];
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let _ = stream.read(&mut buf);
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"hello-from-mock"}}]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();

    // 写 toml：base_url 指本机 mock。
    std::fs::create_dir_all(config.join("avc")).unwrap();
    let toml = format!(
        "[provider.llm.mock]\napi_key = \"sk-test\"\nmodel = \"mock-model\"\nbase_url = \"http://127.0.0.1:{}\"\n",
        port
    );
    std::fs::write(config.join("avc/avc.toml"), toml).unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["ask", "--yes", "ping"])
        .output()
        .unwrap();
    let _ = handle.join();

    assert!(
        r.status.success(),
        "ask 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let stdout = String::from_utf8_lossy(&r.stdout);
    assert!(
        stdout.contains("hello-from-mock"),
        "应回显 LLM reply；stdout={:?}",
        stdout
    );
    assert!(
        stdout.contains("mock"),
        "应包含 provider 名 'mock'；stdout={:?}",
        stdout
    );

    let _ = Arc::new(()); // suppress unused import warning
}

#[test]
fn render_rejects_missing_version() {
    // Task 3 / Step 1: persona 只有 v1，render 指定 version 99 应被拒绝。
    // 期望：exit 3 (NotFound)；jobs 计数为 0（无悬挂 job）。
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();
    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"]).output().unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["render", "run", "--persona", "yu", "--version", "99", "--topic", "demo"])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(3),
        "missing version 应 exit 3 (NotFound); stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    // jobs 表应为空：不得创建悬挂 job
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let job_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM jobs j
         JOIN persona_models pm ON pm.id = j.persona_model_id
         WHERE pm.name = ?",
        ["yu"], |r| r.get(0)).unwrap();
    assert_eq!(job_count, 0, "jobs 不应有任何条目");
}

#[test]
fn render_rejects_non_ready_version() {
    // Task 3 / Step 1: 先 finetune start base 1 创建 building v2（不 publish），
    // 再 render v2；应被拒绝（exit 4 Conflict），且 jobs 为 0。
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config).arg("init").output().unwrap();
    bin().env("XDG_DATA_HOME", &data).env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"]).output().unwrap();

    // 第一次 finetune start base 1：应成功，v2 进入 building 状态（不 publish）。
    let r1 = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "start", "yu", "--base-version", "1"])
        .output()
        .unwrap();
    assert!(
        r1.status.success(),
        "第一次 finetune start base 1 应成功；stderr={}",
        String::from_utf8_lossy(&r1.stderr)
    );

    // render v2：v2 处于 building，应被拒绝（exit 4 Conflict）。
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["render", "run", "--persona", "yu", "--version", "2", "--topic", "demo"])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(4),
        "non-ready version 应 exit 4 (Conflict); stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();

    // jobs 表应为空
    let job_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM jobs j
         JOIN persona_models pm ON pm.id = j.persona_model_id
         WHERE pm.name = ?",
        ["yu"], |r| r.get(0)).unwrap();
    assert_eq!(job_count, 0, "jobs 不应有任何条目");
}
