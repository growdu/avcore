//! render-svc：渲染出片
//!
//! Phase 1：仅骨架 + 与 artifacts 表交互。
//! 详见 docs/modules/video-generation.md。

use crate::db::Db;
use crate::error::{AvcError, AvcResult};
use rusqlite::OptionalExtension;

/// artifact 元组：(kind, name, byte_size, mime)
type ArtifactRow = (String, String, Option<i64>, Option<String>);
/// pack 错误列表：(topic, error_msg)
type PackErrors = Vec<(String, String)>;

pub fn create_job(db: &Db, name: &str, version: i64, _topic: &str) -> AvcResult<String> {
    let p = crate::svc::persona::get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();

    // Task 3：在任何 INSERT 前、同一连接内校验 version 状态。
    // - 无行 → NotFound("persona '<name>' version <n>")
    // - status 既不是 'ready' 也不是 'pending' → Conflict (信息含 version/status)
    let ver_status: Option<String> = conn
        .query_row(
            "SELECT status FROM persona_versions
             WHERE persona_model_id = ? AND version = ?",
            rusqlite::params![&p.id, version],
            |r| r.get(0),
        )
        .optional()?;
    match ver_status {
        None => {
            return Err(AvcError::NotFound(format!(
                "persona '{}' version {}",
                name, version
            )));
        }
        Some(s) if s != "ready" && s != "pending" => {
            return Err(AvcError::Conflict(format!(
                "persona '{}' version {} is not stable (status: {})",
                name, version, s
            )));
        }
        _ => {} // ready 或 pending，放行
    }

    let job_id = crate::svc::new_id("job");
    let now = crate::svc::now_iso();
    conn.execute(
        "INSERT INTO jobs (id, script_id, persona_model_id, persona_version, status, progress, created_at)
         VALUES (?, NULL, ?, ?, 'queued', 0, ?)",
        rusqlite::params![&job_id, &p.id, version, &now],
    )?;
    Ok(job_id)
}

pub fn list_jobs(db: &Db, name: &str) -> AvcResult<Vec<String>> {
    let p = crate::svc::persona::get_persona(db, name)?;
    let conn = db.conn.lock().unwrap();
    let mut stmt =
        conn.prepare("SELECT id FROM jobs WHERE persona_model_id = ? ORDER BY created_at DESC")?;
    let rows = stmt.query_map([&p.id], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get_job(db: &Db, job_id: &str) -> AvcResult<String> {
    let conn = db.conn.lock().unwrap();
    let status: String = conn
        .query_row("SELECT status FROM jobs WHERE id = ?", [job_id], |r| {
            r.get(0)
        })
        .map_err(|_| AvcError::NotFound(format!("job '{}'", job_id)))?;
    Ok(status)
}

/// Phase 2: list_artifacts → 给 `avc job show --artifacts` 用。
pub fn list_artifacts(db: &Db, job_id: &str) -> AvcResult<Vec<ArtifactRow>> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT kind, name, byte_size, mime FROM artifacts WHERE job_id = ? ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([job_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Phase 2.5: export 目标。
///
/// - `Local(dir)`: 落 FS 到 `dir/<kind>__<name>__<id>.bin`（Phase 2 既有行为）。
/// - `S3 { bucket, prefix, upload_cmd }`: 每个 artifact materializes 到 tmp 后用
///   `upload_cmd` 模板（`{local} {bucket} {prefix} {name}` 占位符替换后 `sh -c`）
///   跑上传；上传完即删 tmp。`upload_cmd` 默认是 `aws s3 cp --region us-east-1
///   {local} s3://{bucket}/{prefix}{name}`（来自 `[export.s3]` config）。
#[derive(Debug, Clone)]
pub enum ExportTarget<'a> {
    Local(&'a std::path::Path),
    S3 {
        bucket: &'a str,
        prefix: &'a str,
        upload_cmd: &'a str,
    },
}

/// Phase 2.5: 把一个 job 的所有 artifacts 按 target 导出。
/// 返 (写出文件数, 累计 bytes)。
pub fn export_artifacts(
    db: &Db,
    job_id: &str,
    target: ExportTarget<'_>,
) -> AvcResult<(usize, u64)> {
    // 0. 校验 job 存在
    let exists: bool = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT 1 FROM jobs WHERE id = ?", [job_id], |r| {
            r.get::<_, i64>(0).map(|_| true)
        })
        .optional()?
        .unwrap_or(false);
    if !exists {
        return Err(AvcError::NotFound(format!("job '{}'", job_id)));
    }

    // 1. 准备 target
    match target {
        ExportTarget::Local(out_dir) => {
            std::fs::create_dir_all(out_dir)
                .map_err(|e| AvcError::Db(format!("mkdir {}: {}", out_dir.display(), e)))?;
        }
        ExportTarget::S3 { .. } => {
            // S3 不需要 prep
        }
    }

    // 2. 读每条 artifact BLOB
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, kind, name, content FROM artifacts WHERE job_id = ? ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([job_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Vec<u8>>(3)?,
        ))
    })?;
    let mut count = 0usize;
    let mut total_bytes = 0u64;
    for r in rows {
        let (id, kind, name, blob) = r?;
        let safe_kind = sanitize(&kind, &['_', '-']);
        let safe_name = sanitize(&name, &['_', '-', '.']);
        let file_name = format!("{}__{}__{}.bin", safe_kind, safe_name, id);

        match target {
            ExportTarget::Local(out_dir) => {
                let path = out_dir.join(&file_name);
                std::fs::write(&path, &blob)
                    .map_err(|e| AvcError::Db(format!("write {}: {}", path.display(), e)))?;
            }
            ExportTarget::S3 {
                bucket,
                prefix,
                upload_cmd,
            } => {
                // materializes 到 tmp + 调 upload_cmd
                let tmp = std::env::temp_dir().join(format!("avc-export-{}-{}", job_id, file_name));
                std::fs::write(&tmp, &blob)
                    .map_err(|e| AvcError::Db(format!("write tmp {}: {}", tmp.display(), e)))?;
                let cmd_str = upload_cmd
                    .replace("{local}", &shell_quote(&tmp.display().to_string()))
                    .replace("{bucket}", &shell_quote(bucket))
                    .replace("{prefix}", &shell_quote(prefix))
                    .replace("{name}", &shell_quote(&file_name));
                let out = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd_str)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output()
                    .map_err(|e| {
                        // 尽力清 tmp
                        let _ = std::fs::remove_file(&tmp);
                        AvcError::ProviderUpstream(format!("spawn upload_cmd `{}`: {}", cmd_str, e))
                    })?;
                let _ = std::fs::remove_file(&tmp);
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    return Err(AvcError::ProviderUpstream(format!(
                        "upload_cmd `{}` exit {:?}: stdout={} stderr={}",
                        cmd_str,
                        out.status.code(),
                        stdout,
                        stderr
                    )));
                }
            }
        }
        total_bytes += blob.len() as u64;
        count += 1;
    }
    Ok((count, total_bytes))
}

