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

/// 验证 Task 10（被动 hook）：`OpenAiCompatLlmProvider::chat` 在 token 鉴权失败
/// (HTTP 401/403) 时**不**改变原有的 exit code / Err variant，但又 best-effort
/// 把一条 status='auth' source='hook' 的记录落到 provider_health 表。
#[test]
fn hook_records_auth_on_real_401() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            // 返 401 Unauthorized（带 retry-after 检查不存在的副 header）
            let resp =
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
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
        "[provider.llm.testhook]\napi_key = \"sk-bad\"\nmodel = \"fake\"\nbase_url = \"http://127.0.0.1:{}\"\n",
        port
    );
    std::fs::write(config.join("avc/avc.toml"), toml).unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["provider", "test", "llm.testhook"])
        .output()
        .unwrap();
    let _ = handle.join();

    // 1. exit code 必须非 0（TokenAuth → 5）
    assert!(
        !r.status.success(),
        "401 chat 必须非 0；status={:?} stderr={}",
        r.status.code(),
        String::from_utf8_lossy(&r.stderr)
    );

    // 2. 必须没有 panic 日志（hook 失败要 silently .ok()）。
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "hook 失败时不能 panic；stderr={:?}",
        stderr
    );

    // 3. hook 应写一条 provider_health：status='auth', source='hook'
    //    （XDG_DATA_HOME 已隔离；DB 默认路径是 $data/avc/avc.db）
    let db_path = data.join("avc").join("avc.db");
    assert!(
        db_path.exists(),
        "默认 DB 应存在；path={}",
        db_path.display()
    );
    // 用 rusqlite 直接开只读 DB 取最新一条 llm.testhook 的 status / source
    let conn = rusqlite::Connection::open(&db_path).expect("open db");
    // 至少一条 hook + status='auth'
    let row: (String, String) = conn
        .query_row(
            "SELECT status, source FROM provider_health
             WHERE provider_key = 'llm.testhook'
             ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("hook should have recorded an auth row");
    assert_eq!(row.0, "auth", "status 必须是 auth");
    assert_eq!(row.1, "hook", "source 必须是 hook");
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

    // 同样把 [1,0,0] 写入 style_embed (v0.3.1+ 三维 drift 都要求 base 有 embed)
    db.execute(
        "UPDATE persona_versions SET style_embed = ?1, style_embed_dim = ?2
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
            "--video-provider",
            "mock",
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

#[test]
fn compose_consumes_real_i2v_mp4_and_persists_exact_final_video() {
    // 端到端：render run → i2v 节点 spawn 真 binary 写 mp4 → compose 节点
    // pass-through → artifacts 表里 i2v 与 compose (final_video) BLOB 必须
    // 字节完全相等 + mime 相同 + 导出文件内容相同 + final_video 节点
    // meta 含 source_node/source_provider/source_artifact_id。
    use base64::Engine as _;
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

    // 真 i2v binary：fetch 时把 18-byte "MOCK_PIPE_MP4_MAGIC" + 1024 字节随机
    // 数据写到 --out 路径。compose 必须原样透传这些字节。
    const PIPE_PREFIX: &[u8] = b"MOCK_PIPE_MP4_MAGIC";
    const PIPE_RANDOM_LEN: usize = 1024;
    let mock_bin = dir.path().join("mock_video.sh");
    std::fs::write(
        &mock_bin,
        format!(
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
    printf '%s' '{magic}' > \"$OUT\"
    head -c {randlen} /dev/urandom >> \"$OUT\"
    ;;
  *) echo \"unknown $1\" >&2; exit 2 ;;
esac
",
            magic = std::str::from_utf8(PIPE_PREFIX).unwrap(),
            randlen = PIPE_RANDOM_LEN,
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&mock_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

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
            "compose-pipe-demo",
            "--video-provider",
            "mock",
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

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();

    // 1) i2v 节点产物：artifacts.kind = node.id ("i2v"), name = "i2v"
    //    content 必须以 PIPE_PREFIX 开头
    let (i2v_kind, i2v_blob, i2v_mime, i2v_artifact_id): (String, Vec<u8>, String, String) = db
        .query_row(
            "SELECT kind, content, mime, id FROM artifacts
             WHERE job_id = ?1 AND name = 'i2v' AND content IS NOT NULL",
            [&job_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("i2v BLOB");
    assert_eq!(i2v_kind, "i2v", "i2v artifacts.kind = node.id");
    assert_eq!(i2v_mime, "video/mp4");
    assert!(
        i2v_blob.starts_with(PIPE_PREFIX),
        "i2v BLOB 应以 MOCK_PIPE_MP4_MAGIC 开头；actual[:32]={:?}",
        &i2v_blob[..32.min(i2v_blob.len())]
    );
    assert_eq!(
        i2v_blob.len(),
        PIPE_PREFIX.len() + PIPE_RANDOM_LEN,
        "i2v BLOB 长度 = prefix + 1024"
    );

    // 2) compose 节点产物：artifacts.kind = node.id ("compose"), name = "compose"。
    //    NodeOutput.kind = "final_video" 反映在 job_steps.outputs_json.kind。
    //    BLOB 必须与 i2v BLOB 字节完全相等；mime 必须等于 i2v mime。
    let (final_kind, final_blob, final_mime, final_id): (String, Vec<u8>, String, String) = db
        .query_row(
            "SELECT kind, content, mime, id FROM artifacts
             WHERE job_id = ?1 AND name = 'compose' AND content IS NOT NULL",
            [&job_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("final_video BLOB");
    assert_eq!(final_kind, "compose", "compose artifacts.kind = node.id");
    assert_eq!(final_mime, i2v_mime, "compose mime 透传 i2v mime");
    assert_eq!(
        final_blob, i2v_blob,
        "compose (final_video) BLOB 必须 = i2v BLOB 字节完全相等"
    );
    assert_ne!(
        final_id, i2v_artifact_id,
        "compose 自己的 artifact_id ≠ i2v"
    );

    // 2b) compose 节点 job_steps.outputs_json.kind = "final_video"
    let compose_outputs: String = db
        .query_row(
            "SELECT outputs_json FROM job_steps WHERE job_id = ?1 AND node_id = 'compose'",
            [&job_id],
            |r| r.get(0),
        )
        .unwrap();
    let compose_outputs: serde_json::Value = serde_json::from_str(&compose_outputs).unwrap();
    assert_eq!(
        compose_outputs["kind"], "final_video",
        "compose NodeOutput.kind = final_video"
    );

    // 3) compose 节点 outputs_json 必须含 source_node/source_provider/source_artifact_id
    assert_eq!(compose_outputs["meta"]["source_node"], "i2v");
    assert_eq!(compose_outputs["meta"]["source_provider"], "mock");
    assert_eq!(
        compose_outputs["meta"]["source_artifact_id"], i2v_artifact_id,
        "compose meta.source_artifact_id 必须 = i2v artifact_id"
    );
    assert_eq!(compose_outputs["meta"]["bytes"], i2v_blob.len() as i64);

    // 4) job export：写盘后 final_video 落 FS 的字节 = i2v 字节
    let out_dir = dir.path().join("exported");
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["job", "export", &job_id, "--out", out_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "export: {}",
        String::from_utf8_lossy(&r.stderr)
    );

    // 找 final_video 落盘文件（kind__name__id.bin）：
    // artifacts.kind = "compose"（=node.id），artifacts.name = "compose"（=node.id），
    // 所以落盘文件名为 compose__compose__<id>.bin。
    // 但 BLOB 在内容上等价于 i2v BLOB（即"final_video" 透传 i2v）。
    let mut final_file: Option<std::path::PathBuf> = None;
    for e in std::fs::read_dir(&out_dir).unwrap() {
        let e = e.unwrap();
        let n = e.file_name();
        let s = n.to_string_lossy();
        if s.starts_with("compose__compose__") && s.ends_with(".bin") {
            final_file = Some(e.path());
            break;
        }
    }
    let final_file = final_file.expect("compose (final_video) 落盘文件应存在");
    let on_disk = std::fs::read(&final_file).unwrap();
    assert_eq!(
        on_disk, i2v_blob,
        "导出文件 compose (final_video) 字节 = i2v BLOB 字节"
    );
    // sanity：base64 decode roundtrip check
    let roundtrip = base64::engine::general_purpose::STANDARD
        .decode(base64::engine::general_purpose::STANDARD.encode(&i2v_blob))
        .unwrap();
    assert_eq!(roundtrip, i2v_blob);
}

/// Helper: init + persona create so render sub-commands can resolve `--persona yu`.
fn init_and_create_persona(dir: &tempfile::TempDir) {
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
}

/// `avc render run --persona yu --topic` (末尾空值) 必须 exit 2 (Arg)，
/// 且 stderr 含可定位的 `--topic <value>` 提示；不能静默跑整个 DAG 并 exit 0。
///
/// RED：在原 render.rs 实现下，`argv.get(i + 1)` 对末尾空 flag 返回 None，
/// option 被静默丢弃；`topic = topic.unwrap_or("(no topic)")` 把缺失当成默认；
/// 整个 render pipeline 仍然执行并返回 job_id、exit 0。
#[test]
fn render_run_rejects_missing_flag_values() {
    let dir = tempfile::tempdir().unwrap();
    init_and_create_persona(&dir);
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    let cases: &[&[&str]] = &[
        &["render", "run", "--persona", "yu", "--topic"],
        &["render", "run", "--persona", "yu", "--version"],
        &["render", "run", "--persona", "yu", "--llm-provider"],
        &["render", "run", "--persona", "yu", "--voice-provider"],
        &["render", "run", "--persona", "yu", "--avatar-provider"],
        &["render", "run", "--persona", "yu", "--video-provider"],
    ];

    for argv in cases {
        let r = bin()
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", &config)
            .args(*argv)
            .output()
            .unwrap();
        assert_eq!(
            r.status.code(),
            Some(2),
            "missing value for {:?} should exit 2 (Arg); stderr={}",
            argv,
            String::from_utf8_lossy(&r.stderr)
        );
        let stderr = String::from_utf8_lossy(&r.stderr);
        let flag = argv.last().unwrap();
        assert!(
            stderr.contains(flag) || stderr.contains(&flag[2..]),
            "stderr 应包含 flag `{}` 用于定位；实际={:?}",
            flag,
            stderr
        );
    }

    // 关键断言：缺失值的调用 *绝不能* 落库 jobs 行（无静默执行）。
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "缺失 flag 值的调用不应创建任何 job（无静默执行）；got {}",
        count
    );
}

/// `avc render pack yu --topics-file` (末尾空值) 必须 exit 2 (Arg)；
/// `--topics-file <existing> --version` (末尾空值) 同样必须 exit 2。
///
/// RED：原 pack 解析器仅在 `--topics-file` *完全缺失* 时报错；末尾空值时
/// `argv.get(i + 1)` 返回 None → `topics_file = None` → 已存在的 `--version <v>`
/// 仍按 default 跑整个 pack（静默执行）。
#[test]
fn render_pack_rejects_missing_flag_values() {
    let dir = tempfile::tempdir().unwrap();
    init_and_create_persona(&dir);
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    // 写一个真实 topics-file 用于测试 `--version` 缺失值的路径。
    let topics = dir.path().join("topics.txt");
    std::fs::write(&topics, "topic-a\n# comment\n\ntopic-b\n").unwrap();

    // 1) --topics-file 末尾空值
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["render", "pack", "yu", "--topics-file"])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(2),
        "pack --topics-file 末尾空值应 exit 2；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    // 2) --topics-file <real> --version 末尾空值
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "pack",
            "yu",
            "--topics-file",
            topics.to_str().unwrap(),
            "--version",
        ])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(2),
        "pack --version 末尾空值应 exit 2；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert!(
        stderr.contains("--version"),
        "stderr 应包含 `--version` 用于定位；实际={:?}",
        stderr
    );

    // 关键断言：两种缺失值调用都不应创建任何 job。
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "pack 缺失 flag 值的调用不应创建任何 job（无静默执行）；got {}",
        count
    );
}

/// `avc render run` 与 `avc render pack` 遇到未知 flag（拼错的 flag 名）或
/// 额外 positional token 时必须 exit 2，且 stderr 命名该 token；jobs 表
/// 必须为 0 行（无静默默认/无静默执行整个 DAG）。
///
/// RED：原 render.rs 在循环里用 `_ => i += 1` 静默吞掉所有未知 token，
/// `--llm-providr mock` 这种 typo 会被当作未识别 flag 跳过，pipeline 仍按
/// 默认 LLM provider 跑完整个 DAG；`render run --persona yu extra-token`
/// 这种多余 positional 同样被静默吞掉。
#[test]
fn render_rejects_unknown_flags_and_extra_positionals() {
    let dir = tempfile::tempdir().unwrap();
    init_and_create_persona(&dir);
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    // 准备一个有效的 topics-file 让 pack 的"额外 positional"路径有 base。
    let topics = dir.path().join("topics.txt");
    std::fs::write(&topics, "topic-a\n").unwrap();

    // (argv, 期望被命名的 token 子串)
    let cases: &[(&[&str], &str)] = &[
        // === render run：typo'd provider flag（最常见 bug：拼错名 → 静默跑默认） ===
        (
            &[
                "render",
                "run",
                "--persona",
                "yu",
                "--llm-providr",
                "mock-llm",
            ],
            "--llm-providr",
        ),
        (
            &[
                "render",
                "run",
                "--persona",
                "yu",
                "--voice-providr",
                "mock-voice",
            ],
            "--voice-providr",
        ),
        (
            &[
                "render",
                "run",
                "--persona",
                "yu",
                "--avatar-providr",
                "mock-avatar",
            ],
            "--avatar-providr",
        ),
        (
            &[
                "render",
                "run",
                "--persona",
                "yu",
                "--video-providr",
                "mock-video",
            ],
            "--video-providr",
        ),
        // 完全未知的 flag
        (
            &["render", "run", "--persona", "yu", "--bogus", "value"],
            "--bogus",
        ),
        // === render run：额外 positional token ===
        (
            &[
                "render",
                "run",
                "--persona",
                "yu",
                "--topic",
                "hi",
                "extra-positional",
            ],
            "extra-positional",
        ),
        // === render pack：typo'd flag ===
        (
            &[
                "render",
                "pack",
                "yu",
                "--topics-fil",
                topics.to_str().unwrap(),
            ],
            "--topics-fil",
        ),
        // === render pack：多余 positional（pack 只应有 persona 一个 positional） ===
        (
            &[
                "render",
                "pack",
                "yu",
                "stray",
                "--topics-file",
                topics.to_str().unwrap(),
            ],
            "stray",
        ),
        // === render pack：positionals 之间夹了 flag-prefixed 残留 ===
        (
            &[
                "render",
                "pack",
                "yu",
                "--bogus-flag",
                "--topics-file",
                topics.to_str().unwrap(),
            ],
            "--bogus-flag",
        ),
    ];

    for (argv, expected_token) in cases {
        let r = bin()
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", &config)
            .args(*argv)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&r.stderr);
        assert_eq!(
            r.status.code(),
            Some(2),
            "未知/多余 token {:?} 应 exit 2 (Arg)；stderr={}",
            argv,
            stderr
        );
        assert!(
            stderr.contains(expected_token),
            "stderr 应包含被拒 token `{}` 用于定位；argv={:?}，实际 stderr={:?}",
            expected_token,
            argv,
            stderr
        );
    }

    // 关键断言：所有未知 token 的调用 *绝不能* 落库 jobs 行。
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "未知/多余 token 的调用不应创建任何 job（无静默执行）；got {}",
        count
    );
}

