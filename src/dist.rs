use std::path::{Path, PathBuf};

use crate::error::CatClawError;

const GITHUB_REPO: &str = "CatiesGames/catclaw";

/// Detect current platform as (os, arch) for binary naming.
fn current_platform() -> (&'static str, &'static str) {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "unknown"
    };
    (os, arch)
}

/// Binary asset name for the current platform.
fn binary_name() -> String {
    let (os, arch) = current_platform();
    format!("catclaw-{}-{}", os, arch)
}

/// Fetch the latest release tag and asset URLs from GitHub.
async fn fetch_latest_release() -> Result<(String, Vec<ReleaseAsset>), CatClawError> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", GITHUB_REPO);
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("User-Agent", format!("catclaw/{}", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| CatClawError::Update(format!("failed to fetch release info: {}", e)))?;

    if !resp.status().is_success() {
        return Err(CatClawError::Update(format!(
            "GitHub API returned {}", resp.status()
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CatClawError::Update(format!("failed to parse release JSON: {}", e)))?;

    let tag = body["tag_name"]
        .as_str()
        .ok_or_else(|| CatClawError::Update("missing tag_name in release".into()))?
        .to_string();

    let assets = body["assets"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|a| {
            Some(ReleaseAsset {
                name: a["name"].as_str()?.to_string(),
                url: a["browser_download_url"].as_str()?.to_string(),
            })
        })
        .collect();

    Ok((tag, assets))
}

struct ReleaseAsset {
    name: String,
    url: String,
}

/// Fetch a specific release by tag from GitHub.
async fn fetch_release_by_tag(tag: &str) -> Result<(String, Vec<ReleaseAsset>), CatClawError> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        GITHUB_REPO, tag
    );
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("User-Agent", format!("catclaw/{}", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| CatClawError::Update(format!("failed to fetch release info: {}", e)))?;

    if !resp.status().is_success() {
        return Err(CatClawError::Update(format!(
            "release '{}' not found (GitHub API returned {})", tag, resp.status()
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CatClawError::Update(format!("failed to parse release JSON: {}", e)))?;

    let release_tag = body["tag_name"]
        .as_str()
        .ok_or_else(|| CatClawError::Update("missing tag_name in release".into()))?
        .to_string();

    let assets = body["assets"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|a| {
            Some(ReleaseAsset {
                name: a["name"].as_str()?.to_string(),
                url: a["browser_download_url"].as_str()?.to_string(),
            })
        })
        .collect();

    Ok((release_tag, assets))
}

/// Parse version from tag (strip leading 'v').
fn parse_version(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

/// Check if an update is available. Returns Some(new_version) or None.
pub async fn check_update() -> Result<Option<String>, CatClawError> {
    let (tag, _assets) = fetch_latest_release().await?;
    let remote = parse_version(&tag);
    let current = env!("CARGO_PKG_VERSION");

    if version_gt(remote, current) {
        Ok(Some(remote.to_string()))
    } else {
        Ok(None)
    }
}

/// Simple semver comparison: a > b.
fn version_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .filter_map(|p| p.parse::<u64>().ok())
            .collect()
    };
    let va = parse(a);
    let vb = parse(b);
    for i in 0..va.len().max(vb.len()) {
        let pa = va.get(i).copied().unwrap_or(0);
        let pb = vb.get(i).copied().unwrap_or(0);
        if pa > pb {
            return true;
        }
        if pa < pb {
            return false;
        }
    }
    false
}

/// Download, verify, and replace the current binary. Returns Some(new_version) or None if up to date.
/// Perform an update. If `target_version` is Some, install that specific version
/// (e.g. "v0.35.7" or "0.35.7"). If None, install the latest version.
pub async fn perform_update(target_version: Option<&str>) -> Result<Option<String>, CatClawError> {
    let (tag, assets) = if let Some(ver) = target_version {
        let tag = if ver.starts_with('v') { ver.to_string() } else { format!("v{}", ver) };
        fetch_release_by_tag(&tag).await?
    } else {
        fetch_latest_release().await?
    };
    let remote = parse_version(&tag);
    let current = env!("CARGO_PKG_VERSION");

    if target_version.is_none() && !version_gt(remote, current) {
        return Ok(None);
    }

    let bin_name = binary_name();
    let asset = assets
        .iter()
        .find(|a| a.name == bin_name)
        .ok_or_else(|| {
            CatClawError::Update(format!(
                "no binary '{}' found in release {} (available: {})",
                bin_name,
                tag,
                assets.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
            ))
        })?;

    // Find checksums file
    let checksums_asset = assets.iter().find(|a| a.name == "checksums.txt");

    // Download binary to temp file
    let current_exe = std::env::current_exe()
        .map_err(|e| CatClawError::Update(format!("cannot determine current exe: {}", e)))?;
    let temp_path = current_exe.with_extension("update");

    crate::cli_ui::status_msg("⬇️", &format!("Downloading {}...", bin_name));

    let client = reqwest::Client::new();
    let resp = client
        .get(&asset.url)
        .header("User-Agent", format!("catclaw/{}", current))
        .send()
        .await
        .map_err(|e| CatClawError::Update(format!("download failed: {}", e)))?;

    if !resp.status().is_success() {
        return Err(CatClawError::Update(format!(
            "download returned {}", resp.status()
        )));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| CatClawError::Update(format!("failed to read download: {}", e)))?;

    // Verify checksum if checksums file is available
    if let Some(cs_asset) = checksums_asset {
        crate::cli_ui::status_msg("🔐", "Verifying checksum...");
        let cs_resp = client
            .get(&cs_asset.url)
            .header("User-Agent", format!("catclaw/{}", current))
            .send()
            .await
            .map_err(|e| CatClawError::Update(format!("checksum download failed: {}", e)))?;

        let cs_text = cs_resp
            .text()
            .await
            .map_err(|e| CatClawError::Update(format!("failed to read checksums: {}", e)))?;

        let expected_hash = cs_text
            .lines()
            .find(|line| line.contains(&bin_name))
            .and_then(|line| line.split_whitespace().next());

        if let Some(expected) = expected_hash {
            use sha2::{Sha256, Digest};
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let actual = format!("{:x}", hasher.finalize());
            if actual != expected {
                // Clean up temp file if it exists
                let _ = std::fs::remove_file(&temp_path);
                return Err(CatClawError::Update(format!(
                    "checksum mismatch: expected {} got {}", expected, actual
                )));
            }
        }
    }

    // Write to temp file
    std::fs::write(&temp_path, &bytes)
        .map_err(|e| CatClawError::Update(format!("failed to write temp file: {}", e)))?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o755)) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(CatClawError::Update(format!("chmod failed: {}", e)));
        }
    }

    // Atomic rename (Unix)
    if let Err(e) = std::fs::rename(&temp_path, &current_exe) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(CatClawError::Update(format!("failed to replace binary: {}", e)));
    }

    // macOS: remove quarantine attribute.
    // Binary is codesigned by CI with Developer ID — do NOT re-sign with ad-hoc,
    // as that would strip the Developer ID signature and break TCC trust.
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("xattr")
            .args(["-d", "com.apple.quarantine"])
            .arg(&current_exe)
            .output();
    }

    Ok(Some(remote.to_string()))
}

