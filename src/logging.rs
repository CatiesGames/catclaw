use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

/// Global handle for hot-reloading the log filter level at runtime.
static RELOAD_HANDLE: OnceLock<ReloadHandle> = OnceLock::new();

type ReloadHandle = tracing_subscriber::reload::Handle<EnvFilter, tracing_subscriber::Registry>;

/// A single structured log entry (JSONL format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRecord {
    /// ISO 8601 timestamp
    pub ts: String,
    /// Log level: error, warn, info, debug
    pub level: String,
    /// Subsystem that produced the log (e.g. "gateway", "router", "discord")
    pub target: String,
    /// Human-readable message
    pub msg: String,
    /// Optional structured fields
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub fields: serde_json::Map<String, serde_json::Value>,
}

/// Writer that appends JSONL to daily-rotated log files.
///
/// File naming: `catclaw-YYYY-MM-DD.jsonl`
/// Rotation: Automatically switches file at midnight (local time).
#[derive(Clone)]
pub struct DailyFileWriter {
    log_dir: PathBuf,
}

impl DailyFileWriter {
    pub fn new(log_dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(log_dir)?;
        Ok(DailyFileWriter {
            log_dir: log_dir.to_path_buf(),
        })
    }

    fn current_path(&self) -> PathBuf {
        let date = Local::now().format("%Y-%m-%d");
        self.log_dir.join(format!("catclaw-{}.jsonl", date))
    }
}

impl<'a> MakeWriter<'a> for DailyFileWriter {
    type Writer = DailyFileHandle;

    fn make_writer(&'a self) -> Self::Writer {
        let path = self.current_path();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("failed to open log file");
        DailyFileHandle { file }
    }
}

pub struct DailyFileHandle {
    file: std::fs::File,
}

impl Write for DailyFileHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Initialize the tracing subscriber with dual output:
/// - Console: human-readable (compact format)
/// - File: JSON lines to daily-rotated file
///
/// The log filter is wrapped in a reload layer so that `set_log_level()`
/// can change it at runtime without restarting.
pub fn init_logging(log_dir: &Path, level: &str) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let file_writer =
        DailyFileWriter::new(log_dir).expect("failed to create log directory");

    // File layer: JSON format
    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(file_writer)
        .with_target(true)
        .with_timer(tracing_subscriber::fmt::time::SystemTime)
        .with_ansi(false);

    // Console layer: compact human-readable
    let console_layer = tracing_subscriber::fmt::layer()
        .compact()
        .with_target(true)
        .with_writer(io::stderr);

    // Build the filter from RUST_LOG env or config level, wrapped in reload layer
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| build_filter(level));
    let (filter_layer, reload_handle) = tracing_subscriber::reload::Layer::new(filter);

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(file_layer)
        .with(console_layer)
        .init();

    let _ = RELOAD_HANDLE.set(reload_handle);
}

/// Initialize logging with file output only (no console).
/// Used when TUI will take over the terminal.
///
/// The log filter is wrapped in a reload layer so that `set_log_level()`
/// can change it at runtime without restarting.
pub fn init_file_only_logging(log_dir: &Path, level: &str) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let file_writer =
        DailyFileWriter::new(log_dir).expect("failed to create log directory");

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(file_writer)
        .with_target(true)
        .with_timer(tracing_subscriber::fmt::time::SystemTime)
        .with_ansi(false);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| build_filter(level));
    let (filter_layer, reload_handle) = tracing_subscriber::reload::Layer::new(filter);

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(file_layer)
        .init();

    let _ = RELOAD_HANDLE.set(reload_handle);
}

/// Build an EnvFilter directive that applies `level` to catclaw code but suppresses
/// verbose output from noisy third-party crates (serenity, hyper, reqwest, etc.).
/// When RUST_LOG is set, it takes full precedence over this function.
fn build_filter(level: &str) -> EnvFilter {
    // At debug/trace, third-party crates produce enormous output (Discord gateway
    // frames, HTTP internals, TLS handshakes). Cap them at warn.
    let directive = match level.to_lowercase().as_str() {
        "debug" | "trace" => format!(
            "{level},serenity=warn,poise=warn,tracing=warn,hyper=warn,hyper_util=warn,\
             reqwest=warn,h2=warn,rustls=warn,tokio_tungstenite=warn,tungstenite=warn,\
             teloxide=warn,tower=warn"
        ),
        _ => level.to_string(),
    };
    EnvFilter::try_new(&directive).unwrap_or_else(|_| EnvFilter::new(level))
}

