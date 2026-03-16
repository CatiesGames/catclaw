use std::io;
use std::path::{Path, PathBuf};

use crate::config::Config;

/// Determine PID file path: ~/.catclaw/catclaw.pid (or workspace/catclaw.pid as fallback).
pub fn pid_path(config: Option<&Config>) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let catclaw_home = std::path::PathBuf::from(home).join(".catclaw");
    if catclaw_home.exists() {
        catclaw_home.join("catclaw.pid")
    } else if let Some(cfg) = config {
        cfg.general.workspace.join("catclaw.pid")
    } else {
        catclaw_home.join("catclaw.pid")
    }
}

/// Write PID to file
pub fn write_pid(path: &Path, pid: u32) -> io::Result<()> {
    std::fs::write(path, pid.to_string())
}

/// Read PID from file, returns None if missing or invalid
pub fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Remove PID file
pub fn remove_pid(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Check if a process with the given PID is running
pub fn is_running(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Stop a process by PID (SIGTERM)
pub fn stop_process(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