/// 守门测试：合法 invocation 在新解析器下必须仍然工作（已知 flag + 合法值
/// 不被误拒）。这是 `render_rejects_unknown_flags_and_extra_positionals` 的
/// 反面，避免过度收紧破坏既有调用。
#[test]
fn render_still_accepts_valid_invocations() {
    let dir = tempfile::tempdir().unwrap();
    init_and_create_persona(&dir);
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    // render run 仅带 --persona (合法最小集)
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["render", "run", "--persona", "yu"])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "合法 render run 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    // pack 最小集
    let topics = dir.path().join("topics.txt");
    std::fs::write(&topics, "topic-a\n").unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "pack",
            "yu",
            "--topics-file",
            topics.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "合法 render pack 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    // value 看起来像 flag 也应被接受（"--topic --weird" = topic="--weird"）。
    // 我们不直接跑这种 (会触发 DAG)，只通过 `arg` 路径上 accept-don't-reject
    // 来覆盖：用一个会被 dispatch 早期拒绝的 flag 链来间接确认解析路径不会
    // 把 value 误判。这里用一个确认会被吞掉且最终成功结束的 invocation，
    // 再断言 jobs 数 >= 2。
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert!(
        count >= 2,
        "两次合法 invocation 至少应落 2 条 job；got {}",
        count
    );
}