/// Change the active log level filter at runtime.
/// Only works if `init_logging` or `init_file_only_logging` was called first.
/// Returns Ok(()) on success, Err if the level string is invalid or no handle is available.
pub fn set_log_level(level: &str) -> std::result::Result<(), String> {
    let handle = RELOAD_HANDLE
        .get()
        .ok_or_else(|| "logging not initialized with reload support".to_string())?;
    let new_filter = EnvFilter::try_new(level)
        .map(|_| build_filter(level))
        .map_err(|e| format!("invalid log level '{}': {}", level, e))?;
    handle
        .reload(new_filter)
        .map_err(|e| format!("failed to reload log filter: {}", e))
}

/// Initialize logging with console output only (no file).
/// Used for CLI commands that don't need file logging.
pub fn init_console_logging() {
    use tracing_subscriber::EnvFilter;

    tracing_subscriber::fmt()
        .compact()
        .with_target(false)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();
}

// ── Log file reading utilities (for CLI `catclaw logs` and TUI) ──

/// List available log files in the log directory, sorted newest first.
pub fn list_log_files(log_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(log_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .map(|ext| ext == "jsonl")
                .unwrap_or(false)
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("catclaw-"))
                    .unwrap_or(false)
        })
        .collect();
    files.sort_by(|a, b| b.cmp(a)); // newest first
    files
}

/// Get today's log file path.
pub fn today_log_path(log_dir: &Path) -> PathBuf {
    let date = Local::now().format("%Y-%m-%d");
    log_dir.join(format!("catclaw-{}.jsonl", date))
}

/// Extract the date from a log file name.
#[allow(dead_code)]
pub fn log_file_date(path: &Path) -> Option<NaiveDate> {
    let stem = path.file_stem()?.to_str()?;
    let date_str = stem.strip_prefix("catclaw-")?;
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()
}

/// Read and parse log records from a JSONL file.
/// Returns records in chronological order.
pub fn read_log_file(path: &Path) -> Vec<LogRecord> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let reader = BufReader::new(file);
    reader
        .lines()
        .filter_map(|line| {
            let line = line.ok()?;
            if line.trim().is_empty() {
                return None;
            }
            parse_json_log_line(&line)
        })
        .collect()
}

/// Parse a single JSON log line from tracing-subscriber's JSON format.
/// Handles both our structured format and tracing-subscriber's default format.
fn parse_json_log_line(line: &str) -> Option<LogRecord> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let obj = v.as_object()?;

    // tracing-subscriber JSON format uses "timestamp", "level", "target", "fields.message"
    let ts = obj
        .get("timestamp")
        .or_else(|| obj.get("ts"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let level = obj
        .get("level")
        .and_then(|v| v.as_str())
        .unwrap_or("INFO")
        .to_uppercase();

    let target = obj
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // tracing-subscriber puts the message inside "fields.message"
    let msg = obj
        .get("fields")
        .and_then(|f| f.get("message"))
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("msg").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    // Collect extra fields (from tracing structured fields)
    let mut fields = serde_json::Map::new();
    if let Some(f) = obj.get("fields").and_then(|v| v.as_object()) {
        for (k, v) in f {
            if k != "message" {
                fields.insert(k.clone(), v.clone());
            }
        }
    }
    // Also collect span info if present
    if let Some(span) = obj.get("span") {
        fields.insert("span".to_string(), span.clone());
    }

    Some(LogRecord {
        ts,
        level,
        target,
        msg,
        fields,
    })
}

/// Filter records by level (returns records at or above the given level).
pub fn filter_by_level<'a>(records: &'a [LogRecord], min_level: &str) -> Vec<&'a LogRecord> {
    let min = level_priority(min_level);
    records
        .iter()
        .filter(|r| level_priority(&r.level) >= min)
        .collect()
}