// ── Service management ──────────────────────────────────────────────────────

/// Escape a string for safe inclusion in XML plist values.
#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// macOS launchd plist path
#[cfg(target_os = "macos")]
fn plist_path() -> PathBuf {
    dirs_plist().join("com.catclaw.gateway.plist")
}

#[cfg(target_os = "macos")]
fn dirs_plist() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join("Library/LaunchAgents")
}

/// Linux systemd user unit path
#[cfg(target_os = "linux")]
fn unit_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".config/systemd/user/catclaw.service")
}

/// Result of comparing the installed service unit against what
/// `service_install` would produce. Used by both the install path (to skip
/// no-op reinstalls) and the gateway start path (to warn on drift).
///
/// `NotInstalled` and `Drifted` are only ever produced on Linux — the
/// macOS / Windows path of `unit_sync_state` returns `InSync` unconditionally
/// because we don't do plist drift detection (XML whitespace would make it
/// fragile). The variants are still part of the cross-platform API so call
/// sites in `gateway::start` don't need `#[cfg]` arms of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // macOS/Windows builds never construct the non-InSync variants
pub enum UnitSyncState {
    /// No unit file on disk — service was never installed.
    NotInstalled,
    /// Installed unit matches byte-for-byte what we'd write now.
    InSync,
    /// Installed unit differs (out-of-date binary path, stale memory limits,
    /// missing watchdog, etc). Run `catclaw gateway install` to refresh.
    Drifted,
}