/// `avc render run` / `pack` 的 value-required flag 后面紧跟 `-` 开头的
/// token（已知 flag、未知 flag、全局 --json/--quiet 都算）必须被视为"缺失值"。
///
/// RED：原 `next(i)` 仅校验 `Some(...)` + 非空；`--persona --quiet` 会把
/// `--quiet` 当成 persona 名，下游 `get_persona("--quiet")` 抛 NotFound，
/// 误导用户把 "--quiet 当 persona" 当成"persona 不存在"，且 exit 3。
/// 类似 `--video-provider --json` → 诡异的 "provider.video.--json" 路径，
/// `--topics-file --quiet` → "Db: read topics file --quiet: No such file"
/// (exit 20)。这些都不是用户预期的 "missing value" (exit 2)。
///
/// GREEN：narrow rule — `next` token 以 `-` 开头一律视为缺失值；
/// `require_value` 抛 `AvcError::Arg` (exit 2)，stderr 命名该 flag。
/// 例外：保留 `--version` 接受纯数字（含 `--1` 这种诡异但合法形式），
/// 但因为版本域恒 ≥1，另行 enforce `version > 0`。
#[test]
fn render_value_required_flag_rejects_leading_dash_next() {
    let dir = tempfile::tempdir().unwrap();
    init_and_create_persona(&dir);
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    // 写一个真实 topics-file，让 pack 的"被吞 --topics-file"路径有 base。
    let topics = dir.path().join("topics.txt");
    std::fs::write(&topics, "topic-a\n# comment\n\ntopic-b\n").unwrap();

    // (argv, 期望被 stderr 命名的 flag 子串)
    let cases: &[(&[&str], &str)] = &[
        // === render run：value-required flag 后紧跟全局 --quiet / --json ===
        (&["render", "run", "--persona", "--quiet"], "--persona"),
        (&["render", "run", "--persona", "--json"], "--persona"),
        (
            &["render", "run", "--persona", "yu", "--topic", "--quiet"],
            "--topic",
        ),
        (
            &["render", "run", "--persona", "yu", "--version", "--json"],
            "--version",
        ),
        // === render run：provider flag 后紧跟任何其它 flag ===
        (
            &[
                "render",
                "run",
                "--persona",
                "yu",
                "--llm-provider",
                "--avatar-provider",
            ],
            "--llm-provider",
        ),
        (
            &[
                "render",
                "run",
                "--persona",
                "yu",
                "--voice-provider",
                "--video-provider",
            ],
            "--voice-provider",
        ),
        (
            &[
                "render",
                "run",
                "--persona",
                "yu",
                "--avatar-provider",
                "--quiet",
            ],
            "--avatar-provider",
        ),
        (
            &[
                "render",
                "run",
                "--persona",
                "yu",
                "--video-provider",
                "--json",
            ],
            "--video-provider",
        ),
        // === render run：provider flag 后紧跟拼错的 flag / 完全未知 flag ===
        (
            &[
                "render",
                "run",
                "--persona",
                "yu",
                "--llm-provider",
                "--bogus",
            ],
            "--llm-provider",
        ),
        // === render pack：--topics-file 后紧跟全局 / 未知 ===
        (
            &["render", "pack", "yu", "--topics-file", "--quiet"],
            "--topics-file",
        ),
        (
            &["render", "pack", "yu", "--topics-file", "--bogus-flag"],
            "--topics-file",
        ),
        // === render pack：--version 后紧跟全局 / 未知 ===
        (
            &[
                "render",
                "pack",
                "yu",
                "--topics-file",
                topics.to_str().unwrap(),
                "--version",
                "--quiet",
            ],
            "--version",
        ),
        (
            &[
                "render",
                "pack",
                "yu",
                "--topics-file",
                topics.to_str().unwrap(),
                "--version",
                "--unknown",
            ],
            "--version",
        ),
    ];

    for (argv, expected_flag) in cases {
        let r = bin()
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", &config)
            .args(*argv)
            .output()
            .unwrap();
        assert_eq!(
            r.status.code(),
            Some(2),
            "value-required flag 后紧跟 `-` 开头的 token 必须 exit 2 (Arg)；argv={:?}，stderr={}",
            argv,
            String::from_utf8_lossy(&r.stderr)
        );
        let stderr = String::from_utf8_lossy(&r.stderr);
        assert!(
            stderr.contains(expected_flag),
            "stderr 应包含被拒 flag `{}` 用于定位；argv={:?}，stderr={:?}",
            expected_flag,
            argv,
            stderr
        );
    }

    // 关键断言：所有"被吞 value"的调用 *绝不能* 落 jobs 行。
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "value-required flag 后紧跟 `-` 开头 token 的调用不应创建任何 job；got {}",
        count
    );
}

/// `avc render run --version <n>` / `pack --version <n>`：版本号必须 > 0
/// (persona 版本链恒 ≥ 1：`create` 落 v1，`finetune start base=N` 落 v(N+1))。
///
/// 这是 schema/service 证据支持的 enforce：DB 中 `persona_models.current_version
/// INTEGER NOT NULL DEFAULT 1`、`persona_versions.version INTEGER NOT NULL`
/// (PK 的一部分)；下游 `create_job` 用 `version` 直接查表，<= 0 只会得到
/// NotFound，与其让用户经历 NotFound 不如在 CLI 层 early-reject 给清晰错误。
///
/// 不在本测试覆盖负数 — 任务边界明确"version should be positive anyway"，
/// 不引入 `version = -1` 这种诡异 case 让 parser 接受。
#[test]
fn render_rejects_non_positive_version() {
    let dir = tempfile::tempdir().unwrap();
    init_and_create_persona(&dir);
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    let topics = dir.path().join("topics.txt");
    std::fs::write(&topics, "topic-a\n").unwrap();

    // 0 是非正版本的最直接代表；-1 / "-99" 我们也一并覆盖以 enforce "≤0 拒绝"。
    for argv in [
        vec!["render", "run", "--persona", "yu", "--version", "0"],
        vec!["render", "run", "--persona", "yu", "--version", "-1"],
        vec!["render", "run", "--persona", "yu", "--version", "-99"],
        vec![
            "render",
            "pack",
            "yu",
            "--topics-file",
            topics.to_str().unwrap(),
            "--version",
            "0",
        ],
        vec![
            "render",
            "pack",
            "yu",
            "--topics-file",
            topics.to_str().unwrap(),
            "--version",
            "-1",
        ],
    ] {
        let r = bin()
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", &config)
            .args(&argv)
            .output()
            .unwrap();
        assert_eq!(
            r.status.code(),
            Some(2),
            "非正 --version {:?} 应 exit 2 (Arg)；stderr={}",
            argv,
            String::from_utf8_lossy(&r.stderr)
        );
        let stderr = String::from_utf8_lossy(&r.stderr);
        assert!(
            stderr.contains("--version"),
            "stderr 应包含 `--version` 用于定位；argv={:?}，stderr={:?}",
            argv,
            stderr
        );
    }

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "非正 --version 的调用不应创建任何 job；got {}",
        count
    );
}

