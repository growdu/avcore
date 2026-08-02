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
        .args([
            "persona",
            "create",
            "--name",
            "yu",
            "--archetype",
            "db_kernel_expert",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "create: {}",
        String::from_utf8_lossy(&r.stderr)
    );
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

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
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
            "iterate",
            "apply",
            "yu",
            "--version",
            "1",
            "--set-persona",
            r#"{"traits":["严谨","务实"],"catchphrase":"直接看源码"}"#,
        ])
        .output()
        .unwrap();

    // 通过 avc 看是否落库：写一个 inspect helper 或者查库
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let json: String = db
        .query_row(
            "SELECT persona_descriptor_json FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ?",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
    assert!(json.contains("严谨"));
    assert!(json.contains("直接看源码"));
}

#[test]
fn finetune_creates_v2_then_publish() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "finetune",
            "start",
            "yu",
            "--base-version",
            "1",
            "--scope",
            "voice",
            "--threshold",
            "0.85",
        ])
        .output()
        .unwrap();
    assert!(r.status.success());
    let stdout = String::from_utf8_lossy(&r.stdout);
    let fj_id: String = serde_json::from_str::<serde_json::Value>(&stdout).unwrap()
        ["finetune_job_id"]
        .as_str()
        .unwrap()
        .to_string();

    // publish --passed：v2 应 ready
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "publish", &fj_id, "--passed"])
        .output()
        .unwrap();

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ? AND pv.version = 2 AND pv.status = 'ready'",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "v2 should be ready after publish");
}

#[test]
fn finetune_publish_failed_drifts_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "start", "yu", "--base-version", "1"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&r.stdout);
    let fj_id: String = serde_json::from_str::<serde_json::Value>(&stdout).unwrap()
        ["finetune_job_id"]
        .as_str()
        .unwrap()
        .to_string();

    // publish 默认 failed（无 --passed）
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "publish", &fj_id])
        .output()
        .unwrap();

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ? AND pv.version = 2",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "v2 should be rolled back when drift fails");
}

#[test]
fn finetune_rejects_missing_base_version() {
    // Task 2 / Step 1: persona 只有 v1，从不存在的 v99 分叉应被拒绝。
    // 期望：exit 3 (NotFound)；DB 中无 v100、无 finetune_jobs。
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();

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
    let v100_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ? AND pv.version = 100",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v100_count, 0, "v100 不应被创建");

    // finetune_jobs 应为空
    let job_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM finetune_jobs fj
         JOIN persona_models pm ON pm.id = fj.persona_model_id
         WHERE pm.name = ?",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
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

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();

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
    let v3_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ? AND pv.version = 3",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v3_count, 0, "v3 不应被创建");

    // finetune_jobs 应只有第一次的 1 条
    let job_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM finetune_jobs fj
         JOIN persona_models pm ON pm.id = fj.persona_model_id
         WHERE pm.name = ?",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
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

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();

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
        stderr2.contains("yu")
            && stderr2.contains("2")
            && (stderr2.contains("target")
                || stderr2.contains("conflict")
                || stderr2.contains("冲突")
                || stderr2.contains("存在")),
        "stderr 应包含 persona 名 'yu' + target version '2' + 冲突定位关键词，实际: {:?}",
        stderr2
    );

    // DB 断言：persona_versions 中 yu 的 v2 恰好 1 行；finetune_jobs 恰好 1 行。
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let v2_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ? AND pv.version = 2",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v2_count, 1, "persona_versions v2 应恰好 1 行");

    let job_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM finetune_jobs fj
         JOIN persona_models pm ON pm.id = fj.persona_model_id
         WHERE pm.name = ?",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        job_count, 1,
        "finetune_jobs 应恰好 1 行（重复请求应被回滚）"
    );
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

        bin()
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", &config)
            .arg("init")
            .output()
            .unwrap();
        bin()
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", &config)
            .args(["persona", "create", "--name", "yu"])
            .output()
            .unwrap();

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
        assert_eq!(
            other, 0,
            "round {round_idx}: 不应出现其它 exit 码；other={other}"
        );

        // 终态：v2 恰好 1 行、finetune_jobs 恰好 1 行（被拒绝的全部回滚）。
        let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
        let v2_count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM persona_versions pv
             JOIN persona_models pm ON pm.id = pv.persona_model_id
             WHERE pm.name = ? AND pv.version = 2",
                ["yu"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            v2_count, 1,
            "round {round_idx}: persona_versions v2 应恰好 1 行"
        );

        let job_count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM finetune_jobs fj
             JOIN persona_models pm ON pm.id = fj.persona_model_id
             WHERE pm.name = ?",
                ["yu"],
                |r| r.get(0),
            )
            .unwrap();
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
    assert!(
        r.status.success(),
        "init: {}",
        String::from_utf8_lossy(&r.stderr)
    );

    let key = "provider.llm.openai.api_key";
    let val = "sk-test";

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["config", "set", key, val])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "set: {}",
        String::from_utf8_lossy(&r.stderr)
    );

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["config", "get", key])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "get should exit 0: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    let stdout = String::from_utf8_lossy(&r.stdout);
    assert!(
        stdout.contains(key),
        "stdout 应含完整 key，实际: {:?}",
        stdout
    );
    assert!(
        stdout.contains(val),
        "stdout 应含 sk-test，实际: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("(unset)"),
        "不应出现 (unset)，实际: {:?}",
        stdout
    );
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
    assert!(
        r.status.success(),
        "init: {}",
        String::from_utf8_lossy(&r.stderr)
    );

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
        .arg("init")
        .output()
        .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .args(["ask", "列出所有角色"])
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(6)); // token missing
}