/// Compare the installed unit (if any) against what `service_install` would
/// write right now. Best-effort: returns `NotInstalled` when no unit on disk,
/// `InSync` when contents match exactly, `Drifted` otherwise. Errors reading
/// either side surface as `Drifted` (conservative — reinstall is safe).
///
/// Linux only. macOS plist comparison would need XML-aware diff (whitespace
/// noise) which isn't worth the complexity for the launchd path.
#[cfg(target_os = "linux")]
pub fn unit_sync_state(config_path: &Path) -> UnitSyncState {
    let unit = unit_path();
    if !unit.exists() {
        return UnitSyncState::NotInstalled;
    }
    let exe = match std::env::current_exe() {
        Ok(p) => std::fs::canonicalize(&p).unwrap_or(p),
        Err(_) => return UnitSyncState::Drifted,
    };
    let config_abs = std::fs::canonicalize(config_path)
        .unwrap_or_else(|_| config_path.to_path_buf());
    let expected = build_systemd_unit(&exe, &config_abs);
    match std::fs::read_to_string(&unit) {
        Ok(actual) if actual == expected => UnitSyncState::InSync,
        _ => UnitSyncState::Drifted,
    }
}

#[cfg(not(target_os = "linux"))]
pub fn unit_sync_state(_config_path: &Path) -> UnitSyncState {
    // macOS / Windows: not implemented (see comment on Linux variant).
    UnitSyncState::InSync
}