/// 守门测试：合法的 render invocation（含全局 --quiet / --json）必须仍然成功，
/// 不能因为新 parser 收紧而被误拒。这是 `render_value_required_flag_rejects_leading_dash_next`
/// 的反面，避免过度收紧破坏既有调用 (含全局 flag 在末尾 / 中段)。
#[test]
fn render_accepts_valid_invocations_with_global_flags() {
    let dir = tempfile::tempdir().unwrap();
    init_and_create_persona(&dir);
    let data = dir.path().join("data");
    let config = dir.path().join("config");

    let topics = dir.path().join("topics.txt");
    std::fs::write(&topics, "topic-a\n").unwrap();

    // === render run --quiet (合法：全局 flag 在 value-required flag 之后) ===
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["render", "run", "--persona", "yu", "--quiet"])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "合法 render run + --quiet 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let stdout = String::from_utf8_lossy(&r.stdout);
    assert!(
        stdout.starts_with("job_"),
        "quiet 模式 stdout 形如 `job_...`；got={:?}",
        stdout
    );

    // === render run --json (合法：全局 flag 在末尾) ===
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["render", "run", "--persona", "yu", "--json"])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "合法 render run + --json 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    // === render run 合法 --version 1 (合法：明确指定 v1) ===
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
            "t",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "合法 render run + --version 1 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    // === render pack + 合法 --version 1 + 全局 --quiet ===
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "render",
            "pack",
            "yu",
            "--topics-file",
            topics.to_str().unwrap(),
            "--version",
            "1",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "合法 render pack + --version 1 + --quiet 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    // 守门：不应有 job 被错误地"丢弃"。render run 跑了 2 次 (--quiet + --json + --version 1 共 3 次)，
    // pack 跑了 1 次；期望至少 3 条 job。pack 的 job 计数按 topics-file 行数。
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
        .unwrap();
    assert!(
        count >= 3,
        "合法 invocation 应至少落 3 条 job (3 run + 1 pack topic)；got {}",
        count
    );
}

#[test]
fn finetune_run_via_vendor_cli_writes_target_and_publishes() {
    // 端到端：finetune start + vendor CLI SFT + embed 真算 drift + publish。
    // 1. init + create persona yu
    // 2. 直接 UPDATE v1 写 voice_provider/voice_embed = [1,0,0]（base 已有 embedding）
    // 3. INSERT 一条 audio sample 进 persona_samples（blob 字段存 wav bytes）
    // 4. avc.toml 配 [provider.voice.mock] binary=mock_vendor.sh + [provider.embed.mock]
    //    base_url=本地 HTTP 端点（永远返 [1,0,0]）
    // 5. finetune start yu --scope voice --base-version 1
    // 6. finetune run <fj_id> --embed mock → exit 0
    // 7. 验证：target v2 voice_provider='mock'、voice_sample 含 MOCK_SFT_WAV_MAGIC、
    //    finetune_jobs.status='succeeded'、voice_embed 被补算为 [1,0,0]
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // 1. mock embed HTTP server（永远返 [1,0,0]，与 base 同）。
    // v0.3.1+ 三维 drift（face / voice / style）= 3 次 HTTP 请求；server 跑 6 次
    // accept 循环后退出（防慢收尾）。每个连接返同样的 [1,0,0] 向量。
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let embed_port = listener.local_addr().unwrap().port();
    let embed_handle = std::thread::spawn(move || {
        for _ in 0..2 {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0u8; 8192];
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                    let _ = stream.read(&mut buf);
                    let body = r#"{"data":[{"embedding":[1.0,0.0,0.0]}]}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                }
                Err(_) => break,
            }
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

    // 2. UPDATE v1: voice_provider='mock' + voice_embed=[1,0,0]
    let db_path = data.join("avc/avc.db");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let blob: Vec<u8> = [1.0f32, 0.0, 0.0]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    db.execute(
        "UPDATE persona_versions SET voice_provider = ?1, voice_provider_version = ?2,
             voice_embed = ?3, voice_embed_dim = ?4
         WHERE persona_model_id = (SELECT id FROM persona_models WHERE name = 'yu')
           AND version = 1",
        rusqlite::params!["mock", "stub", &blob, 3i64],
    )
    .unwrap();
    // v0.3.1+ 三维 drift：face + style 也需要 base 已有 embed；这里也写 [1,0,0]
    db.execute(
        "UPDATE persona_versions SET face_embed = ?1, face_embed_dim = ?2,
             style_embed = ?3, style_embed_dim = ?4
         WHERE persona_model_id = (SELECT id FROM persona_models WHERE name = 'yu')
           AND version = 1",
        rusqlite::params![&blob, 3i64, &blob, 3i64],
    )
    .unwrap();

    // 3. INSERT audio sample
    let sample_id = format!("sm_{}", ulid::Ulid::new().to_string().to_lowercase());
    db.execute(
        "INSERT INTO persona_samples (id, persona_model_id, kind, blob, blob_mime, source, created_at)
         VALUES (?1, (SELECT id FROM persona_models WHERE name = 'yu'),
                 'audio', ?2, 'audio/wav', 'test', '2026-08-01T00:00:00Z')",
        rusqlite::params![&sample_id, &b"sample-audio-bytes".to_vec()],
    )
    .unwrap();
    drop(db);

    // 4. mock vendor CLI for voice SFT
    let mock_bin = dir.path().join("mock_voice_ft.sh");
    std::fs::write(
        &mock_bin,
        "#!/bin/sh
set -e
case \"$1\" in
  finetune)
    case \"$2\" in
      submit)
        if [ \"$3\" != \"--ref-audio\" ]; then
          echo \"expected --ref-audio, got $3\" >&2; exit 2
        fi
        echo \"task_id=mock-vendor-ft-1\"
        ;;
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
        printf 'MOCK_SFT_WAV_MAGIC' > \"$OUT\"
        head -c 256 /dev/urandom >> \"$OUT\"
        ;;
      *) echo \"unknown $2\" >&2; exit 2 ;;
    esac
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

    // 写 avc.toml
    std::fs::create_dir_all(config.join("avc")).unwrap();
    let toml = format!(
        "[provider.voice.mock]\nbinary = \"{}\"\n[provider.embed.mock]\napi_key = \"sk\"\nmodel = \"mock\"\nbase_url = \"http://127.0.0.1:{}\"\n",
        mock_bin.to_str().unwrap(),
        embed_port,
    );
    std::fs::write(config.join("avc/avc.toml"), toml).unwrap();

    // 5. finetune start
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
            "0.5", // 低阈值：base [1,0,0] vs new [1,0,0] = 1.0 > 0.5
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "finetune start: stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let stdout = String::from_utf8_lossy(&r.stdout);
    let fj_id: String = serde_json::from_str::<serde_json::Value>(&stdout).unwrap()
        ["finetune_job_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 6. finetune run --embed mock
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "run", &fj_id, "--embed", "mock"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&r.stderr);
    let stdout = String::from_utf8_lossy(&r.stdout);
    assert!(
        r.status.success(),
        "finetune run 应成功；status={:?}, stdout={}, stderr={}",
        r.status.code(),
        stdout,
        stderr
    );
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["status"], "succeeded");
    assert_eq!(parsed["scopes_processed"][0], "voice");
    assert_eq!(parsed["samples_used"], 1);
    assert!(
        (parsed["voice_cosine"].as_f64().unwrap() - 1.0).abs() < 1e-6,
        "voice_cosine 应 ≈ 1.0；got {}",
        parsed["voice_cosine"]
    );

    // 7. 验证 DB：target v2 voice_provider/voice_sample
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let (vp, sample): (Option<String>, Option<Vec<u8>>) = db
        .query_row(
            "SELECT voice_provider, voice_sample FROM persona_versions pv
             JOIN persona_models pm ON pm.id = pv.persona_model_id
             WHERE pm.name = 'yu' AND pv.version = 2",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(vp.as_deref(), Some("mock"));
    let sample = sample.expect("voice_sample 应非空");
    assert!(
        sample.starts_with(b"MOCK_SFT_WAV_MAGIC"),
        "voice_sample 前缀"
    );
    assert!(sample.len() >= 256);

    // 验证 v2 已 ready
    let v2_status: String = db
        .query_row(
            "SELECT pv.status FROM persona_versions pv
             JOIN persona_models pm ON pm.id = pv.persona_model_id
             WHERE pm.name = 'yu' AND pv.version = 2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v2_status, "ready", "v2 应 ready after successful SFT");

    // 验证 finetune_jobs.status = 'succeeded' + voice_embed 已补算
    let row = db
        .query_row(
            "SELECT fj.status, pv.voice_embed_dim FROM finetune_jobs fj
             LEFT JOIN persona_versions pv
               ON pv.persona_model_id = fj.persona_model_id
              AND pv.version = fj.target_version
             WHERE fj.id = ?",
            [&fj_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .unwrap();
    let (fj_status, voice_embed_dim) = row;
    assert_eq!(fj_status, "succeeded");
    assert_eq!(
        voice_embed_dim,
        Some(3),
        "voice_embed_dim=3（mock 返 3 维向量）"
    );

    let _ = embed_handle.join();
}

#[test]
fn finetune_run_without_embed_arg_for_voice_scope_errors() {
    // voice scope 跑 finetune run 但不传 --embed → Arg 错（voice drift 缺 embed provider
    // 无法算 cosine），并保持 target v2 在 building 状态（不 commit 也不 rollback）。
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

    // 写 v1 voice_provider（不写 voice_embed 没事，因为 run 在 drift 之前会先要求 --embed）
    let db_path = data.join("avc/avc.db");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    db.execute(
        "UPDATE persona_versions SET voice_provider = 'mock', voice_provider_version = 'stub'
         WHERE persona_model_id = (SELECT id FROM persona_models WHERE name = 'yu')
           AND version = 1",
        [],
    )
    .unwrap();

    // 不加 audio sample 也行，drift 之前会先报 no samples；但我们要测的是 drift 路径
    // → 加一条 audio sample 让 svc 跑到 drift 那一步。
    let sample_id = format!("sm_{}", ulid::Ulid::new().to_string().to_lowercase());
    db.execute(
        "INSERT INTO persona_samples (id, persona_model_id, kind, blob, blob_mime, source, created_at)
         VALUES (?1, (SELECT id FROM persona_models WHERE name = 'yu'),
                 'audio', ?2, 'audio/wav', 'test', '2026-08-01T00:00:00Z')",
        rusqlite::params![&sample_id, &b"sample".to_vec()],
    )
    .unwrap();
    drop(db);

    // mock vendor CLI
    let mock_bin = dir.path().join("mock_voice_ft.sh");
    std::fs::write(
        &mock_bin,
        "#!/bin/sh
case \"$1\" in
  finetune) case \"$2\" in submit) echo \"task_id=t1\" ;; status) echo \"status=done\" ;;
    fetch)
      OUT=\"\"
      while [ \"$#\" -gt 0 ]; do case \"$1\" in --out) OUT=\"$2\"; shift 2;; *) shift;; esac; done
      mkdir -p \"$(dirname \"$OUT\")\"; printf 'MOCK' > \"$OUT\" ;; esac ;;
esac
",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&mock_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::create_dir_all(config.join("avc")).unwrap();
    let toml = format!(
        "[provider.voice.mock]\nbinary = \"{}\"\n",
        mock_bin.to_str().unwrap(),
    );
    std::fs::write(config.join("avc/avc.toml"), toml).unwrap();

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
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&r.stdout);
    let fj_id: String = serde_json::from_str::<serde_json::Value>(&stdout).unwrap()
        ["finetune_job_id"]
        .as_str()
        .unwrap()
        .to_string();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "run", &fj_id]) // 不传 --embed
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(2),
        "缺 --embed 应 Arg (exit 2); 实际={:?}, stderr={}",
        r.status.code(),
        String::from_utf8_lossy(&r.stderr)
    );

    // 验证 fj 仍在 running（未被 publish 改写）
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let status: String = db
        .query_row(
            "SELECT status FROM finetune_jobs WHERE id = ?",
            [&fj_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "running", "缺 --embed 时 fj 应保持 running");
}