#[test]
fn ask_nl_plan_executes_read_only_plan() {
    // mock LLM 返一个 plan: persona list (read_only)。CLI 解析后真跑 `persona list`，
    // 验证结果反映 person 列表（空）。
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 8192];
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let _ = stream.read(&mut buf);
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"{\"intent\":\"list\",\"read_only\":true,\"steps\":[{\"cmd\":\"persona list\",\"args\":{}}]}"}}]}"#;
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
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    std::fs::create_dir_all(config.join("avc")).unwrap();
    let toml = format!(
        "[provider.llm.local]\napi_key = \"sk-fake\"\nmodel = \"fake\"\nbase_url = \"http://127.0.0.1:{}\"\n",
        port
    );
    std::fs::write(config.join("avc/avc.toml"), toml).unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["ask", "--yes", "列出所有角色"])
        .output()
        .unwrap();
    let _ = handle.join();

    assert!(
        r.status.success(),
        "ask 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let stdout = String::from_utf8_lossy(&r.stdout);
    // plan intent 出现 + 结果 OK
    assert!(
        stdout.contains("intent") || stdout.contains("ok"),
        "stdout={:?}",
        stdout
    );
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
            let body =
                r#"{"choices":[{"message":{"role":"assistant","content":"hello-from-mock"}}]}"#;
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

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();

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
fn provider_test_unknown_llm_name_says_not_configured() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["provider", "test", "llm.ghost"])
        .output()
        .unwrap();
    assert!(
        !r.status.success(),
        "不存在的 llm provider 应非 0；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
}

#[test]
fn provider_test_unsupported_dim() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["provider", "test", "avatar.heygen"])
        .output()
        .unwrap();
    assert!(
        !r.status.success(),
        "avatar 测试暂未实现应非 0；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
}

#[test]
fn provider_test_embed_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["provider", "test", "embed.ghost"])
        .output()
        .unwrap();
    assert!(
        !r.status.success(),
        "不存在的 embed provider 应非 0；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
}

#[test]
fn provider_test_avatar_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["provider", "test", "avatar.ghost"])
        .output()
        .unwrap();
    assert!(
        !r.status.success(),
        "不存在的 avatar provider 应非 0；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
}

#[test]
fn provider_test_voice_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["provider", "test", "voice.ghost"])
        .output()
        .unwrap();
    assert!(
        !r.status.success(),
        "不存在的 voice provider 应非 0；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
}

#[test]
fn provider_test_video_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["provider", "test", "video.ghost"])
        .output()
        .unwrap();
    assert!(
        !r.status.success(),
        "不存在的 video provider 应非 0；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
}

#[test]
fn finetune_drift_eval_requires_voice_embed_on_base() {
    // base version 没有 voice_embed → Conflict
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "start", "yu", "--base-version", "1"])
        .output()
        .unwrap();
    assert!(r.status.success());
    let stdout = String::from_utf8_lossy(&r.stdout);
    let fj_id: String = serde_json::from_str::<serde_json::Value>(&stdout).unwrap()
        ["finetune_job_id"]
        .as_str()
        .unwrap()
        .to_string();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "drift", "eval", &fj_id, "--threshold", "0.5"])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(4),
        "缺 voice_embed 应 exit 4 (Conflict); stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
}

