//! avc daemon <verb> 后台进程管理

use std::process::Command;

use crate::error::{AvcError, AvcResult};
use crate::svc::daemon::{clear_pid, is_alive, log_path, read_pid, write_pid};

pub fn dispatch(args: &[String]) -> AvcResult<()> {
    if args.is_empty() {
        return cmd_help();
    }
    match args[0].as_str() {
        "start" => cmd_start(&args[1..]),
        "stop" => cmd_stop(),
        "status" => cmd_status(),
        "logs" => cmd_logs(),
        _ => Err(AvcError::Arg(format!("unknown daemon verb: {}", args[0]))),
    }
}

fn cmd_help() -> AvcResult<()> {
    println!(
        "avc daemon <verb>
verbs:
  start    fork child process running ping loop + HTTP
  stop     send SIGTERM and clear pidfile
  status   show pid, alive, started_at, port
  logs     tail ~/.local/share/avc/avc.log"
    );
    Ok(())
}

fn cmd_start(_args: &[String]) -> AvcResult<()> {
    // 检查已有 daemon
    if let Some(p) = read_pid()? {
        if is_alive(p) {
            return Err(AvcError::AlreadyRunning { pid: p, port: 7891 });
        }
        // 僵死：清 pidfile，继续
        eprintln!("removing stale pidfile for pid {}", p);
        clear_pid()?;
    }
    // fork 子进程跑 _run
    let exe = std::env::current_exe()?;
    let child = Command::new(exe)
        .arg("_run")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    write_pid(child.id())?;
    println!("avc daemon started: pid {}", child.id());
    Ok(())
}

fn cmd_stop() -> AvcResult<()> {
    let pid = match read_pid()? {
        Some(p) => p,
        None => {
            println!("daemon not running");
            return Ok(());
        }
    };
    if !is_alive(pid) {
        println!("daemon not running (stale pid {})", pid);
        clear_pid()?;
        return Ok(());
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(pid, SIGTERM) is the standard graceful-shutdown signal.
        // On non-Unix, we don't have a portable signal; return an error.
        // (v1 doesn't support Windows daemon stop.)
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
        println!("sent SIGTERM to pid {}", pid);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        Err(AvcError::Internal(
            "daemon stop not supported on non-Unix yet".into(),
        ))
    }
}

fn cmd_status() -> AvcResult<()> {
    let pid = match read_pid()? {
        Some(p) => p,
        None => {
            println!("daemon not running");
            return Ok(());
        }
    };
    let alive = is_alive(pid);
    let log = log_path()?.display().to_string();
    println!("pid:   {}\nalive: {}\nlog:   {}", pid, alive, log);
    Ok(())
}

fn cmd_logs() -> AvcResult<()> {
    let p = log_path()?;
    if !p.exists() {
        println!("no log file at {}", p.display());
        return Ok(());
    }
    let s = std::fs::read_to_string(&p)?;
    // tail：取最后 2000 字符
    let tail: String = s
        .chars()
        .rev()
        .take(2000)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    print!("{}", tail);
    Ok(())
}