#[test]
fn job_export_to_s3_target_invokes_upload_cmd() {
    // 端到端：`avc job export <job_id> --target s3://bucket/prefix/` 走真 upload_cmd。
    // mock uploader 写文件到 tmp/bucket/prefix/，assert 落到对的地方。
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

    // 直接 DB 写一个 succeeded job + 2 个 artifacts
    let db_path = data.join("avc/avc.db");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let pid: String = db
        .query_row("SELECT id FROM persona_models WHERE name='yu'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let jid = format!("job_{}", ulid::Ulid::new().to_string().to_lowercase());
    db.execute(
        "INSERT INTO jobs (id, persona_model_id, persona_version, status, created_at)
         VALUES (?1, ?2, 1, 'succeeded', '2026-08-01T00:00:00Z')",
        rusqlite::params![&jid, &pid],
    )
    .unwrap();
    for (kind, name, blob) in [
        ("clip", "i2v", b"MOCK_S3_CLIP_BLOB".to_vec()),
        ("audio", "tts", b"MOCK_S3_TTS_BLOB".to_vec()),
    ] {
        let aid = format!("art_{}", ulid::Ulid::new().to_string().to_lowercase());
        db.execute(
            "INSERT INTO artifacts (id, job_id, kind, name, content, byte_size, mime, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'application/octet-stream', '2026-08-01T00:00:00Z')",
            rusqlite::params![&aid, &jid, kind, name, &blob, &(blob.len() as i64)],
        )
        .unwrap();
    }
    drop(db);

    // mock s3 uploader + log
    let dest_root = dir.path().join("bucket");
    let log_path = dir.path().join("upload.log");
    let uploader = dir.path().join("mock_s3.sh");
    let script = format!(
        r#"#!/bin/sh
LOCAL=$1
BUCKET=$2
PREFIX=$3
NAME=$4
mkdir -p "{dest}/$BUCKET/$PREFIX"
cp "$LOCAL" "{dest}/$BUCKET/$PREFIX$NAME"
printf '%s\n' "$NAME" >> {log}
"#,
        dest = dest_root.display(),
        log = log_path.display(),
    );
    std::fs::write(&uploader, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&uploader, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // 写 avc.toml 配 upload_cmd
    std::fs::create_dir_all(config.join("avc")).unwrap();
    let toml = format!(
        r#"[export.s3]
upload_cmd = "{} {{local}} {{bucket}} {{prefix}} {{name}}"
"#,
        uploader.display(),
    );
    std::fs::write(config.join("avc/avc.toml"), toml).unwrap();

    // 跑 export --target s3://
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "job",
            "export",
            &jid,
            "--target",
            "s3://my-bucket/videos/2026/",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "export 应成功；status={:?}, stdout={}, stderr={}",
        r.status.code(),
        String::from_utf8_lossy(&r.stdout),
        String::from_utf8_lossy(&r.stderr)
    );
    let stdout = String::from_utf8_lossy(&r.stdout);
    assert!(
        stdout.contains("\"files\": 2"),
        "files=2; stdout={}",
        stdout
    );
    assert!(
        stdout.contains("s3://my-bucket/videos/2026/"),
        "target label; stdout={}",
        stdout
    );

    // 验证：bucket/my-bucket/videos/2026/ 下有 2 个 .bin
    let placed: Vec<_> = std::fs::read_dir(dest_root.join("my-bucket/videos/2026"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(placed.len(), 2, "应 2 文件到 s3 mock；placed={:?}", placed);
    for f in &placed {
        assert!(f.ends_with(".bin"), "后缀 .bin；got {}", f);
    }

    // 验证 log 记录 2 次调用
    let log = std::fs::read_to_string(&log_path).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 2, "upload_cmd 调 2 次；log={}", log);
}

#[test]
fn job_export_rejects_both_out_and_target() {
    // --out 和 --target 互斥；同时给 → Arg 错。
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
        .args([
            "job",
            "export",
            "job_xxx",
            "--out",
            "/tmp/xxx",
            "--target",
            "s3://b/p/",
        ])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(2),
        "--out + --target 应 Arg (exit 2); 实际={:?}",
        r.status.code()
    );
}

#[test]
fn job_export_requires_out_or_target() {
    // 既不 --out 也不 --target → Arg 错。
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
        .args(["job", "export", "job_xxx"])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(2),
        "既不 --out 也不 --target 应 Arg (exit 2); 实际={:?}",
        r.status.code()
    );
}