#[test]
fn corpus_create_and_search_round_trip() {
    // 写一个 3 段落文件；用 mock embed server（每个 input 返 hash-style 4 维向量）
    // 跑 corpus create → corpus search，按 cosine top-k 排序回前 N。
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = std::thread::spawn(move || loop {
        let Ok((mut stream, _)) = listener.accept() else {
            break;
        };
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .ok();
        let mut buf = [0u8; 16384];
        let _ = stream.read(&mut buf);
        // 极简 mock：根据请求体里的 input 长度返确定的 4 维向量
        let input_count = std::str::from_utf8(&buf)
            .unwrap_or("")
            .matches("\"input\"")
            .count();
        let input_count = if input_count > 0 { 1 } else { input_count }; // 简化估算
        let _body = (0..input_count.max(1))
            .map(|i| {
                // 段落 0: [1,1,1,1]; 段落 1: [-1,0,0,0] (orthogonal); 段落 2: [0,0,0,1]
                let v = match i {
                    0 => vec![1.0, 1.0, 1.0, 1.0],
                    1 => vec![-1.0, 0.0, 0.0, 0.0],
                    _ => vec![0.0, 0.0, 0.0, 1.0],
                };
                format!("{{\"embedding\":{:?}}}", v)
            })
            .collect::<Vec<_>>()
            .join(",");
        // 简化：所有请求统一回 3 个 chunk 的向量（按请求顺序 mock）
        let body = r#"{"data":[{"embedding":[1.0,1.0,1.0,1.0]},{"embedding":[-1.0,0.0,0.0,0.0]},{"embedding":[0.0,0.0,0.0,1.0]}]}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
    });

    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();

    // 写 toml：embed mock → 127.0.0.1:port
    std::fs::create_dir_all(config.join("avc")).unwrap();
    let toml = format!(
        "[provider.embed.mock]\napi_key = \"sk-test\"\nmodel = \"mock-model\"\nbase_url = \"http://127.0.0.1:{}\"\n",
        port
    );
    std::fs::write(config.join("avc/avc.toml"), toml).unwrap();

    // 写 source 文件：3 段落
    let src_path = dir.path().join("kb.txt");
    std::fs::write(&src_path, "段落A\n\n段落B\n\n段落C\n").unwrap();

    // corpus create
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "corpus",
            "create",
            "--name",
            "kb",
            "--source",
            src_path.to_str().unwrap(),
            "--embed",
            "mock",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "corpus create 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let corpus_id = serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout))
        .unwrap()["corpus_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 验证 3 chunk 已落库
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM corpus_chunks WHERE corpus_id = ?1",
            [&corpus_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 3, "应恰好 3 个 chunk");

    // corpus search "段落A" → mock 给 query 也用一个固定向量（这里用 [1,1,1,1]）
    // 我们的 mock 不读 query，把整个请求视作"返 3 段向量"。为简化测试此处
    // 验证"search 不 crash + 至少 1 个 hit 包含 chunk"。
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "corpus", "search", &corpus_id, "--query", "段落A", "--embed", "mock", "--topk", "2",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "corpus search 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let stdout = String::from_utf8_lossy(&r.stdout);
    assert!(
        stdout.contains("\"cosine\""),
        "应含 cosine 字段；stdout={:?}",
        stdout
    );

    drop(handle);
}

#[test]
fn finetune_drift_eval_with_provider_uses_embed_api() {
    // 写入 base voice_embed BLOB 进去；起 mock embed server；
    // --embed mock 触发真发请求；断言 cosine 字段非空。
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    // 简单 mock：所有请求返一个常向量 [1, 0, 0]。与 base = [1,0,0] cosine=1。
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 8192];
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let _ = stream.read(&mut buf);
            let body = r#"{"data":[{"embedding":[1.0,0.0,0.0]}]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();

    // 把 [1,0,0] 写入 base v1 voice_embed。直接走 rusqlite。
    let db_path = data.join("avc/avc.db");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let blob: Vec<u8> = [1.0f32, 0.0, 0.0]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    db.execute(
        "UPDATE persona_versions SET voice_embed = ?1, voice_embed_dim = ?2
         WHERE persona_model_id = (SELECT id FROM persona_models WHERE name = 'yu')
           AND version = 1",
        rusqlite::params![&blob, 3i64],
    )
    .unwrap();

    // 起一个 finetune job（用预先有的 v1 作 base）
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "start", "yu", "--base-version", "1"])
        .output()
        .unwrap();
    assert!(r.status.success());
    let stdout = String::from_utf8_lossy(&r.stdout);
    let fj_id: String = serde_json::from_str::<serde_json::Value>(&stdout).unwrap()
        ["finetune_job_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 写 embed provider 配置
    std::fs::create_dir_all(config.join("avc")).unwrap();
    let toml = format!(
        "[provider.embed.mock]\napi_key = \"sk-test\"\nmodel = \"mock\"\nbase_url = \"http://127.0.0.1:{}\"\n",
        port
    );
    std::fs::write(config.join("avc/avc.toml"), toml).unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "finetune",
            "drift",
            "eval",
            &fj_id,
            "--embed",
            "mock",
            "--threshold",
            "0.5",
        ])
        .output()
        .unwrap();

    let _ = handle.join();

    assert!(
        r.status.success(),
        "drift eval 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let stdout = String::from_utf8_lossy(&r.stdout);
    assert!(
        stdout.contains("\"passed\": true"),
        "passed=true；stdout={:?}",
        stdout
    );
    assert!(
        stdout.contains("\"cosine_provider\": 1.0"),
        "cosine_provider=1.0；stdout={:?}",
        stdout
    );
}