fn sanitize(s: &str, extra: &[char]) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || extra.contains(&c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 单引号包裹（POSIX shell）；对已含单引号的字符串不完美，但 artifact name 是受控的
///（只走 sanitize 输出），实际不会出现 '。
fn shell_quote(s: &str) -> String {
    format!("'{}'", s)
}

/// Phase 2: 反馈入口 — 用户标 "looks_unlike" → 写 persona_samples(kind='feedback', source='user_feedback')。
/// reason 可空（CLI --reason 留空就 NULL）。
/// 返 sample_id。
pub fn feedback(
    db: &Db,
    job_id: &str,
    looks_unlike: bool,
    reason: Option<&str>,
) -> AvcResult<String> {
    if !looks_unlike {
        return Err(AvcError::Arg(
            "feedback: only --looks-unlike is supported in Phase 2".into(),
        ));
    }
    let conn = db.conn.lock().unwrap();
    let (persona_id, persona_version, _script_id): (String, i64, Option<String>) = conn
        .query_row(
            "SELECT persona_model_id, persona_version, script_id FROM jobs WHERE id = ?",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| AvcError::NotFound(format!("job '{}'", job_id)))?;
    drop(conn);

    let sample_id = crate::svc::new_id("smp");
    let now = crate::svc::now_iso();
    let text = reason.unwrap_or("looks_unlike");
    let conn2 = db.conn.lock().unwrap();
    conn2.execute(
        "INSERT INTO persona_samples (
            id, persona_model_id, version_id_at_collection, source, kind, text, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            &sample_id,
            &persona_id,
            persona_version,
            "user_feedback",
            "feedback",
            text,
            &now,
        ],
    )?;
    Ok(sample_id)
}