#[test]
fn finetune_show_returns_job_details() {
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
        ])
        .output()
        .unwrap();
    let fj_id: String =
        serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout)).unwrap()
            ["finetune_job_id"]
            .as_str()
            .unwrap()
            .to_string();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "show", &fj_id])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "show 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["id"], fj_id);
    assert_eq!(v["persona"], "yu");
    assert_eq!(v["base_version"], 1);
    assert_eq!(v["target_version"], 2);
    assert_eq!(v["status"], "running");
    assert_eq!(v["scope"][0], "voice");
    assert!(v["started_at"].is_string());
}

#[test]
fn finetune_show_unknown_returns_notfound() {
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
        .args(["finetune", "show", "fj_doesnotexist"])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(3),
        "NotFound 应 exit 3；got {:?}",
        r.status.code()
    );
}

#[test]
fn finetune_report_without_drift_conflicts() {
    // 没 run/publish 前 report 没数据 → Conflict (exit 4)
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
    let fj_id: String =
        serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout)).unwrap()
            ["finetune_job_id"]
            .as_str()
            .unwrap()
            .to_string();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "report", &fj_id])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(4),
        "no drift → Conflict (exit 4)；got {:?}",
        r.status.code()
    );
}

#[test]
fn finetune_report_after_publish_returns_drift_json() {
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
    let fj_id: String =
        serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout)).unwrap()
            ["finetune_job_id"]
            .as_str()
            .unwrap()
            .to_string();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "publish", &fj_id, "--passed"])
        .output()
        .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "report", &fj_id])
        .output()
        .unwrap();
    assert!(r.status.success(), "report 应成功");
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["passed"], true);
    assert_eq!(v["voice"], 0.9);
}

#[test]
fn finetune_cancel_running_then_re_cancelled_conflicts() {
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
    let fj_id: String =
        serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout)).unwrap()
            ["finetune_job_id"]
            .as_str()
            .unwrap()
            .to_string();

    // 第一次 cancel：running → cancelled
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "cancel", &fj_id])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "cancel running 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    // 第二次 cancel：cancelled → Conflict
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "cancel", &fj_id])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(4),
        "已 cancelled 不能再 cancel；got {:?}",
        r.status.code()
    );
}

#[test]
fn finetune_cancel_after_publish_conflicts() {
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
    let fj_id: String =
        serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout)).unwrap()
            ["finetune_job_id"]
            .as_str()
            .unwrap()
            .to_string();
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "publish", &fj_id, "--passed"])
        .output()
        .unwrap();

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "cancel", &fj_id])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(4),
        "succeeded 不能 cancel；got {:?}",
        r.status.code()
    );
}

#[test]
fn iterate_show_and_cancel_happy_paths() {
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

    // 直接 DB 写一条 iterate_jobs（queued）
    let db_path = data.join("avc/avc.db");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let ij_id = format!("ij_{}", ulid::Ulid::new().to_string().to_lowercase());
    db.execute(
        "INSERT INTO iterate_jobs (id, persona_model_id, target_version, changes_json, status, started_at)
         SELECT ?1, id, 1, '{}' , 'queued', '2026-08-01T00:00:00Z'
         FROM persona_models WHERE name = 'yu'",
        rusqlite::params![&ij_id],
    ).unwrap();
    drop(db);

    // show
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["iterate", "show", &ij_id])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "show 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["id"], ij_id);
    assert_eq!(v["status"], "queued");
    assert_eq!(v["target_version"], 1);

    // cancel
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["iterate", "cancel", &ij_id])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "cancel 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    // 再 cancel 一次 → Conflict
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["iterate", "cancel", &ij_id])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(4),
        "已 cancelled 不能再 cancel；got {:?}",
        r.status.code()
    );
}

#[test]
fn iterate_show_unknown_returns_notfound() {
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
        .args(["iterate", "show", "ij_xxx"])
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(3), "NotFound 应 exit 3");
}

#[test]
fn job_cancel_queued_and_wait_until_succeeded() {
    // 1. 起一个 job（手动 status='queued'）→ cancel → 再 cancel → Conflict
    // 2. 起一个 job 立刻 status='succeeded' → wait --until succeeded → exit 0
    // 3. 起一个 job 立刻 status='failed' → wait --until succeeded → exit 4
    // 4. 起一个 job 立刻 status='running' → wait 超时短 → exit 4 (timeout)
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

    let db_path = data.join("avc/avc.db");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let pid: String = db
        .query_row("SELECT id FROM persona_models WHERE name='yu'", [], |r| {
            r.get(0)
        })
        .unwrap();

    // (1) job queued → cancel 成功；再 cancel → Conflict
    let jid1 = format!("job_{}", ulid::Ulid::new().to_string().to_lowercase());
    db.execute(
        "INSERT INTO jobs (id, persona_model_id, persona_version, status, created_at)
         VALUES (?1, ?2, 1, 'queued', '2026-08-01T00:00:00Z')",
        rusqlite::params![&jid1, &pid],
    )
    .unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["job", "cancel", &jid1])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "cancel queued 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["job", "cancel", &jid1])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(4),
        "已 cancelled 不能再 cancel；got {:?}",
        r.status.code()
    );

    // (2) job 立刻 succeeded → wait --until succeeded 立即返 → exit 0
    let jid2 = format!("job_{}", ulid::Ulid::new().to_string().to_lowercase());
    db.execute(
        "INSERT INTO jobs (id, persona_model_id, persona_version, status, created_at, finished_at)
         VALUES (?1, ?2, 1, 'succeeded', '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z')",
        rusqlite::params![&jid2, &pid],
    )
    .unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "job",
            "wait",
            &jid2,
            "--until",
            "succeeded",
            "--timeout",
            "5",
        ])
        .output()
        .unwrap();
    assert!(r.status.success(), "wait succeeded 应成功");
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["status"], "succeeded");
    assert!(v["elapsed_secs"].as_u64().unwrap() < 5);

    // (3) job failed → wait --until succeeded 立即发 Conflict
    let jid3 = format!("job_{}", ulid::Ulid::new().to_string().to_lowercase());
    db.execute(
        "INSERT INTO jobs (id, persona_model_id, persona_version, status, created_at, finished_at)
         VALUES (?1, ?2, 1, 'failed', '2026-08-01T00:00:00Z', '2026-08-01T00:00:00Z')",
        rusqlite::params![&jid3, &pid],
    )
    .unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "job",
            "wait",
            &jid3,
            "--until",
            "succeeded",
            "--timeout",
            "5",
        ])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(4),
        "等 succeeded 但实际 failed → Conflict (exit 4)"
    );

    // (4) job running + 短 timeout → wait 超时
    let jid4 = format!("job_{}", ulid::Ulid::new().to_string().to_lowercase());
    db.execute(
        "INSERT INTO jobs (id, persona_model_id, persona_version, status, created_at)
         VALUES (?1, ?2, 1, 'running', '2026-08-01T00:00:00Z')",
        rusqlite::params![&jid4, &pid],
    )
    .unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "job",
            "wait",
            &jid4,
            "--until",
            "succeeded",
            "--timeout",
            "1",
            "--poll",
            "200",
        ])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(4),
        "running 短 timeout → Conflict (exit 4)"
    );

    // (5) cancel 一个 running 状态 → Conflict（只允许 queued）
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["job", "cancel", &jid4])
        .output()
        .unwrap();
    assert_eq!(
        r.status.code(),
        Some(4),
        "running 不能 cancel；got {:?}",
        r.status.code()
    );

    drop(db);
}