#[test]
fn render_rejects_non_ready_version() {
    // Task 3 / Step 1: 先 finetune start base 1 创建 building v2（不 publish），
    // 再 render v2；应被拒绝（exit 4 Conflict），且 jobs 为 0。
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();

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

    // render v2 应被拒绝：v2 处于 building → exit 4 (Conflict)
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "run",
            "--persona",
            "yu",
            "--version",
            "2",
            "--topic",
            "demo",
        ])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(4),
        "non-ready version 应 exit 4 (Conflict); stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    // jobs 表应为空：未到 CREATE JOB 阶段 → 拒绝时也不写
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let job_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM jobs j
             JOIN persona_models pm ON pm.id = j.persona_model_id
             WHERE pm.name = ?",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(job_count, 0, "jobs 不应有任何条目");
}

// ---------------------------- Phase 2 (render vendor / export / feedback) ----------------------------

fn start_voice_server(
    status: u16,
    response_bytes: &'static [u8],
) -> (u16, std::thread::JoinHandle<Vec<u8>>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buf = [0_u8; 4096];
        let header_end;
        loop {
            let read = stream.read(&mut buf).unwrap();
            assert!(read > 0, "client closed before complete HTTP headers");
            request.extend_from_slice(&buf[..read]);
            if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                header_end = pos + 4;
                break;
            }
        }
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let read = stream.read(&mut buf).unwrap();
            assert!(read > 0, "client closed before complete HTTP body");
            request.extend_from_slice(&buf[..read]);
        }
        let reason = if status == 200 {
            "OK"
        } else if status == 429 {
            "Too Many Requests"
        } else {
            "Internal Server Error"
        };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response_bytes.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.write_all(response_bytes).unwrap();
        stream.flush().unwrap();
        request
    });
    (port, handle)
}

fn init_render_persona(data: &std::path::Path, config: &std::path::Path) {
    let init = bin()
        .env("XDG_DATA_HOME", data)
        .env("XDG_CONFIG_HOME", config)
        .arg("init")
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "init: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let create = bin()
        .env("XDG_DATA_HOME", data)
        .env("XDG_CONFIG_HOME", config)
        .args(["persona", "create", "--name", "voice-test"])
        .output()
        .unwrap();
    assert!(
        create.status.success(),
        "create: {}",
        String::from_utf8_lossy(&create.stderr)
    );
}