/// Phase 2.4: pack — 从 topics-file 逐行读 topic → 跑 create_job + pipeline.run。
/// 失败不中断其他 topic；返 (job_ids, errors: Vec<(topic, error_msg)>)。
/// - 失败容忍：单 topic 失败 → 记录 + 继续
/// - 失败后 job 状态由 `pipeline::run` 写为 'failed' + error_json，**继续**跑下一个 topic
/// - 全部结束后由 CLI 一次性输出汇总 JSON
pub fn pack(
    db: &Db,
    persona: &str,
    version: Option<i64>,
    topics_file: &std::path::Path,
) -> AvcResult<(Vec<String>, PackErrors)> {
    // 1. 校验 persona 存在 + version 决定
    let v = match version {
        Some(v) => v,
        None => crate::svc::persona::get_persona(db, persona)?.current_version,
    };

    // 2. 读 topics 文件（每行一条，跳过空行 + `#` 开头注释）
    let text = std::fs::read_to_string(topics_file)
        .map_err(|e| AvcError::Db(format!("read topics file {}: {}", topics_file.display(), e)))?;
    let topics: Vec<String> = text
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect();
    if topics.is_empty() {
        return Err(AvcError::Arg(format!(
            "topics file {} is empty (only blank lines / comments)",
            topics_file.display()
        )));
    }

    // 3. 复用 render_publishment_spec 单跑每条
    let spec = crate::svc::pipeline::render_publishment_spec();
    let mut job_ids = Vec::with_capacity(topics.len());
    let mut errors = Vec::new();
    for topic in topics {
        match create_job(db, persona, v, &topic) {
            Ok(job_id) => {
                if let Err(e) = crate::svc::pipeline::run(db, &job_id, &spec, &topic) {
                    // job 已被 pipeline::run 写 failed + error_json；记录 + 继续
                    errors.push((topic.clone(), e.to_string()));
                }
                job_ids.push(job_id);
            }
            Err(e) => {
                errors.push((topic.clone(), e.to_string()));
            }
        }
    }
    Ok((job_ids, errors))
}

#[cfg(test)]
mod export_tests {
    use super::*;
    use crate::db::Db;

