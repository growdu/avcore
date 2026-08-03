//! 后台 daemon runtime
//!
//! 详见 docs/superpowers/specs/2026-08-03-provider-daemon-design.md §4-5
//!
//! T13 adds pidfile/lockfile helpers. T14-T18 add HTTP server, ping loop, and CLI.

use std::path::PathBuf;

use crate::error::{AvcError, AvcResult};

/// Returns the pidfile path: `<data_dir>/avc.pid`
pub fn pid_path() -> AvcResult<PathBuf> {
    let dir = data_dir()?;
    Ok(dir.join("avc.pid"))
}

/// Returns the log file path: `<data_dir>/avc.log`
pub fn log_path() -> AvcResult<PathBuf> {
    let dir = data_dir()?;
    Ok(dir.join("avc.log"))
}

fn data_dir() -> AvcResult<PathBuf> {
    let dir = dirs::data_dir()
        .ok_or_else(|| AvcError::Internal("no data_dir".into()))?
        .join("avc");
    Ok(dir)
}

/// Write the current process pid to the pidfile
pub fn write_pid(pid: u32) -> AvcResult<()> {
    let p = pid_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(p, pid.to_string())?;
    Ok(())
}

/// Read the pidfile; returns None if absent, or `Err(PidfileStale)` if contents are invalid
pub fn read_pid() -> AvcResult<Option<u32>> {
    let p = pid_path()?;
    if !p.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(&p)?;
    let pid = s.trim().parse::<u32>().ok();
    if pid.is_none() {
        return Err(AvcError::PidfileStale(p.display().to_string()));
    }
    Ok(pid)
}

/// Delete the pidfile (no-op if it doesn't exist)
pub fn clear_pid() -> AvcResult<()> {
    let p = pid_path()?;
    if p.exists() {
        std::fs::remove_file(&p)?;
    }
    Ok(())
}

/// Check if a process with the given pid is alive
#[cfg(target_os = "linux")]
pub fn is_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(not(target_os = "linux"))]
pub fn is_alive(_pid: u32) -> bool {
    // macOS / Windows: no portable check without adding dependencies.
    // T18 (start command) handles this case by checking exit status.
    // For v1, we accept "true" (assume alive if pidfile exists).
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: override the data dir for tests by setting XDG_DATA_HOME
    fn with_temp_data_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        std::env::set_var("XDG_DATA_HOME", dir.path());
        dir
    }

    #[test]
    fn pid_round_trip() {
        let _dir = with_temp_data_dir();
        // data_dir() might not honor XDG_DATA_HOME on all systems, so use write_pid's return value
        // to determine the actual path
        let p = pid_path().unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        write_pid(12345).unwrap();
        assert_eq!(read_pid().unwrap(), Some(12345));
        clear_pid().unwrap();
        assert_eq!(read_pid().unwrap(), None);
    }
}