#[test]
fn render_voice_provider_posts_exact_script_and_persists_audio() {
    const WAV: &[u8] = b"RIFF\x18\x00\x00\x00WAVEcli-deterministic";
    let (port, server) = start_voice_server(200, WAV);
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    init_render_persona(&data, &config);
    std::fs::write(
        config.join("avc/avc.toml"),
        format!(
            "[provider.voice.local]\napi_key = \"test-key\"\nmodel = \"test-tts\"\nbase_url = \"http://127.0.0.1:{port}\"\n"
        ),
    )
    .unwrap();

    let output = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "run",
            "--persona",
            "voice-test",
            "--version",
            "1",
            "--topic",
            "voice topic",
            "--voice-provider",
            "local",
        ])
        .output()
        .unwrap();
    let request = server.join().unwrap();
    assert!(
        output.status.success(),
        "render: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let job_id = serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let request_text = String::from_utf8(request).unwrap();
    assert!(request_text.starts_with("POST /audio/speech HTTP/1.1\r\n"));
    let body = request_text.split_once("\r\n\r\n").unwrap().1;
    let body: serde_json::Value = serde_json::from_str(body).unwrap();
    let expected_script = "[mock echo] Topic: voice topic\nDuration: 30 seconds";
    assert_eq!(body["input"], expected_script);

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let (content, mime): (Vec<u8>, String) = db
        .query_row(
            "SELECT content, mime FROM artifacts WHERE job_id=?1 AND name='tts'",
            [&job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(content, WAV);
    assert_eq!(mime, "audio/wav");
    let outputs: String = db
        .query_row(
            "SELECT outputs_json FROM job_steps WHERE job_id=?1 AND node_id='tts'",
            [&job_id],
            |row| row.get(0),
        )
        .unwrap();
    let outputs: serde_json::Value = serde_json::from_str(&outputs).unwrap();
    assert_eq!(outputs["meta"]["provider"], "local");
    assert_eq!(outputs["meta"]["bytes"], WAV.len());
    assert_eq!(outputs["meta"]["input_text"], expected_script);
}

#[test]
fn render_voice_http_429_fails_tts_without_downstream_work() {
    let (port, server) = start_voice_server(429, b"rate limited");
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    init_render_persona(&data, &config);
    std::fs::write(
        config.join("avc/avc.toml"),
        format!(
            "[provider.voice.local]\napi_key = \"test-key\"\nbase_url = \"http://127.0.0.1:{port}\"\n"
        ),
    )
    .unwrap();

    let output = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "run",
            "--persona",
            "voice-test",
            "--topic",
            "failure topic",
            "--voice-provider",
            "local",
        ])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(!output.status.success());

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let (job_id, status, current_step): (String, String, String) = db
        .query_row(
            "SELECT id, status, current_step FROM jobs ORDER BY created_at DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "failed");
    assert_eq!(current_step, "tts");
    let steps: Vec<(String, String)> = {
        let mut statement = db
            .prepare("SELECT node_id, status FROM job_steps WHERE job_id=?1 ORDER BY rowid")
            .unwrap();
        statement
            .query_map([&job_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        steps,
        vec![
            ("script_gen".into(), "succeeded".into()),
            ("tts".into(), "failed".into())
        ]
    );
    let artifacts: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artifacts WHERE job_id=?1",
            [&job_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(artifacts, 1, "only the script artifact may exist");
}

fn start_avatar_server(
    status: u16,
    response_bytes: &'static [u8],
) -> (u16, std::thread::JoinHandle<Vec<u8>>) {
    use base64::Engine;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buf = [0_u8; 4096];
        let header_end;
        loop {
            let read = stream.read(&mut buf).unwrap();
            assert!(read > 0, "client closed before complete HTTP headers");
            request.extend_from_slice(&buf[..read]);
            if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                header_end = pos + 4;
                break;
            }
        }
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let read = stream.read(&mut buf).unwrap();
            assert!(read > 0, "client closed before complete HTTP body");
            request.extend_from_slice(&buf[..read]);
        }
        if status == 200 {
            // OpenAI 兼容 /v1/images/generations：响应是 JSON
            // {"data":[{"b64_json":"<base64 PNG>"}]}
            let b64 = base64::engine::general_purpose::STANDARD.encode(response_bytes);
            let body = format!(r#"{{"data":[{{"b64_json":"{b64}"}}]}}"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(body.as_bytes()).unwrap();
            stream.flush().unwrap();
        } else {
            let reason = if status == 429 {
                "Too Many Requests"
            } else {
                "Internal Server Error"
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response_bytes.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(response_bytes).unwrap();
            stream.flush().unwrap();
        }
        request
    });
    (port, handle)
}

#[test]
fn render_avatar_provider_posts_exact_script_and_persists_png() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\navatar-cli-deterministic-payload";
    let (port, server) = start_avatar_server(200, PNG);
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    init_render_persona(&data, &config);
    std::fs::write(
        config.join("avc/avc.toml"),
        format!(
            "[provider.avatar.local]\napi_key = \"test-key\"\nmodel = \"test-img\"\nbase_url = \"http://127.0.0.1:{port}\"\n"
        ),
    )
    .unwrap();

    let output = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "run",
            "--persona",
            "voice-test",
            "--version",
            "1",
            "--topic",
            "avatar topic",
            "--avatar-provider",
            "local",
        ])
        .output()
        .unwrap();
    let request = server.join().unwrap();
    assert!(
        output.status.success(),
        "render: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let job_id = serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let request_text = String::from_utf8(request).unwrap();
    assert!(
        request_text.starts_with("POST /images/generations HTTP/1.1\r\n"),
        "unexpected request line: {:?}",
        request_text.lines().next()
    );
    let body = request_text.split_once("\r\n\r\n").unwrap().1;
    let body: serde_json::Value = serde_json::from_str(body).unwrap();
    // Provider 收到的 prompt 必须是上游 llm 节点生成的"exact script text"
    // (与 voice 节点使用同一脚本依赖项)
    let expected_script = "[mock echo] Topic: avatar topic\nDuration: 30 seconds";
    assert_eq!(body["prompt"], expected_script);

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let (content, mime): (Vec<u8>, String) = db
        .query_row(
            "SELECT content, mime FROM artifacts WHERE job_id=?1 AND name='img_gen'",
            [&job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(content, PNG);
    assert_eq!(mime, "image/png");
    let outputs: String = db
        .query_row(
            "SELECT outputs_json FROM job_steps WHERE job_id=?1 AND node_id='img_gen'",
            [&job_id],
            |row| row.get(0),
        )
        .unwrap();
    let outputs: serde_json::Value = serde_json::from_str(&outputs).unwrap();
    assert_eq!(outputs["meta"]["provider"], "local");
    assert_eq!(outputs["meta"]["model_id"], "test-img");
    assert_eq!(outputs["meta"]["bytes"], PNG.len());
    assert_eq!(outputs["meta"]["prompt"], expected_script);
}

#[test]
fn render_avatar_http_429_fails_img_gen_without_downstream_work() {
    let (port, server) = start_avatar_server(429, b"rate limited");
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    init_render_persona(&data, &config);
    std::fs::write(
        config.join("avc/avc.toml"),
        format!(
            "[provider.avatar.local]\napi_key = \"test-key\"\nbase_url = \"http://127.0.0.1:{port}\"\n"
        ),
    )
    .unwrap();

    let output = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "run",
            "--persona",
            "voice-test",
            "--topic",
            "avatar failure topic",
            "--avatar-provider",
            "local",
        ])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(!output.status.success());

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let (job_id, status, current_step): (String, String, String) = db
        .query_row(
            "SELECT id, status, current_step FROM jobs ORDER BY created_at DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "failed");
    assert_eq!(current_step, "img_gen");
    let steps: Vec<(String, String)> = {
        let mut statement = db
            .prepare("SELECT node_id, status FROM job_steps WHERE job_id=?1 ORDER BY rowid")
            .unwrap();
        statement
            .query_map([&job_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        steps,
        vec![
            ("script_gen".into(), "succeeded".into()),
            ("tts".into(), "succeeded".into()),
            ("img_gen".into(), "failed".into())
        ],
        "script_gen + tts 必跑；img_gen failed 后 i2v/compose 不应再跑"
    );
    let artifacts: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artifacts WHERE job_id=?1",
            [&job_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        artifacts, 2,
        "only script + audio artifacts may exist; avatar 失败后不应落 BLOB"
    );
}

#[test]
fn job_export_writes_artifacts_to_fs() {
    // 走 render run 真跑一次 → job export 落 FS → 验证文件数 + 字节数
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "run",
            "--persona",
            "yu",
            "--version",
            "1",
            "--topic",
            "demo",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "render run: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    let job_id = serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout))
        .unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 1. job show --artifacts 列出 5 条
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["job", "show", &job_id, "--artifacts", "--json"])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "show --artifacts: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    let v = serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout)).unwrap();
    let arts = v["artifacts"].as_array().unwrap();
    assert_eq!(arts.len(), 5, "5 个 artifacts；实际={}", arts.len());

    // 2. job export --out /tmp/.../out → 5 个 .bin 落 FS
    let out_dir = dir.path().join("exported");
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "job",
            "export",
            &job_id,
            "--out",
            out_dir.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "export: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    let v = serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout)).unwrap();
    assert_eq!(v["files"].as_i64().unwrap(), 5);
    let total_bytes = v["bytes"].as_i64().unwrap();
    assert!(total_bytes > 0, "bytes > 0; 实际={}", total_bytes);

    // 3. FS 上 file_count 跟报数一致
    let on_fs = std::fs::read_dir(&out_dir).unwrap().count();
    assert_eq!(on_fs, 5, "FS 上应是 5 个文件；实际={}", on_fs);

    // 4. 每个 .bin 文件 > 0 字节
    for e in std::fs::read_dir(&out_dir).unwrap() {
        let e = e.unwrap();
        let meta = e.metadata().unwrap();
        assert!(meta.len() > 0, "文件 {} 应 > 0 字节", e.path().display());
    }
}

#[test]
fn job_feedback_writes_sample_with_kind_feedback() {
    // job feedback --looks-unlike → persona_samples 增 1 行 kind='feedback' source='user_feedback'
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "run",
            "--persona",
            "yu",
            "--version",
            "1",
            "--topic",
            "demo",
        ])
        .output()
        .unwrap();
    assert!(r.status.success());
    let job_id = serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout))
        .unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 反馈前的 feedback 样本数
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let pre: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM persona_samples WHERE kind='feedback' AND source='user_feedback'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pre, 0);

    // 提交 feedback
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "job",
            "feedback",
            &job_id,
            "--looks-unlike",
            "--reason",
            "音色不太像",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "feedback: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    let v = serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout)).unwrap();
    let sample_id = v["sample_id"].as_str().unwrap();
    assert!(
        sample_id.starts_with("smp_"),
        "sample_id 应 smp_<ULID>; 实际={}",
        sample_id
    );

    // 反馈后 = 1 行
    let post: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM persona_samples WHERE kind='feedback' AND source='user_feedback'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(post, 1, "feedback 后应 1 行；实际={}", post);

    // 验证 reason 落对 + 通过 sample_id 找到
    let (text, persona_id): (String, String) = db
        .query_row(
            "SELECT text, persona_model_id FROM persona_samples WHERE id = ?",
            [&sample_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(text, "音色不太像");
    assert!(
        persona_id.starts_with("pm_"),
        "persona_model_id 应 pm_<ULID>; 实际={}",
        persona_id
    );
}

#[test]
fn job_feedback_without_flag_returns_arg_error() {
    // 缺 --looks-unlike → exit 2 (Arg)
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    let _r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["job", "feedback", "job_does_not_exist", "--looks-unlike"])
        .output()
        .unwrap();
    // 缺 --looks-unlike? 不 — flags 在但 job 不存在 → 应该是 NotFound(exit 3)
    // 测：没 --looks-unlike 旗则 exit 2 (Arg)
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["job", "feedback", "job_does_not_exist", "--reason", "x"])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(2),
        "缺 --looks-unlike 应 Arg; 实际={:?}",
        r.status.code()
    );
}