    fn open_temp_db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().expect("tmpdir");
        let db = Db::open(&dir.path().join("test.db")).expect("open db");
        (dir, db)
    }

    /// 起一个 job + 2 个 artifacts（kind=clip, name=i2v / script_gen）。
    fn seed_job_with_artifacts(db: &Db) -> String {
        // 1. persona
        let pid = crate::svc::new_id("pm");
        let now = crate::svc::now_iso();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO persona_models (id, name, current_version, status, created_at, updated_at)
             VALUES (?1, 'yu', 1, 'active', ?2, ?2)",
            rusqlite::params![&pid, &now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO persona_versions (persona_model_id, version, status, created_at)
             VALUES (?1, 1, 'ready', ?2)",
            rusqlite::params![&pid, &now],
        )
        .unwrap();
        // 2. job（jobs 表无 topic 列；status='succeeded' 让 export 不依赖 step 状态）
        let jid = crate::svc::new_id("job");
        conn.execute(
            "INSERT INTO jobs (id, persona_model_id, persona_version, status, created_at)
             VALUES (?1, ?2, 1, 'succeeded', ?3)",
            rusqlite::params![&jid, &pid, &now],
        )
        .unwrap();
        // 3. artifacts
        for (kind, name, blob) in [
            ("clip", "i2v", b"MOCK_CLIP_BLOB".to_vec()),
            ("audio", "tts", b"MOCK_TTS_BLOB".to_vec()),
        ] {
            let aid = crate::svc::new_id("art");
            conn.execute(
                "INSERT INTO artifacts (id, job_id, kind, name, content, byte_size, mime, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'application/octet-stream', ?7)",
                rusqlite::params![&aid, &jid, kind, name, &blob, &(blob.len() as i64), &now],
            )
            .unwrap();
        }
        drop(conn);
        jid
    }

    #[test]
    fn export_local_writes_files_to_dir() {
        let (dir, db) = open_temp_db();
        let jid = seed_job_with_artifacts(&db);
        let out_dir = dir.path().join("out");
        let (count, bytes) =
            export_artifacts(&db, &jid, ExportTarget::Local(&out_dir)).expect("export ok");
        assert_eq!(count, 2);
        assert!(bytes > 20);
        let files: Vec<_> = std::fs::read_dir(&out_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(files.len(), 2, "应写 2 个文件；out_dir={:?}", out_dir);
    }

    #[test]
    fn export_s3_invokes_upload_cmd_per_artifact() {
        // mock uploader 写到 /tmp/avc-s3-mock/<bucket>/<prefix><name>，并 echo 调用 args 到 log
        let dir = tempfile::tempdir().expect("tmpdir");
        let dest_root = dir.path().join("bucket");
        std::fs::create_dir_all(&dest_root).unwrap();
        let log_path = dir.path().join("upload.log");

        let uploader = dir.path().join("mock_s3.sh");
        let script = format!(
            r#"#!/bin/sh
# mock s3 cp: 接 4 个替换后的参数，写文件 + 记 log
# 期望 args: <local> <bucket> <prefix> <name>
LOCAL=$1
BUCKET=$2
PREFIX=$3
NAME=$4
mkdir -p "{dest}/$BUCKET/$PREFIX"
mkdir -p "$(dirname "$LOCAL")"
cp "$LOCAL" "{dest}/$BUCKET/$PREFIX$NAME"
printf '%s %s %s %s
' "$LOCAL" "$BUCKET" "$PREFIX" "$NAME" >> {log}
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
        let upload_cmd = format!(
            "{} {{local}} {{bucket}} {{prefix}} {{name}}",
            uploader.display()
        );

        let (tdb_dir, db) = open_temp_db();
        let jid = seed_job_with_artifacts(&db);
        let (count, bytes) = export_artifacts(
            &db,
            &jid,
            ExportTarget::S3 {
                bucket: "yu-bucket",
                prefix: "videos/2026/",
                upload_cmd: &upload_cmd,
            },
        )
        .expect("export ok");
        assert_eq!(count, 2);
        assert!(bytes > 20);

        // 验证：2 个文件落到 dest_root/yu-bucket/videos/2026/<name>
        let placed: Vec<_> = std::fs::read_dir(dest_root.join("yu-bucket/videos/2026"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            placed.len(),
            2,
            "应 2 个文件落到 s3 mock；placed={:?}",
            placed
        );
        for f in &placed {
            assert!(f.ends_with(".bin"), "文件名后缀 .bin；got {}", f);
        }

        // 验证 log 记录了 2 次调用
        let log = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2, "upload_cmd 应被调 2 次；log={}", log);
        for line in &lines {
            assert!(line.contains("yu-bucket videos/2026/"), "line={}", line);
        }

        // 验证 tdb_dir 已经被 drop（虽然不是真删，但表明 no panic）
        let _ = tdb_dir;
    }

    #[test]
    fn export_s3_upload_cmd_failure_returns_provider_upstream() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let uploader = dir.path().join("fail.sh");
        std::fs::write(
            &uploader,
            "#!/bin/sh
echo upload failed >&2
exit 1
",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&uploader, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let upload_cmd = format!(
            "{} {{local}} {{bucket}} {{prefix}} {{name}}",
            uploader.display()
        );

        let (_tdb_dir, db) = open_temp_db();
        let jid = seed_job_with_artifacts(&db);
        let res = export_artifacts(
            &db,
            &jid,
            ExportTarget::S3 {
                bucket: "b",
                prefix: "p/",
                upload_cmd: &upload_cmd,
            },
        );
        assert!(
            matches!(res, Err(AvcError::ProviderUpstream(_))),
            "upload_cmd exit !=0 应 ProviderUpstream；got {:?}",
            res.map(|r| r.0)
        );
    }

    #[test]
    fn export_s3_upload_cmd_missing_returns_provider_upstream() {
        let (_tdb_dir, db) = open_temp_db();
        let jid = seed_job_with_artifacts(&db);
        let res = export_artifacts(
            &db,
            &jid,
            ExportTarget::S3 {
                bucket: "b",
                prefix: "p/",
                upload_cmd: "/nonexistent/path/to/uploader {local} {bucket} {prefix} {name}",
            },
        );
        assert!(
            matches!(res, Err(AvcError::ProviderUpstream(_))),
            "spawn fail 应 ProviderUpstream；got {:?}",
            res.map(|r| r.0)
        );
    }

    #[test]
    fn export_nonexistent_job_returns_notfound() {
        let (_tdb_dir, db) = open_temp_db();
        let res = export_artifacts(
            &db,
            "job_xxx",
            ExportTarget::Local(std::path::Path::new("/tmp")),
        );
        assert!(matches!(res, Err(AvcError::NotFound(_))));
    }

    #[test]
    fn shell_quote_wraps_in_single_quotes() {
        assert_eq!(shell_quote("abc"), "'abc'");
        assert_eq!(shell_quote("/tmp/x"), "'/tmp/x'");
    }

    #[test]
    fn sanitize_strips_unsafe_chars() {
        assert_eq!(sanitize("hello world", &['_', '-']), "hello_world");
        assert_eq!(sanitize("foo.bar", &['_', '-', '.']), "foo.bar");
        assert_eq!(sanitize(r"a/b\c", &['_', '-']), "a_b_c");
    }
}