/// Filter records by time range.
pub fn filter_by_time<'a>(
    records: &'a [LogRecord],
    since: Option<&str>,
    until: Option<&str>,
) -> Vec<&'a LogRecord> {
    records
        .iter()
        .filter(|r| {
            if let Some(since) = since {
                if r.ts.as_str() < since {
                    return false;
                }
            }
            if let Some(until) = until {
                if r.ts.as_str() > until {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Filter records by regex pattern on message + target.
pub fn filter_by_grep<'a>(records: &'a [LogRecord], pattern: &str) -> Vec<&'a LogRecord> {
    // Use simple substring match if not valid regex
    if let Ok(re) = regex::Regex::new(pattern) {
        records
            .iter()
            .filter(|r| re.is_match(&r.msg) || re.is_match(&r.target))
            .collect()
    } else {
        let pat = pattern.to_lowercase();
        records
            .iter()
            .filter(|r| {
                r.msg.to_lowercase().contains(&pat) || r.target.to_lowercase().contains(&pat)
            })
            .collect()
    }
}

fn level_priority(level: &str) -> u8 {
    match level.to_uppercase().as_str() {
        "ERROR" => 4,
        "WARN" => 3,
        "INFO" => 2,
        "DEBUG" => 1,
        "TRACE" => 0,
        _ => 2,
    }
}

/// Format a LogRecord for human-readable console output.
pub fn format_record(record: &LogRecord, use_color: bool) -> String {
    let time = extract_time_from_iso(&record.ts);
    let level_display = format!("{:<5}", record.level);
    let target_short = shorten_target(&record.target);

    let extra: String = if record.fields.is_empty() {
        String::new()
    } else {
        let pairs: Vec<String> = record
            .fields
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        format!(" {}", pairs.join(" "))
    };

    if use_color {
        let level_color = match record.level.as_str() {
            "ERROR" => "\x1b[31m", // red
            "WARN" => "\x1b[33m",  // yellow
            "INFO" => "\x1b[32m",  // green
            "DEBUG" => "\x1b[36m", // cyan
            _ => "\x1b[0m",
        };
        let reset = "\x1b[0m";
        let dim = "\x1b[2m";

        format!(
            "{dim}{time}{reset} {color}{level}{reset} {dim}{target}{reset} {msg}{extra}",
            dim = dim,
            time = time,
            reset = reset,
            color = level_color,
            level = level_display,
            target = target_short,
            msg = record.msg,
            extra = extra,
        )
    } else {
        format!(
            "{} {} {} {}{}",
            time, level_display, target_short, record.msg, extra
        )
    }
}

fn extract_time_from_iso(ts: &str) -> &str {
    // Extract HH:MM:SS from "2026-03-10T12:34:56.789Z" or similar
    if let Some(t_pos) = ts.find('T') {
        let after = &ts[t_pos + 1..];
        if after.len() >= 8 {
            &after[..8]
        } else {
            after
        }
    } else {
        ts
    }
}

fn shorten_target(target: &str) -> String {
    // "catclaw::gateway" → "[gateway]"
    // "catclaw::session::manager" → "[session::manager]"
    if let Some(stripped) = target.strip_prefix("catclaw::") {
        format!("[{}]", stripped)
    } else if target.is_empty() {
        String::new()
    } else {
        format!("[{}]", target)
    }
}

/// Tail a log file, printing new lines as they appear (like `tail -f`).
/// Blocks until interrupted.
pub fn tail_follow(
    log_dir: &Path,
    min_level: &str,
    grep: Option<&str>,
    use_color: bool,
) -> io::Result<()> {
    use std::thread;
    use std::time::Duration;

    let mut current_date = Local::now().date_naive();
    let mut current_path = today_log_path(log_dir);
    let mut offset: u64 = if current_path.exists() {
        fs::metadata(&current_path)?.len()
    } else {
        0
    };

    loop {
        // Check for date rollover
        let now_date = Local::now().date_naive();
        if now_date != current_date {
            current_date = now_date;
            current_path = today_log_path(log_dir);
            offset = 0;
        }

        if current_path.exists() {
            let file_len = fs::metadata(&current_path)?.len();
            if file_len > offset {
                let file = fs::File::open(&current_path)?;
                let mut reader = BufReader::new(file);
                // Seek to offset
                io::Seek::seek(&mut reader, io::SeekFrom::Start(offset))?;

                let mut new_offset = offset;
                for line in reader.lines() {
                    let line = line?;
                    new_offset += line.len() as u64 + 1; // +1 for newline
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Some(record) = parse_json_log_line(&line) {
                        if level_priority(&record.level) < level_priority(min_level) {
                            continue;
                        }
                        if let Some(pattern) = grep {
                            let matches = if let Ok(re) = regex::Regex::new(pattern) {
                                re.is_match(&record.msg) || re.is_match(&record.target)
                            } else {
                                let pat = pattern.to_lowercase();
                                record.msg.to_lowercase().contains(&pat)
                                    || record.target.to_lowercase().contains(&pat)
                            };
                            if !matches {
                                continue;
                            }
                        }
                        println!("{}", format_record(&record, use_color));
                    }
                }
                offset = new_offset;
            }
        }

        thread::sleep(Duration::from_millis(500));
    }
}