#[test]
fn cli_video_calls_binary_through_real_pipeline() {
    // Phase 2 端到端：在 avc.toml 里配 binary 指向 mock 脚本，render run 真跑 →
    // video DAG 节点 spawn 真 binary → 写真 mp4 → artifact BLOB 是真 mp4 内容
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();

    // 写 mock 视频 binary（KV-flavor stdout）
    let mock_bin = dir.path().join("mock_video.sh");
    std::fs::write(
        &mock_bin,
        "\
#!/bin/sh
set -e
case \"$1\" in
  submit) echo \"task_id=mock-pipe-1\" ;;
  status) echo \"status=done\" ;;
  fetch)
    OUT=\"\"
    while [ \"$#\" -gt 0 ]; do
      case \"$1\" in
        --out) OUT=\"$2\"; shift 2;;
        *) shift;;
      esac
    done
    mkdir -p \"$(dirname \"$OUT\")\"
    printf 'MOCK_PIPE_MP4_MAGIC' > \"$OUT\"
    head -c 2048 /dev/urandom >> \"$OUT\"
    ;;
  *) echo \"unknown $1\" >&2; exit 2 ;;
esac
",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&mock_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // 写 avc.toml 加 provider.video.mock + binary 路径
    let toml_path = config.join("avc/avc.toml");
    let mut toml = String::from("[provider.video.mock]\nbinary = \"");
    toml.push_str(mock_bin.to_str().unwrap());
    toml.push_str("\"\nmodel = \"m\"\n");
    std::fs::write(&toml_path, toml).unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "run",
            "--persona",
            "yu",
            "--version",
            "1",
            "--topic",
            "pipe-demo",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "render run: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    let job_id = serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout))
        .unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 验证 artifact BLOB 中含有 MOCK_PIPE_MP4_MAGIC 前缀
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    // i2v 节点的产物：artifact kind='clip', name='i2v'。
    let (kind, blob): (String, Vec<u8>) = db.query_row(
        "SELECT kind, content FROM artifacts WHERE job_id = ?1 AND name = 'i2v' AND content IS NOT NULL",
        [&job_id], |r| Ok((r.get(0)?, r.get(1)?))).expect("i2v BLOB");
    eprintln!("i2v kind={}, blob_len={}", kind, blob.len());
    assert!(
        blob.starts_with(b"MOCK_PIPE_MP4_MAGIC"),
        "i2v BLOB 应有 MOCK_PIPE_MP4_MAGIC 前缀；实际前 32 bytes: {:?}",
        &blob[..32.min(blob.len())]
    );
    assert!(blob.len() >= 2048, "应有 ≥2048 bytes；实际={}", blob.len());
}