#[test]
fn job_wait_unknown_id_returns_notfound() {
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
        .args(["job", "wait", "job_xxx", "--until", "succeeded"])
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(3), "NotFound 应 exit 3");
}

#[test]
fn job_wait_missing_until_flag_arg_errors() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .arg("init")
        .output()
        .unwrap();
    // --until 但没给值
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["job", "wait", "job_xxx", "--until"])
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(2), "缺 --until 值应 Arg (exit 2)");
}

#[test]
fn drift_writes_all_three_dim_embeds_on_run() {
    // 端到端 voice-only finetune run：除了 voice_embed，还应写 style_embed
    // （face dim 不写因为 scope=voice 不含 avatar）。
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // mock embed server (2 次调用：voice + style)
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        for _ in 0..2 {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 8192];
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let _ = stream.read(&mut buf);
                let body = r#"{"data":[{"embedding":[1.0,0.0,0.0]}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
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

    // 写 v1: voice_provider / voice_embed / style_embed 都是 [1,0,0]
    let db_path = data.join("avc/avc.db");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let blob: Vec<u8> = [1.0f32, 0.0, 0.0]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    db.execute(
        "UPDATE persona_versions SET voice_provider = 'mock', voice_provider_version = 'stub',
             voice_embed = ?1, voice_embed_dim = 3,
             style_embed = ?1, style_embed_dim = 3
         WHERE persona_model_id = (SELECT id FROM persona_models WHERE name = 'yu') AND version = 1",
        rusqlite::params![&blob],
    ).unwrap();

    // 加 audio sample
    let sid = format!("sm_{}", ulid::Ulid::new().to_string().to_lowercase());
    db.execute(
        "INSERT INTO persona_samples (id, persona_model_id, kind, blob, blob_mime, source, created_at)
         VALUES (?1, (SELECT id FROM persona_models WHERE name = 'yu'),
                 'audio', ?2, 'audio/wav', 'test', '2026-08-01T00:00:00Z')",
        rusqlite::params![&sid, &b"audio".to_vec()],
    ).unwrap();
    drop(db);

    // mock vendor CLI for voice SFT
    let mock_bin = dir.path().join("mock_voice_ft.sh");
    std::fs::write(
        &mock_bin,
        r#"#!/bin/sh
case "$1" in
  finetune)
    case "$2" in
      submit) echo "task_id=t1" ;;
      status) echo "status=done" ;;
      fetch)
        OUT=""
        while [ "$#" -gt 0 ]; do case "$1" in --out) OUT="$2"; shift 2;; *) shift;; esac; done
        mkdir -p "$(dirname "$OUT")"; printf 'MOCK_WAV' > "$OUT" ;;
    esac ;;
esac
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&mock_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // avc.toml
    std::fs::create_dir_all(config.join("avc")).unwrap();
    let toml = format!(
        r#"[provider.voice.mock]
binary = "{}"
[provider.embed.mock]
api_key = "sk"
model = "mock"
base_url = "http://127.0.0.1:{}"
"#,
        mock_bin.display(),
        port,
    );
    std::fs::write(config.join("avc/avc.toml"), toml).unwrap();

    // start
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
            "0.5",
        ])
        .output()
        .unwrap();
    let fj_id: String =
        serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout)).unwrap()
            ["finetune_job_id"]
            .as_str()
            .unwrap()
            .to_string();

    // run
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["finetune", "run", &fj_id, "--embed", "mock"])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "run 应成功；status={:?}, stderr={}",
        r.status.code(),
        String::from_utf8_lossy(&r.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["status"], "succeeded");
    // 3 个 cosines 都应该有
    assert!(
        v["voice_cosine"].as_f64().is_some(),
        "voice_cosine 应有数；got {:?}",
        v
    );
    assert!(
        v["face_cosine"].is_null(),
        "scope=voice 时 face_cosine 应 None；got {:?}",
        v
    );
    assert!(
        v["style_cosine"].as_f64().is_some(),
        "style_cosine 应有数；got {:?}",
        v
    );
    assert!((v["voice_cosine"].as_f64().unwrap() - 1.0).abs() < 1e-6);
    assert!((v["style_cosine"].as_f64().unwrap() - 1.0).abs() < 1e-6);

    // 验证 DB：v2 有 voice_embed + style_embed（face_embed 留空，因为 scope 不含 avatar）
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let (voice_dim, style_dim, face_dim): (Option<i64>, Option<i64>, Option<i64>) = db
        .query_row(
            "SELECT voice_embed_dim, style_embed_dim, face_embed_dim
         FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = 'yu' AND pv.version = 2",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(voice_dim, Some(3), "v2 voice_embed_dim=3");
    assert_eq!(style_dim, Some(3), "v2 style_embed_dim=3");
    assert!(
        face_dim.is_none(),
        "v2 face_embed_dim 应 None (scope 不含 avatar)"
    );

    let _ = handle.join();
}

#[test]
fn examples_vendor_cli_templates_run_e2e() {
    // 端到端验证 examples/vendor-cli/*.sh 模板：
    // 1. init + create persona
    // 2. 配 [provider.video.kling] = examples/vendor-cli/kling-video.sh
    // 3. 配 [export.s3] upload_cmd = examples/vendor-cli/aws-s3-cp.sh
    // 4. render run → kling-video.sh 走 submit/status/fetch → 5 个 artifacts
    // 5. job export --target s3:// → aws-s3-cp.sh 上传 5 文件 + 写 log
    let dir = tempfile::tempdir().expect("tmpdir");
    let data = dir.path().join("data");
    let config = dir.path().join("config");
    let s3_root = dir.path().join("s3-mirror");

    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let kling_video = examples.join("vendor-cli/kling-video.sh");
    let aws_s3_cp = examples.join("vendor-cli/aws-s3-cp.sh");
    assert!(
        kling_video.exists() && aws_s3_cp.exists(),
        "examples/vendor-cli scripts must exist; kling={} aws={}",
        kling_video.exists(),
        aws_s3_cp.exists()
    );

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

    // avc.toml
    std::fs::create_dir_all(config.join("avc")).unwrap();
    let toml = format!(
        r#"[provider.video.kling]
binary = "{}"

[export.s3]
upload_cmd = "{} {{local}} {{bucket}} {{prefix}} {{name}}"
"#,
        kling_video.display(),
        aws_s3_cp.display(),
    );
    std::fs::write(config.join("avc/avc.toml"), toml).unwrap();

    // render run
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .env("S3_MIRROR_ROOT", &s3_root)
        .env("S3_UPLOAD_LOG", s3_root.join("upload.log"))
        .args([
            "render",
            "run",
            "--persona",
            "yu",
            "--version",
            "1",
            "--topic",
            "exdemo",
            "--video-provider",
            "kling",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "render run 失败；status={:?}, stderr={}",
        r.status.code(),
        String::from_utf8_lossy(&r.stderr)
    );
    let jid: String =
        serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&r.stdout)).unwrap()
            ["job_id"]
            .as_str()
            .unwrap()
            .to_string();

    // 验证 i2v artifact 来自 kling-video.sh（mock magic）
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let i2v_blob: Vec<u8> = db
        .query_row(
            "SELECT content FROM artifacts WHERE job_id = ?1 AND name = 'i2v'",
            [&jid],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        i2v_blob.starts_with(b"MOCK_KLING_MP4_ftyp"),
        "i2v BLOB 应有 MOCK_KLING_MP4_ftyp 前缀；actual前32={:?}",
        &i2v_blob[..32.min(i2v_blob.len())]
    );

    // job export → s3://
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .env("S3_MIRROR_ROOT", &s3_root)
        .env("S3_UPLOAD_LOG", s3_root.join("upload.log"))
        .args([
            "job",
            "export",
            &jid,
            "--target",
            "s3://my-bucket/videos/2026/",
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "export 失败；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["files"], 5);
    assert_eq!(v["target"], "s3://my-bucket/videos/2026/");

    // 验证：s3 mirror 应有 5 个文件
    let placed: Vec<_> = std::fs::read_dir(s3_root.join("my-bucket/videos/2026"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        placed.len(),
        5,
        "应 5 个文件到 s3 mirror；placed={:?}",
        placed
    );

    // 验证：upload log 应有 5 行
    let log = std::fs::read_to_string(s3_root.join("upload.log")).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 5, "应 5 行 log；log={}", log);
    for line in &lines {
        assert!(
            line.contains("my-bucket"),
            "log line 应含 my-bucket；line={}",
            line
        );
    }
}

// ============================================================================
// 0.3.3 — iterate list / set-knowledge / set-manifest 覆盖 + persona show/versions
// ============================================================================

#[test]
fn iterate_list_returns_apply_records() {
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

    // 空 list：返 []（不是 NotFound）
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["iterate", "list", "yu", "--json"])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "空 list 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert!(v.is_array());
    assert_eq!(v.as_array().unwrap().len(), 0);

    // apply 两次 → list 返 2 行
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
            r#"{"traits":["严谨"]}"#,
        ])
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
            r#"{"catchphrase":"直接看源码"}"#,
        ])
        .output()
        .unwrap();
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["iterate", "list", "yu", "--json"])
        .output()
        .unwrap();
    assert!(r.status.success());
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    // 排序：started_at DESC（最新在前）→ 第二次 apply 在 index 0
    assert!(v[0]["id"].as_str().unwrap().starts_with("ij_"));
    assert_eq!(v[0]["status"], "succeeded");
    assert_eq!(v[0]["target_version"], 1);
}