/// Check if the service is installed.
pub fn is_service_installed() -> bool {
    #[cfg(target_os = "macos")]
    {
        plist_path().exists()
    }
    #[cfg(target_os = "linux")]
    {
        unit_path().exists()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

/// Install and start the system service.
/// If a gateway is already running (manually started), it will be stopped first
/// to avoid port conflicts with the service-managed instance.
pub fn service_install(config_path: &Path) -> Result<(), CatClawError> {
    // Drift check (Linux): if the installed unit byte-matches what we'd
    // write, this is a no-op. Skip the stop-gateway + uninstall + reinstall
    // + restart dance entirely. Lets deploy scripts call `catclaw gateway
    // install` unconditionally on every release without disrupting a
    // running service when nothing changed.
    //
    // The check is byte-for-byte against `build_systemd_unit`. Anything
    // that varies between deploys (binary path, memory limits, watchdog
    // timer) lives there — so a real drift always trips a reinstall.
    #[cfg(target_os = "linux")]
    if matches!(unit_sync_state(config_path), UnitSyncState::InSync) {
        crate::cli_ui::status_msg("✅", "Service unit already in sync (no changes needed)");
        return Ok(());
    }

    // Stop any manually-started gateway to avoid port conflict
    let config = crate::config::Config::load(config_path).ok();
    let pid_path = crate::pidfile::pid_path(config.as_ref());
    if let Some(pid) = crate::pidfile::read_pid(&pid_path) {
        if crate::pidfile::is_running(pid) {
            crate::cli_ui::status_msg("⏳", &format!("Stopping running gateway (PID {}) before service install...", pid));
            crate::pidfile::stop_process(pid);
            // Wait for it to stop
            for _ in 0..30 {
                if !crate::pidfile::is_running(pid) { break; }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            crate::pidfile::remove_pid(&pid_path);
        }
    }

    // If service is already installed, unload first (idempotent reinstall)
    if is_service_installed() {
        crate::cli_ui::status_msg("🔄", "Reinstalling service (unit drift detected)...");
        let _ = service_uninstall();
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    let exe = std::env::current_exe()
        .map_err(|e| CatClawError::Service(format!("cannot determine exe path: {}", e)))?;
    let exe = std::fs::canonicalize(&exe)
        .unwrap_or(exe);
    let config_abs = std::fs::canonicalize(config_path)
        .unwrap_or_else(|_| config_path.to_path_buf());

    #[cfg(target_os = "macos")]
    {
        service_install_macos(&exe, &config_abs)
    }
    #[cfg(target_os = "linux")]
    {
        service_install_linux(&exe, &config_abs)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(CatClawError::Service("unsupported platform".into()))
    }
}

#[cfg(target_os = "macos")]
fn service_install_macos(exe: &Path, config_path: &Path) -> Result<(), CatClawError> {
    let plist = plist_path();
    let plist_dir = dirs_plist();
    std::fs::create_dir_all(&plist_dir)
        .map_err(|e| CatClawError::Service(format!("failed to create LaunchAgents dir: {}", e)))?;

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let log_out = format!("{}/Library/Logs/catclaw.log", home);
    let log_err = format!("{}/Library/Logs/catclaw.error.log", home);

    let exe_s = xml_escape(&exe.display().to_string());
    let config_s = xml_escape(&config_path.display().to_string());
    let log_out_s = xml_escape(&log_out);
    let log_err_s = xml_escape(&log_err);
    let home_s = xml_escape(&home);

    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.catclaw.gateway</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe_s}</string>
        <string>--config</string>
        <string>{config_s}</string>
        <string>gateway</string>
        <string>start</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{log_out_s}</string>
    <key>StandardErrorPath</key>
    <string>{log_err_s}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin:{home_s}/.local/bin</string>
    </dict>
</dict>
</plist>"#,
    );

    std::fs::write(&plist, content)
        .map_err(|e| CatClawError::Service(format!("failed to write plist: {}", e)))?;

    // Load the service using modern launchctl API (bootstrap instead of load -w)
    let uid = unsafe { libc::getuid() };
    let domain_target = format!("gui/{}", uid);
    let status = std::process::Command::new("launchctl")
        .args(["bootstrap", &domain_target])
        .arg(&plist)
        .status()
        .map_err(|e| CatClawError::Service(format!("launchctl bootstrap failed: {}", e)))?;

    if !status.success() {
        // Fallback to legacy load for older macOS
        let status = std::process::Command::new("launchctl")
            .args(["load", "-w"])
            .arg(&plist)
            .status()
            .map_err(|e| CatClawError::Service(format!("launchctl load failed: {}", e)))?;
        if !status.success() {
            return Err(CatClawError::Service("launchctl load returned non-zero".into()));
        }
    }

    Ok(())
}

/// Render the systemd user unit file for the given binary + config path.
/// Pure function — same inputs always produce the same bytes, so callers can
/// hash-compare the result against the installed unit to detect drift.
///
/// Anything that varies between deploys (memory limits, watchdog timer,
/// timeout, env vars) must live here, not be patched on after the fact —
/// otherwise drift detection sees false positives every time.
#[cfg(target_os = "linux")]
fn build_systemd_unit(exe: &Path, config_path: &Path) -> String {
    // Type=notify + watchdog: the gateway sends sd_notify(READY=1) once it's up
    // and sd_notify(WATCHDOG=1) every ~45s. If the tokio runtime freezes (e.g.
    // a deadlock or the box gets dragged into swap thrash), the watchdog stops
    // firing and systemd restarts the unit instead of leaving it stuck for ~45
    // minutes (prod incident 2026-05-13).
    //
    // MemoryHigh/MemoryMax: soft+hard memory caps for the gateway's cgroup
    // (which includes spawned `claude` subprocesses). When a runaway subprocess
    // blows past MemoryMax, the cgroup OOM-killer kills the biggest offender
    // (usually that subprocess; sometimes the gateway, which systemd restarts
    // in ~10s) — far better than the whole VM thrashing.
    //
    // Budget (assumes host has ≥8 GiB RAM):
    //   BGE-M3 owned bytes  ~2.3 GiB  (loaded as anon, see src/memory/embed.rs)
    //   catclaw heap+stack  ~1.5 GiB
    //   one claude session  ~1-2 GiB
    //   slack for spikes    ~1 GiB
    //                      ─────────
    //                       ~6 GiB → MemoryMax=6G, MemoryHigh=5G
    //
    // Before BGE-M3 was loaded as owned bytes the model sat in page cache and
    // wasn't billed to the cgroup at all — limits used to be 3G/4G. Owned
    // bytes are anon memory, which IS billed. Lower limits will trip the
    // cgroup OOM the moment the embedder finishes initialising.
    //
    // Tune via `systemctl --user edit catclaw` if your host has less than
    // 8 GiB (in which case BGE-M3 is not really feasible — consider switching
    // embedding model).
    //
    // NOTE: memory accounting in *user* units needs cgroup delegation
    // (systemd ≥ v244 with `DefaultMemoryAccounting=yes`, true on most modern
    // distros). If unavailable systemd silently ignores these directives.
    //
    // TimeoutStartSec=300: first run downloads the ~560 MB BGE-M3 model
    // synchronously before READY=1; give it room on slow links.
    format!(
        r#"[Unit]
Description=CatClaw AI Gateway
After=network-online.target
Wants=network-online.target

[Service]
Type=notify
ExecStart="{exe}" --config "{config}" gateway start
Restart=always
RestartSec=5
TimeoutStartSec=300
WatchdogSec=120
MemoryHigh=5G
MemoryMax=6G
Environment=PATH=/usr/local/bin:/usr/bin:/bin:%h/.local/bin

[Install]
WantedBy=default.target
"#,
        exe = exe.display(),
        config = config_path.display(),
    )
}

#[cfg(target_os = "linux")]
fn service_install_linux(exe: &Path, config_path: &Path) -> Result<(), CatClawError> {
    let unit = unit_path();
    if let Some(parent) = unit.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CatClawError::Service(format!("failed to create systemd user dir: {}", e)))?;
    }

    let content = build_systemd_unit(exe, config_path);

    std::fs::write(&unit, content)
        .map_err(|e| CatClawError::Service(format!("failed to write unit file: {}", e)))?;

    // Reload, enable, start
    let run = |args: &[&str]| -> Result<(), CatClawError> {
        let status = std::process::Command::new("systemctl")
            .args(args)
            .status()
            .map_err(|e| CatClawError::Service(format!("systemctl {} failed: {}", args.join(" "), e)))?;
        if !status.success() {
            return Err(CatClawError::Service(format!("systemctl {} returned non-zero", args.join(" "))));
        }
        Ok(())
    };

    run(&["--user", "daemon-reload"])?;
    run(&["--user", "enable", "catclaw"])?;
    run(&["--user", "start", "catclaw"])?;

    // Enable linger so user services survive SSH logout
    let user = std::env::var("USER").unwrap_or_default();
    if !user.is_empty() {
        let _ = std::process::Command::new("loginctl")
            .args(["enable-linger", &user])
            .status();
    }

    Ok(())
}