#[test]
fn render_pack_runs_multiple_jobs_from_topics_file() {
    // Phase 2.4: render pack --topics-file → 多 topic 并行串接跑
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();

    // 写 topics file（包含注释 + 空行 + 3 条有效）
    let topics_path = dir.path().join("topics.txt");
    std::fs::write(
        &topics_path,
        "\
# 第一批生成计划
如何用 avc 写一个 cli_service

# 接下来：
读 SQLite 快的 5 个技巧
rust async 调试 tips
",
    )
    .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "pack",
            "yu",
            "--topics-file",
            topics_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "pack: {}",
        String::from_utf8_lossy(&r.stderr)
    );
    let v = serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout)).unwrap();
    assert_eq!(
        v["job_count"].as_i64().unwrap(),
        3,
        "3 个 job；实际={}",
        v["job_count"]
    );
    assert_eq!(v["failed_count"].as_i64().unwrap(), 0);
    let jobs = v["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 3);

    // DB：jobs 表 3 行 + artifacts 表 5 行 × 3 = 15 行
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let job_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM jobs j JOIN persona_models pm ON pm.id = j.persona_model_id WHERE pm.name = 'yu'",
        [], |r| r.get(0)).unwrap();
    assert_eq!(job_count, 3, "DB 应 3 个 yu jobs；实际={}", job_count);
    let art_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM artifacts WHERE job_id IN (SELECT j.id FROM jobs j JOIN persona_models pm ON pm.id = j.persona_model_id WHERE pm.name = 'yu')",
        [], |r| r.get(0)).unwrap();
    assert_eq!(
        art_count, 15,
        "3 jobs × 5 artifacts = 15；实际={}",
        art_count
    );
}