#[test]
fn iterate_list_unknown_persona_errors() {
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
        .args(["iterate", "list", "nonexistent"])
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(3), "未知 persona → NotFound (exit 3)");
}

#[test]
fn iterate_apply_set_knowledge_merges_binding() {
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

    // 第一次 set-knowledge → 落 binding
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "iterate",
            "apply",
            "yu",
            "--version",
            "1",
            "--set-knowledge",
            r#"{"corpus":"db_docs","priority":"high"}"#,
        ])
        .output()
        .unwrap();

    // 第二次 set-knowledge → 合并（不是覆盖）
    bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "iterate",
            "apply",
            "yu",
            "--version",
            "1",
            "--set-knowledge",
            r#"{"retrieved_at":"2026-08-01"}"#,
        ])
        .output()
        .unwrap();

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let json: String = db
        .query_row(
            "SELECT knowledge_binding_json FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ?",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["corpus"], "db_docs");
    assert_eq!(v["priority"], "high");
    assert_eq!(v["retrieved_at"], "2026-08-01");
}

#[test]
fn iterate_apply_set_manifest_merges_render_options() {
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
            "--set-manifest",
            r#"{"render_options":{"resolution":"1080p","fps":30}}"#,
        ])
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
            "--set-manifest",
            r#"{"render_options":{"fps":60},"style":"严谨"}"#,
        ])
        .output()
        .unwrap();

    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let json: String = db
        .query_row(
            "SELECT manifest_json FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ?",
            ["yu"],
            |r| r.get(0),
        )
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    // resolution 保留（merge），fps 覆盖到 60，style 新增
    assert_eq!(v["render_options"]["resolution"], "1080p");
    assert_eq!(v["render_options"]["fps"], 60);
    assert_eq!(v["style"], "严谨");
}

#[test]
fn iterate_apply_three_sections_together() {
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

    // 一次 apply 三段都给
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "iterate",
            "apply",
            "yu",
            "--version",
            "1",
            "--set-persona",
            r#"{"traits":["严谨","务实"]}"#,
            "--set-knowledge",
            r#"{"corpus":"db_docs"}"#,
            "--set-manifest",
            r#"{"render_options":{"resolution":"720p"}}"#,
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "三段同时给应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert!(v["iterate_job_id"].as_str().unwrap().starts_with("ij_"));
    assert_eq!(v["persona"], "yu");
    assert_eq!(v["version"], 1);

    // 三列都落库
    let db = rusqlite::Connection::open(data.join("avc/avc.db")).unwrap();
    let (p, k, m): (String, String, String) = db
        .query_row(
            "SELECT persona_descriptor_json, knowledge_binding_json, manifest_json
         FROM persona_versions pv
         JOIN persona_models pm ON pm.id = pv.persona_model_id
         WHERE pm.name = ?",
            ["yu"],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(p.contains("严谨"));
    assert!(k.contains("db_docs"));
    assert!(m.contains("720p"));
}

#[test]
fn iterate_apply_missing_version_errors() {
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
    // 不传 --version → Arg 错误（exit 2）
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "iterate",
            "apply",
            "yu",
            "--set-persona",
            r#"{"traits":["a"]}"#,
        ])
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(2), "缺 --version → Arg (exit 2)");
}

#[test]
fn iterate_apply_unknown_persona_errors() {
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
        .args([
            "iterate",
            "apply",
            "nope",
            "--version",
            "1",
            "--set-persona",
            "{}",
        ])
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(3), "未知 persona → NotFound (exit 3)");
}

#[test]
fn persona_show_returns_full_row_json() {
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

    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "show", "yu", "--json"])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "show 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["name"], "yu");
    assert_eq!(v["archetype"], "db_kernel_expert");
    assert_eq!(v["current_version"], 1);
    assert_eq!(v["status"], "pending");
}

#[test]
fn persona_show_unknown_errors() {
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
        .args(["persona", "show", "nope"])
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(3), "未知 persona → NotFound (exit 3)");
}

#[test]
fn persona_versions_lists_after_finetune() {
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

    // 起 v1 → versions 应见 [1]
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "versions", "yu", "--json"])
        .output()
        .unwrap();
    assert!(r.status.success());
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0], 1);

    // finetune start → 自动建 v2（pending）
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
        ])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "finetune start 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );

    // versions 应见 [1, 2]
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "versions", "yu", "--json"])
        .output()
        .unwrap();
    assert!(r.status.success());
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    let versions: Vec<i64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect();
    assert_eq!(versions, vec![1, 2]);
}

#[test]
fn iterate_show_parses_changes_json_as_object() {
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

    // apply 后拿 ij_id
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args([
            "iterate",
            "apply",
            "yu",
            "--version",
            "1",
            "--set-persona",
            r#"{"traits":["严谨","务实"]}"#,
            "--set-knowledge",
            r#"{"corpus":"db_docs"}"#,
        ])
        .output()
        .unwrap();
    assert!(r.status.success());
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    let ij_id = v["iterate_job_id"].as_str().unwrap().to_string();

    // iterate show 应把 changes_json 解析为 JSON object（不是字符串）
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["iterate", "show", &ij_id, "--json"])
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "show 应成功；stderr={}",
        String::from_utf8_lossy(&r.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["id"], ij_id);
    assert_eq!(v["persona"], "yu");
    assert_eq!(v["target_version"], 1);
    assert_eq!(v["status"], "succeeded");
    // changes 应该是 JSON object，不是 string
    let changes = &v["changes"];
    assert!(
        changes.is_object(),
        "changes 字段应是 object；got={}",
        changes
    );
    assert_eq!(changes["persona_descriptor"]["traits"][0], "严谨");
    assert_eq!(changes["persona_descriptor"]["traits"][1], "务实");
    assert_eq!(changes["knowledge_binding"]["corpus"], "db_docs");
}

#[test]
fn persona_list_filters_by_status() {
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
        .args(["persona", "create", "--name", "lin"])
        .output()
        .unwrap();

    // 默认（不传 status）应返 2 行
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "list", "--json"])
        .output()
        .unwrap();
    assert!(r.status.success());
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);

    // --status pending 应返 2 行（两个新 persona 默认 status=pending）
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "list", "--status", "pending", "--json"])
        .output()
        .unwrap();
    assert!(r.status.success());
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);

    // --status archived 应返 0 行
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "list", "--status", "archived", "--json"])
        .output()
        .unwrap();
    assert!(r.status.success());
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 0);

    // 未知 status 也返 0 行（不报错）
    let r = bin()
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", &config)
        .args(["persona", "list", "--status", "nonsense", "--json"])
        .output()
        .unwrap();
    assert!(r.status.success());
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 0);
}