/// Uninstall the system service.
pub fn service_uninstall() -> Result<(), CatClawError> {
    #[cfg(target_os = "macos")]
    {
        let plist = plist_path();
        // Use modern bootout API to fully remove from launchd (no residual override records)
        let uid = unsafe { libc::getuid() };
        let service_target = format!("gui/{}/com.catclaw.gateway", uid);
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &service_target])
            .status();
        // Remove plist file
        if plist.exists() {
            std::fs::remove_file(&plist)
                .map_err(|e| CatClawError::Service(format!("failed to remove plist: {}", e)))?;
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let unit = unit_path();
        if unit.exists() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "stop", "catclaw"])
                .status();
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "catclaw"])
                .status();
            std::fs::remove_file(&unit)
                .map_err(|e| CatClawError::Service(format!("failed to remove unit file: {}", e)))?;
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status();
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(CatClawError::Service("unsupported platform".into()))
    }
}

/// Show service status.
pub fn service_status() -> Result<String, CatClawError> {
    #[cfg(target_os = "macos")]
    {
        let plist = plist_path();
        if !plist.exists() {
            return Ok("Service not installed".to_string());
        }
        let output = std::process::Command::new("launchctl")
            .args(["list", "com.catclaw.gateway"])
            .output()
            .map_err(|e| CatClawError::Service(format!("launchctl list failed: {}", e)))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Parse PID from launchctl output
            let pid_line = stdout.lines().find(|l| l.contains("PID"));
            if let Some(line) = pid_line {
                Ok(format!("🟢 Service running ({})", line.trim()))
            } else {
                // launchctl list succeeded — service is registered
                // Check if there's a non-zero exit status
                let last_exit = stdout.lines()
                    .find(|l| l.contains("\"LastExitStatus\""))
                    .and_then(|l| l.split('=').nth(1))
                    .map(|s| s.trim().trim_matches(';'))
                    .unwrap_or("0");
                if last_exit == "0" {
                    Ok("🟡 Service installed, not currently running".to_string())
                } else {
                    Ok(format!("🔴 Service installed, last exit status: {}", last_exit))
                }
            }
        } else {
            Ok("🔴 Service not running (registered but not loaded)".to_string())
        }
    }
    #[cfg(target_os = "linux")]
    {
        let unit = unit_path();
        if !unit.exists() {
            return Ok("Service not installed".to_string());
        }

        let active = std::process::Command::new("systemctl")
            .args(["--user", "is-active", "catclaw"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let enabled = std::process::Command::new("systemctl")
            .args(["--user", "is-enabled", "catclaw"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let emoji = if active == "active" { "🟢" } else { "🔴" };
        Ok(format!("{} Service: {} (enabled: {})", emoji, active, enabled))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(CatClawError::Service("unsupported platform".into()))
    }
}

/// Restart an already-installed service.
pub fn restart_service() -> Result<(), CatClawError> {
    #[cfg(target_os = "macos")]
    {
        let plist = plist_path();
        if !plist.exists() {
            return Err(CatClawError::Service("service not installed".into()));
        }
        let uid = unsafe { libc::getuid() };
        let service_target = format!("gui/{}/com.catclaw.gateway", uid);
        let domain_target = format!("gui/{}", uid);
        // bootout (stop + unregister)
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &service_target])
            .status();
        std::thread::sleep(std::time::Duration::from_millis(500));
        // bootstrap (register + start)
        let status = std::process::Command::new("launchctl")
            .args(["bootstrap", &domain_target])
            .arg(&plist)
            .status()
            .map_err(|e| CatClawError::Service(format!("launchctl bootstrap failed: {}", e)))?;
        if !status.success() {
            return Err(CatClawError::Service("launchctl bootstrap returned non-zero".into()));
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("systemctl")
            .args(["--user", "restart", "catclaw"])
            .status()
            .map_err(|e| CatClawError::Service(format!("systemctl restart failed: {}", e)))?;
        if !status.success() {
            return Err(CatClawError::Service("systemctl restart returned non-zero".into()));
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(CatClawError::Service("unsupported platform".into()))
    }
}

// ── Pending resume (survives gateway restart) ──────────────────
//
// When the agent invokes `catclaw gateway restart --resume` (or
// `catclaw update --resume`), CatClaw records the session it should
// auto-resume after coming back up. The gateway reads this file at
// startup, resumes the session, and injects a continuation prompt so
// the agent silently picks up where it left off. No channel
// notification is sent — the agent's next response IS the signal that
// it is back online.

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PendingResume {
    /// `catclaw:{agent_id}:{origin}:{context_id}` — keys the session in state.db.
    pub session_key: String,
    /// "restart" or "update" — used to phrase the continuation prompt.
    pub kind: String,
    /// Optional version string (set by `update --resume`).
    pub version: Option<String>,
    pub created_at: String,
}

fn pending_resume_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".catclaw").join("pending_resume.json")
}

/// Queue an auto-resume for the next gateway startup.
pub fn write_pending_resume(
    session_key: &str,
    kind: &str,
    version: Option<&str>,
) -> Result<(), CatClawError> {
    let resume = PendingResume {
        session_key: session_key.to_string(),
        kind: kind.to_string(),
        version: version.map(|s| s.to_string()),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let json = serde_json::to_string_pretty(&resume)
        .map_err(|e| CatClawError::Config(format!("serialize pending_resume: {}", e)))?;
    if let Some(parent) = pending_resume_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(pending_resume_path(), json)?;
    Ok(())
}

/// Read and delete the pending resume file. Returns None if no file exists or it is stale.
pub fn read_and_clear_pending_resume() -> Option<PendingResume> {
    let path = pending_resume_path();
    let data = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let resume: PendingResume = serde_json::from_str(&data).ok()?;
    // Skip if older than 10 minutes (stale from a failed restart)
    if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&resume.created_at) {
        if chrono::Utc::now().signed_duration_since(created).num_seconds() > 600 {
            tracing::warn!("stale pending resume (>10min), discarding");
            return None;
        }
    }
    Some(resume)
}

/// Interactive uninstall command.
pub async fn cmd_uninstall(config_path: &Path) {
    use crate::{cli_ui, config::Config, pidfile};

    println!();
    cli_ui::status_msg("🗑️", "CatClaw Uninstall");
    println!();

    // 1. Stop gateway if running
    if let Ok(config) = Config::load(config_path) {
        let pid_path = pidfile::pid_path(Some(&config));
        if let Some(pid) = pidfile::read_pid(&pid_path) {
            if pidfile::is_running(pid) {
                cli_ui::status_msg("⏳", "Stopping gateway...");
                pidfile::stop_process(pid);
                for _ in 0..20 {
                    if !pidfile::is_running(pid) { break; }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                pidfile::remove_pid(&pid_path);
                cli_ui::status_msg("✅", "Gateway stopped");
            }
        }
    }

    // 2. Remove service if installed
    if is_service_installed() {
        cli_ui::status_msg("⏳", "Removing service...");
        match service_uninstall() {
            Ok(()) => cli_ui::status_msg("✅", "Service removed"),
            Err(e) => cli_ui::status_msg("⚠️", &format!("Service removal failed: {}", e)),
        }
    }

    // 3. Remove binary
    if let Ok(exe) = std::env::current_exe() {
        let exe_str = exe.display().to_string();
        println!();
        cli_ui::status_msg("📍", &format!("Binary: {}", exe_str));

        // On Unix, a running binary can be unlinked (will be deleted after process exits)
        #[cfg(unix)]
        {
            if cli_ui::section_confirm("Remove binary?", false) {
                match std::fs::remove_file(&exe) {
                    Ok(()) => cli_ui::status_msg("✅", "Binary removed (will take effect after this process exits)"),
                    Err(e) => cli_ui::status_msg("⚠️", &format!("Failed to remove binary: {}", e)),
                }
            }
        }
    }

    // 4. Workspace
    println!();
    let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
    let catclaw_dir = format!("{}/.catclaw", home);
    cli_ui::status_msg("ℹ️", "Config and workspace files are preserved.");
    cli_ui::status_msg("ℹ️", &format!("To remove everything: rm -rf {}", catclaw_dir));
    println!();
}

// ── systemd sd_notify protocol ────────────────────────────────────────────
//
// Minimal hand-rolled implementation of the sd_notify(3) protocol (no crate
// dependency). The unit is `Type=notify` with `WatchdogSec`, so the gateway
// must send `READY=1` once it's up and `WATCHDOG=1` periodically. When
// `NOTIFY_SOCKET` isn't set (not run under systemd, or older unit) every call
// is a silent no-op, so it's safe to call unconditionally.

/// True if running under a systemd `Type=notify` unit (NOTIFY_SOCKET present).
pub fn under_systemd_notify() -> bool {
    std::env::var_os("NOTIFY_SOCKET").is_some()
}

/// Send a raw sd_notify message (e.g. "READY=1", "WATCHDOG=1"). No-op when
/// NOTIFY_SOCKET is unset. Errors are swallowed — notify is best-effort and
/// must never take the gateway down.
pub fn sd_notify(msg: &str) {
    let Some(path) = std::env::var_os("NOTIFY_SOCKET") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    // systemd uses an abstract namespace socket when the path starts with '@';
    // std::os::unix::net::UnixDatagram doesn't speak abstract sockets, so for
    // the (rare) abstract case we just skip — the common case is a real path
    // like /run/user/<uid>/systemd/notify.
    if path.as_os_str().to_string_lossy().starts_with('@') {
        return;
    }
    if let Ok(sock) = std::os::unix::net::UnixDatagram::unbound() {
        let _ = sock.send_to(msg.as_bytes(), &path);
    }
}

/// Convenience: emit `READY=1`.
pub fn notify_ready() {
    sd_notify("READY=1");
}

/// Convenience: emit `WATCHDOG=1`.
pub fn notify_watchdog() {
    sd_notify("WATCHDOG=1");
}