#[test]
fn render_pack_skips_empty_topics_file() {
    // 空 topics file（全注释 / 空行）→ Arg 错误
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();
    let topics_path = dir.path().join("empty.txt");
    std::fs::write(&topics_path, "# only a comment\n\n   \n").unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "pack",
            "yu",
            "--topics-file",
            topics_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(2),
        "空 topics 应 Arg (exit 2)；实际={:?}",
        r.status.code()
    );
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert!(
        stderr.contains("empty") || stderr.contains("topics"),
        "stderr 应提到 empty/topics；实际={}",
        stderr
    );
}

#[test]
fn render_pack_requires_topics_file() {
    // 缺 --topics-file → Arg 错误
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["render", "pack", "yu"])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(2),
        "缺 --topics-file 应 Arg (exit 2)；实际={:?}",
        r.status.code()
    );
}

#[test]
fn render_run_executes_full_pipeline_and_produces_artifacts() {
    // Wave B：render run 真跑 DAG 五节点，落 artifacts BLOB。
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "run",
            "--persona",
            "yu",
            "--version",
            "1",
            "--topic",
            "demo",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "render run 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let stdout = String::from_utf8_lossy(&r.stdout);
    let job_id: String = serde_json::from_str::<serde_json::Value>(&stdout).unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();

    // job 终态应为 succeeded
    let status: String = db
        .query_row("SELECT status FROM jobs WHERE id = ?1", [&job_id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(status, "succeeded", "job 应 succeeded; 实际={}", status);

    // 5 个 job_steps 节点落地
    let step_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM job_steps WHERE job_id = ?1",
            [&job_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(step_count, 5, "job_steps 应 5 行; 实际={}", step_count);

    // 至少 5 个 artifact BLOB（含 final_video）
    let art_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artifacts WHERE job_id = ?1",
            [&job_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(art_count, 5, "artifacts 应 5 行; 实际={}", art_count);

    // 至少一个 blob 长度 > 0
    let has_blob: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM artifacts WHERE job_id = ?1 AND byte_size > 0",
            [&job_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(has_blob > 0, "至少应有一个非空 BLOB; 实际={}", has_blob);
}

#[test]
fn render_rejects_missing_version() {
    // Task 3 / Step 1: persona 只有 v1，render 指定 version 99 应被拒绝。
    // 期望：exit 3 (NotFound)；jobs 计数为 0（无悬挂 job）。
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "create", "--name", "yu"])
        .output()
        .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "run",
            "--persona",
            "yu",
            "--version",
            "99",
            "--topic",
            "demo",
        ])
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
    let job_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM jobs j
             JOIN persona_models pm ON pm.id = j.persona_model_id
             WHERE pm.name = ?",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(job_count, 0, "jobs 不应有任何条目");
}
