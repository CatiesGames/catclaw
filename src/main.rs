mod agent;
mod approval;
mod channel;
mod cmd_hook;
mod cli_ui;
mod config;
mod error;
mod gateway;
mod logging;
mod mcp_server;
mod pidfile;
mod router;
mod scheduler;
mod session;
mod state;
mod tui;
mod ws_client;
mod ws_protocol;
mod ws_server;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::agent::AgentLoader;
use crate::config::Config;
use crate::error::Result;
use crate::state::StateDb;

#[derive(Parser)]
#[command(name = "catclaw", version, about = "Personal AI assistant gateway powered by Claude Code CLI")]
struct Cli {
    /// Path to catclaw.toml config file
    #[arg(short, long, default_value = "./catclaw.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage the gateway daemon
    Gateway {
        #[command(subcommand)]
        command: GatewayCommands,
    },

    /// Launch the TUI interface
    Tui,

    /// Manage agents
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },

    /// Manage channel-to-agent bindings
    Bind {
        /// Binding pattern (e.g., "discord:channel:123")
        pattern: String,
        /// Agent ID to bind to
        agent: String,
    },

    /// Manage configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },

    /// Manage sessions
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },

    /// Add a channel adapter
    Channel {
        #[command(subcommand)]
        command: ChannelCommands,
    },

    /// Manage agent skills
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },

    /// Manage scheduled tasks
    Task {
        #[command(subcommand)]
        command: TaskCommands,
    },

    /// View gateway logs
    Logs {
        /// Stream logs in real-time (like tail -f)
        #[arg(short, long)]
        follow: bool,

        /// Minimum log level: error, warn, info, debug
        #[arg(short, long, default_value = "info")]
        level: String,

        /// Filter by regex pattern on message/target
        #[arg(short, long)]
        grep: Option<String>,

        /// Show logs since this time (ISO 8601 or HH:MM:SS)
        #[arg(long)]
        since: Option<String>,

        /// Show logs until this time (ISO 8601 or HH:MM:SS)
        #[arg(long)]
        until: Option<String>,

        /// Output raw JSON lines instead of formatted text
        #[arg(long)]
        json: bool,

        /// Maximum number of entries to show (most recent)
        #[arg(short = 'n', long, default_value = "100")]
        limit: usize,
    },

    /// Internal hooks called by Claude Code (not for direct user use)
    #[command(hide = true)]
    Hook {
        #[command(subcommand)]
        command: HookCommands,
    },
}

#[derive(Subcommand)]
enum HookCommands {
    /// PreToolUse hook — called by Claude Code before each tool execution
    PreTool {
        /// Session key for routing approval requests
        #[arg(long)]
        session_key: String,
    },
}

#[derive(Subcommand)]
enum GatewayCommands {
    /// Start the gateway (foreground by default, use -d for background daemon)
    Start {
        /// Run as background daemon
        #[arg(short, long)]
        daemon: bool,
    },
    /// Stop the background gateway
    Stop,
    /// Restart the background gateway
    Restart,
    /// Show gateway status
    Status,
}

#[derive(Subcommand)]
enum AgentCommands {
    /// Create a new agent
    New {
        /// Agent name/ID
        name: String,
    },
    /// List all agents
    List,
    /// Edit an agent's MD file
    Edit {
        /// Agent name/ID
        name: String,
        /// Which file to edit (soul, user, identity, agents, tools, boot, heartbeat, memory)
        file: String,
    },
    /// Configure tool permissions
    Tools {
        /// Agent name/ID
        name: String,
        /// Allowed tools (comma-separated)
        #[arg(long)]
        allow: Option<String>,
        /// Denied tools (comma-separated)
        #[arg(long)]
        deny: Option<String>,
        /// Tools requiring user approval before execution (comma-separated)
        #[arg(long)]
        approve: Option<String>,
    },
    /// Delete an agent
    Delete {
        /// Agent name/ID
        name: String,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Set a configuration value
    Set {
        /// Config key (e.g., "max_concurrent_sessions")
        key: String,
        /// Config value
        value: String,
    },
    /// Get a configuration value
    Get {
        /// Config key (e.g., "channels[0].dm_policy")
        key: String,
    },
    /// Show current configuration
    Show,
}

#[derive(Subcommand)]
enum SessionCommands {
    /// List all sessions
    List,
    /// Delete a session
    Delete {
        /// Session key
        key: String,
    },
}

#[derive(Subcommand)]
enum SkillCommands {
    /// List skills for an agent
    List {
        /// Agent name/ID
        agent: String,
    },
    /// Enable a skill
    Enable {
        /// Agent name/ID
        agent: String,
        /// Skill name
        skill: String,
    },
    /// Disable a skill
    Disable {
        /// Agent name/ID
        agent: String,
        /// Skill name
        skill: String,
    },
    /// Create a new custom skill
    Add {
        /// Agent name/ID
        agent: String,
        /// Skill name
        skill: String,
    },
    /// Install a skill from a source
    ///
    /// Sources:
    ///   @anthropic/<name>              — Official Anthropic skill
    ///   github:<owner>/<repo>/<path>   — GitHub repository
    ///   /local/path/to/skill           — Local directory
    Install {
        /// Agent name/ID
        agent: String,
        /// Skill source (e.g. @anthropic/skill-creator, github:user/repo/skill, ./path)
        source: String,
    },
    /// Uninstall a skill (delete skill directory)
    Uninstall {
        /// Agent name/ID
        agent: String,
        /// Skill name
        skill: String,
    },
}

#[derive(Subcommand)]
enum TaskCommands {
    /// List all scheduled tasks
    List,
    /// Add a one-shot task (runs once at the specified time)
    Add {
        /// Task name
        name: String,
        /// Agent ID to run the task
        #[arg(long, default_value = "main")]
        agent: String,
        /// Prompt to send to the agent
        #[arg(long)]
        prompt: String,
        /// When to run (minutes from now, e.g., "30" for 30 minutes)
        #[arg(long)]
        in_mins: Option<u64>,
        /// Cron expression (e.g., "0 */6 * * *" for every 6 hours)
        #[arg(long)]
        cron: Option<String>,
        /// Repeat interval in minutes (e.g., 60 for hourly)
        #[arg(long)]
        every: Option<i64>,
    },
    /// Enable a task
    Enable {
        /// Task ID
        id: i64,
    },
    /// Disable a task
    Disable {
        /// Task ID
        id: i64,
    },
    /// Delete a task
    Delete {
        /// Task ID
        id: i64,
    },
}

#[derive(Subcommand)]
enum ChannelCommands {
    /// Add a channel adapter
    Add {
        /// Channel type (discord, telegram, slack)
        #[arg(rename_all = "lowercase")]
        channel_type: String,
        /// Environment variable name for the token
        #[arg(long)]
        token_env: String,
        /// Guild IDs (Discord only)
        #[arg(long)]
        guilds: Option<Vec<String>>,
        /// Activation mode (mention, all)
        #[arg(long, default_value = "mention")]
        activation: String,
    },
    /// List configured channels
    List,
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Initialize logging based on command:
    // - Gateway foreground: dual output (file + console)
    // - None (default, TUI will start): file-only (no console pollution)
    // - Other commands: console-only (quiet)
    let is_gateway_fg = matches!(
        cli.command,
        Some(Commands::Gateway { command: GatewayCommands::Start { daemon: false } })
    );
    let is_default = cli.command.is_none();
    if is_gateway_fg || is_default {
        // Load config early to get log settings
        let config = if cli.config.exists() {
            Config::load(&cli.config).ok()
        } else {
            None
        };
        let workspace = config
            .as_ref()
            .map(|c| c.general.workspace.clone())
            .unwrap_or_else(|| std::path::PathBuf::from("./workspace"));
        let log_level = config
            .as_ref()
            .map(|c| c.logging.level.clone())
            .unwrap_or_else(|| "info".to_string());
        let log_dir = config
            .as_ref()
            .map(|c| c.logging.resolve_log_dir(&c.general.workspace))
            .unwrap_or_else(|| workspace.join("logs"));

        if is_gateway_fg {
            logging::init_logging(&log_dir, &log_level);
        } else {
            // Default mode: file-only logging so console stays clean for TUI
            logging::init_file_only_logging(&log_dir, &log_level);
        }
    } else {
        logging::init_console_logging();
    }

    match cli.command {
        None => {
            // Unified startup: splash → auto-init → ensure gateway → TUI
            tui::splash::print_splash_to_terminal();

            // Check if config exists, auto-init if not
            let config = if cli.config.exists() {
                cli_ui::spinner_start("Loading configuration...");
                let cfg = Config::load(&cli.config)?;
                cli_ui::spinner_finish("✓", "Configuration loaded");
                cfg
            } else {
                cmd_init(&cli.config).await?
            };

            load_dotenv();

            let ws_port = config.general.port;
            let ws_url = format!("ws://127.0.0.1:{}/ws", ws_port);

            cli_ui::status_msg("⏳", "Preparing gateway...");

            // Check if a gateway is already running
            let pid_path = pidfile::pid_path(Some(&config));
            let our_pid = pidfile::read_pid(&pid_path);
            let gateway_running = our_pid
                .map(|pid| pidfile::is_running(pid))
                .unwrap_or(false);

            if !gateway_running {
                // Port conflict check before spawning
                if let Err(e) = check_port_conflict(ws_port, our_pid).await {
                    eprintln!("{}", e);
                    std::process::exit(1);
                }
                start_background_gateway_quiet(&cli.config)?;
                let ready = wait_for_gateway(&ws_url, 150).await;
                if !ready {
                    eprintln!("Gateway failed to start within timeout.");
                    std::process::exit(1);
                }
            } else {
                // PID exists — verify WS port is actually reachable
                let addr = ws_url.strip_prefix("ws://").unwrap_or(&ws_url).split('/').next().unwrap_or("127.0.0.1:21130");
                let reachable = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    tokio::net::TcpStream::connect(addr),
                ).await.map(|r| r.is_ok()).unwrap_or(false);

                if !reachable {
                    // Stale PID — restart
                    start_background_gateway_quiet(&cli.config)?;
                    let ready = wait_for_gateway(&ws_url, 150).await;
                    if !ready {
                        eprintln!("Gateway failed to start within timeout.");
                        std::process::exit(1);
                    }
                }
            }

            // Read gateway PID for display
            let gw_pid = pidfile::read_pid(&pid_path).unwrap_or(0);

            // Summary box
            let agents_str = config
                .agents
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let channels_str = if config.channels.is_empty() {
                "none".to_string()
            } else {
                config
                    .channels
                    .iter()
                    .map(|c| c.channel_type.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            cli_ui::summary_box(&[
                ("Config", &cli.config.display().to_string()),
                ("Agents", &agents_str),
                ("Channels", &channels_str),
                ("Gateway", &format!("PID {} · ws://127.0.0.1:{}", gw_pid, ws_port)),
            ]);

            // Ask: TUI or exit
            cli_ui::section_header("🚀", "Launch");
            cli_ui::section_empty();
            let choice = cli_ui::section_select(
                &["Launch TUI", "Exit"],
                0,
            );
            cli_ui::section_empty();
            cli_ui::section_footer();

            if choice == 0 {
                tui::run(config, cli.config.clone(), &ws_url).await?;
            }
        }

        Some(Commands::Tui) => {
            let config = if cli.config.exists() {
                Config::load(&cli.config)?
            } else {
                eprintln!("No catclaw.toml found. Run `catclaw` first.");
                std::process::exit(1);
            };
            load_dotenv();

            let ws_url = format!("ws://127.0.0.1:{}/ws", config.general.port);

            // Check that gateway is running
            let pid_path = pidfile::pid_path(Some(&config));
            let gateway_running = pidfile::read_pid(&pid_path)
                .map(|pid| pidfile::is_running(pid))
                .unwrap_or(false);

            if !gateway_running {
                eprintln!("Gateway is not running. Start it with: catclaw gateway");
                eprintln!("Or run `catclaw` to start both gateway and TUI.");
                std::process::exit(1);
            }

            cli_ui::spinner_start("Connecting to gateway...");
            // WS connect has retry logic built-in, just show progress
            cli_ui::spinner_finish("🟢", &format!("Connected to gateway (port {})", config.general.port));

            tui::run(config, cli.config.clone(), &ws_url).await?;
        }

        Some(Commands::Gateway { command }) => {
            match command {
                GatewayCommands::Start { daemon } => {
                    load_dotenv();
                    let config = Config::load(&cli.config)?;
                    let ws_port = config.general.port;
                    let pid_path = pidfile::pid_path(Some(&config));
                    let our_pid = pidfile::read_pid(&pid_path);
                    if let Err(e) = check_port_conflict(ws_port, our_pid).await {
                        eprintln!("{}", e);
                        std::process::exit(1);
                    }
                    if daemon {
                        start_background_gateway(&cli.config)?;
                    } else {
                        gateway::run(config, cli.config.clone()).await?;
                    }
                }

                GatewayCommands::Stop => {
                    let config = if cli.config.exists() {
                        Some(Config::load(&cli.config)?)
                    } else {
                        None
                    };
                    let pid_path = pidfile::pid_path(config.as_ref());
                    cmd_gateway_stop(&pid_path);
                }

                GatewayCommands::Restart => {
                    load_dotenv();
                    let config = Config::load(&cli.config)?;
                    let pid_path = pidfile::pid_path(Some(&config));
                    // Stop if running
                    cmd_gateway_stop(&pid_path);
                    // Brief pause before restart
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    // Check port then start
                    let our_pid = pidfile::read_pid(&pid_path);
                    if let Err(e) = check_port_conflict(config.general.port, our_pid).await {
                        eprintln!("{}", e);
                        std::process::exit(1);
                    }
                    start_background_gateway(&cli.config)?;
                }

                GatewayCommands::Status => {
                    let config = if cli.config.exists() {
                        Some(Config::load(&cli.config)?)
                    } else {
                        None
                    };
                    let pid_path = pidfile::pid_path(config.as_ref());
                    match pidfile::read_pid(&pid_path) {
                        Some(pid) if pidfile::is_running(pid) => {
                            cli_ui::status_msg("🟢", &format!("Gateway running (PID {})", pid));
                        }
                        Some(_pid) => {
                            cli_ui::status_msg("🔴", "Gateway not running (stale PID file)");
                            pidfile::remove_pid(&pid_path);
                        }
                        None => {
                            cli_ui::status_msg("⚪", "Gateway not running");
                        }
                    }
                }
            }
        }

        Some(Commands::Agent { command }) => {
            let mut config = Config::load(&cli.config)?;
            match command {
                AgentCommands::New { name } => {
                    cmd_agent_new(&mut config, &cli.config, &name).await?;
                }
                AgentCommands::List => {
                    cmd_agent_list(&config);
                }
                AgentCommands::Edit { name, file } => {
                    cmd_agent_edit(&config, &name, &file)?;
                }
                AgentCommands::Tools { name, allow, deny, approve } => {
                    cmd_agent_tools(&mut config, &cli.config, &name, allow, deny, approve).await?;
                }
                AgentCommands::Delete { name } => {
                    cmd_agent_delete(&mut config, &cli.config, &name)?;
                }
            }
        }

        Some(Commands::Bind { pattern, agent }) => {
            let mut config = Config::load(&cli.config)?;
            // Remove existing binding with same pattern
            config.bindings.retain(|b| b.pattern != pattern);
            config.bindings.push(config::BindingConfig {
                pattern: pattern.clone(),
                agent: agent.clone(),
            });
            config.save(&cli.config)?;
            println!("Bound '{}' → agent '{}' (saved to {})", pattern, agent, cli.config.display());
        }

        Some(Commands::Config { command }) => {
            let config = Config::load(&cli.config)?;
            match command {
                ConfigCommands::Show => {
                    let toml = toml::to_string_pretty(&config)?;
                    println!("{}", toml);
                }
                ConfigCommands::Get { key } => {
                    // Try gateway WS first, fall back to local config
                    let ws_url = format!("ws://127.0.0.1:{}/ws", config.general.port);
                    if let Ok((client, _event_rx)) = crate::ws_client::GatewayClient::connect(&ws_url, &config.general.ws_token).await {
                        match client.request("config.get", serde_json::json!({"key": &key})).await {
                            Ok(resp) => {
                                let value = resp.get("value").and_then(|v| v.as_str()).unwrap_or("");
                                println!("{} = {}", key, value);
                            }
                            Err(e) => {
                                eprintln!("Error: {}", e);
                                std::process::exit(1);
                            }
                        }
                    } else {
                        match config.config_get(&key) {
                            Ok(value) => println!("{} = {}", key, value),
                            Err(e) => {
                                eprintln!("Error: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                }
                ConfigCommands::Set { key, value } => {
                    // Try gateway WS first (hot-reload), fall back to file-only
                    let ws_url = format!("ws://127.0.0.1:{}/ws", config.general.port);
                    if let Ok((client, _event_rx)) = crate::ws_client::GatewayClient::connect(&ws_url, &config.general.ws_token).await {
                        match client.request("config.set", serde_json::json!({"key": &key, "value": &value})).await {
                            Ok(resp) => {
                                let needs_restart = resp.get("needs_restart").and_then(|v| v.as_bool()).unwrap_or(false);
                                if needs_restart {
                                    println!("Set {} = {} (requires gateway restart to take effect)", key, value);
                                } else {
                                    println!("Set {} = {} (applied immediately)", key, value);
                                }
                            }
                            Err(e) => {
                                eprintln!("Error: {}", e);
                                std::process::exit(1);
                            }
                        }
                    } else {
                        // Gateway not running, modify file directly
                        let mut config = config;
                        match config.apply_config_set(&key, &value) {
                            Ok(_) => {
                                config.save(&cli.config)?;
                                println!("Set {} = {} (saved to file, will apply on next gateway start)", key, value);
                            }
                            Err(e) => {
                                eprintln!("Error: {}", e);
                                std::process::exit(1);
                            }
                        }
                    }
                }
            }
        }

        Some(Commands::Session { command }) => {
            let config = Config::load(&cli.config)?;
            let state_db = StateDb::open(&config.general.state_db)?;
            match command {
                SessionCommands::List => {
                    let sessions = state_db.list_sessions()?;
                    if sessions.is_empty() {
                        println!("No sessions.");
                    } else {
                        println!(
                            "{:<50} {:<10} {:<10} {:<10}",
                            "KEY", "AGENT", "STATE", "ORIGIN"
                        );
                        for s in &sessions {
                            println!(
                                "{:<50} {:<10} {:<10} {:<10}",
                                s.session_key, s.agent_id, s.state, s.origin
                            );
                        }
                    }
                }
                SessionCommands::Delete { key } => {
                    state_db.delete_session(&key)?;
                    println!("Deleted session '{}'", key);
                }
            }
        }

        Some(Commands::Skill { command }) => {
            let config = Config::load(&cli.config)?;
            match command {
                SkillCommands::List { agent } => {
                    cmd_skill_list(&config, &agent)?;
                }
                SkillCommands::Enable { agent, skill } => {
                    cmd_skill_toggle(&config, &agent, &skill, true)?;
                }
                SkillCommands::Disable { agent, skill } => {
                    cmd_skill_toggle(&config, &agent, &skill, false)?;
                }
                SkillCommands::Add { agent, skill } => {
                    cmd_skill_add(&config, &agent, &skill)?;
                }
                SkillCommands::Install { agent, source } => {
                    cmd_skill_install(&config, &agent, &source).await?;
                }
                SkillCommands::Uninstall { agent, skill } => {
                    cmd_skill_uninstall(&config, &agent, &skill)?;
                }
            }
        }

        Some(Commands::Task { command }) => {
            let config = Config::load(&cli.config)?;
            let state_db = StateDb::open(&config.general.state_db)?;
            match command {
                TaskCommands::List => {
                    cmd_task_list(&state_db)?;
                }
                TaskCommands::Add {
                    name,
                    agent,
                    prompt,
                    in_mins,
                    cron,
                    every,
                } => {
                    cmd_task_add(&state_db, &name, &agent, &prompt, in_mins, cron, every)?;
                }
                TaskCommands::Enable { id } => {
                    state_db.enable_task(id)?;
                    println!("Task {} enabled.", id);
                }
                TaskCommands::Disable { id } => {
                    state_db.disable_task(id)?;
                    println!("Task {} disabled.", id);
                }
                TaskCommands::Delete { id } => {
                    state_db.delete_task(id)?;
                    println!("Task {} deleted.", id);
                }
            }
        }

        Some(Commands::Logs {
            follow,
            level,
            grep,
            since,
            until,
            json,
            limit,
        }) => {
            let config = Config::load(&cli.config)?;
            let log_dir = config.logging.resolve_log_dir(&config.general.workspace);

            if follow {
                // Tail mode: stream new entries
                let use_color = atty::is(atty::Stream::Stdout);
                logging::tail_follow(
                    &log_dir,
                    &level,
                    grep.as_deref(),
                    use_color && !json,
                )?;
            } else {
                // Read existing logs
                let files = logging::list_log_files(&log_dir);
                if files.is_empty() {
                    println!("No log files found in {}", log_dir.display());
                    return Ok(());
                }

                // Collect records from files (newest file first, but we want chronological)
                let mut all_records: Vec<logging::LogRecord> = Vec::new();
                for file in files.iter().rev() {
                    all_records.extend(logging::read_log_file(file));
                }

                // Apply filters
                let filtered: Vec<&logging::LogRecord> = all_records
                    .iter()
                    .filter(|r| logging::filter_by_level(std::slice::from_ref(r), &level).len() == 1)
                    .collect();

                let filtered: Vec<&logging::LogRecord> = if let Some(ref pattern) = grep {
                    filtered
                        .into_iter()
                        .filter(|r| {
                            logging::filter_by_grep(std::slice::from_ref(r), pattern).len() == 1
                        })
                        .collect()
                } else {
                    filtered
                };

                let filtered: Vec<&logging::LogRecord> = if since.is_some() || until.is_some() {
                    filtered
                        .into_iter()
                        .filter(|r| {
                            logging::filter_by_time(
                                std::slice::from_ref(r),
                                since.as_deref(),
                                until.as_deref(),
                            )
                            .len()
                                == 1
                        })
                        .collect()
                } else {
                    filtered
                };

                // Take last N entries
                let start = if filtered.len() > limit {
                    filtered.len() - limit
                } else {
                    0
                };
                let display = &filtered[start..];

                let use_color = atty::is(atty::Stream::Stdout);
                for record in display {
                    if json {
                        if let Ok(j) = serde_json::to_string(record) {
                            println!("{}", j);
                        }
                    } else {
                        println!("{}", logging::format_record(record, use_color));
                    }
                }

                if display.is_empty() {
                    println!("No matching log entries.");
                }
            }
        }

        Some(Commands::Channel { command }) => {
            let mut config = Config::load(&cli.config)?;
            match command {
                ChannelCommands::Add {
                    channel_type,
                    token_env,
                    guilds,
                    activation,
                } => {
                    config.channels.push(crate::config::ChannelConfig {
                        channel_type,
                        token_env,
                        guilds: guilds.unwrap_or_default(),
                        activation,
                        overrides: vec![],
                        dm_policy: "open".to_string(),
                        dm_allow: vec![],
                        dm_deny: vec![],
                        group_policy: "open".to_string(),
                        group_allow: vec![],
                        group_deny: vec![],
                    });
                    config.save(&cli.config)?;
                    println!("Channel added. Config saved.");
                }
                ChannelCommands::List => {
                    if config.channels.is_empty() {
                        println!("No channels configured.");
                    } else {
                        for ch in &config.channels {
                            println!(
                                "  {} (token_env: {}, activation: {})",
                                ch.channel_type, ch.token_env, ch.activation
                            );
                        }
                    }
                }
            }
        }

        Some(Commands::Hook { command }) => {
            match command {
                HookCommands::PreTool { session_key } => {
                    // Note: this exits the process directly (exits 0 or 2)
                    cmd_hook::run_pre_tool(&cli.config, &session_key).await;
                }
            }
        }
    }

    Ok(())
}

async fn cmd_init(config_path: &PathBuf) -> Result<Config> {
    use dialoguer::{Input, Password};

    let is_update = config_path.exists();
    let mut config = if is_update {
        Config::load(config_path)?
    } else {
        Config::default_init()
    };

    let total_steps = 2;

    // ── Step 1: Claude Code CLI ────────────────────────────────────────
    cli_ui::step_indicator(1, total_steps, "Prerequisites");
    cli_ui::section_header("🔧", "Claude Code CLI");
    cli_ui::section_empty();

    let claude_ok = match std::process::Command::new("claude")
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            cli_ui::section_ok(&format!("claude CLI found: {}", version.trim()));
            true
        }
        _ => {
            cli_ui::section_err("claude CLI not found");
            cli_ui::section_hint("Install → https://docs.anthropic.com/en/docs/claude-code");
            cli_ui::section_hint("Then run: claude login");
            false
        }
    };

    if claude_ok {
        if std::env::var("CLAUDECODE").is_ok() {
            cli_ui::section_warn("Running inside Claude Code — cannot verify login");
            cli_ui::section_hint("Make sure you've run: claude login");
        } else {
            // Subscription check can take a few seconds
            cli_ui::section_line(&format!(
                "{}Checking subscription...{}",
                cli_ui::OVERLAY, cli_ui::RESET
            ));
            match std::process::Command::new("claude")
                .args(["-p", "reply with just OK", "--output-format", "text", "--max-turns", "1"])
                .env_remove("CLAUDECODE")
                .output()
            {
                Ok(output) if output.status.success() => {
                    cli_ui::section_ok("Active subscription confirmed");
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    cli_ui::section_err("Subscription issue detected");
                    if stderr.contains("login") || stderr.contains("auth") {
                        cli_ui::section_hint("Run: claude login");
                    } else {
                        cli_ui::section_hint(&format!("Error: {}", stderr.trim()));
                    }
                }
                Err(_) => {
                    cli_ui::section_warn("Could not verify — make sure claude is logged in");
                }
            }
        }
    }

    cli_ui::section_empty();

    if !claude_ok {
        let choice = cli_ui::section_select(
            &["Continue anyway", "Abort setup"],
            1,
        );
        cli_ui::section_empty();
        cli_ui::section_footer();
        if choice != 0 {
            return Err(crate::error::CatClawError::Config(
                "Claude Code CLI required. Install and login first.".to_string(),
            ));
        }
    } else {
        cli_ui::section_footer();
    }

    // ── Auto-create main agent if none exists ──────────────────────────
    if config.agents.is_empty() {
        let agent_name = "main";
        let workspace_path = config.general.workspace.join("agents").join(agent_name);
        std::fs::create_dir_all(&workspace_path)?;
        AgentLoader::create_workspace(&workspace_path, &config.general.workspace, agent_name)?;
        cli_ui::spinner_start("Installing default skills...");
        AgentLoader::install_remote_skills(&config.general.workspace).await?;
        cli_ui::spinner_finish("✓", "Default skills installed");
        config.agents.push(crate::config::AgentConfig {
            id: agent_name.to_string(),
            workspace: workspace_path,
            default: true,
            model: None,
            fallback_model: None,
            approval: crate::config::ApprovalConfig::default(),
        });
        cli_ui::status_msg("✅", "Agent 'main' created (default)");
        cli_ui::section_hint("  Edit personality: catclaw agent edit main soul");
        cli_ui::section_hint("  Add more agents:  catclaw agent new <name>");
        println!();
    }

    // ── Step 2: Channel adapters ─────────────────────────────────────
    cli_ui::step_indicator(2, total_steps, "Channels");
    cli_ui::section_header("📡", "Channels");
    cli_ui::section_empty();

    let existing_channels: Vec<String> = config.channels.iter().map(|c| c.channel_type.clone()).collect();

    // Show existing channels
    if !existing_channels.is_empty() {
        for ch in &existing_channels {
            cli_ui::section_ok(&format!("{} configured", ch));
        }
        cli_ui::section_empty();
    }

    // Available channels (type, label, implemented)
    let available_channels: Vec<(&str, &str, bool)> = vec![
        ("discord", "Discord", true),
        ("telegram", "Telegram", true),
        ("slack", "Slack", false),
    ];

    // Load existing .env if present
    let env_path = std::path::Path::new(".env");
    let mut env_lines: Vec<String> = if env_path.exists() {
        std::fs::read_to_string(env_path)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    } else {
        Vec::new()
    };

    // Channel selection + setup loop
    loop {
        let configured: Vec<String> = config.channels.iter().map(|c| c.channel_type.clone()).collect();
        let mut select_labels: Vec<String> = Vec::new();
        let mut select_indices: Vec<Option<usize>> = Vec::new();

        for (i, (ch_type, ch_label, implemented)) in available_channels.iter().enumerate() {
            if !implemented { continue; }
            if configured.contains(&ch_type.to_string()) {
                select_labels.push(format!("{} ✓ (reconfigure)", ch_label));
            } else {
                select_labels.push(format!("Add {}", ch_label));
            }
            select_indices.push(Some(i));
        }
        select_labels.push("Done".to_string());
        select_indices.push(None);

        cli_ui::section_line(&format!(
            "{}Select a channel to configure:{}",
            cli_ui::SUBTEXT, cli_ui::RESET
        ));
        cli_ui::section_empty();

        let labels_ref: Vec<&str> = select_labels.iter().map(|s| s.as_str()).collect();
        let choice = cli_ui::section_select(&labels_ref, labels_ref.len() - 1);

        let idx = match select_indices[choice] {
            Some(i) => i,
            None => break, // "Done"
        };

        // Remove existing config if reconfiguring
        let (ch_type, _, _) = available_channels[idx];
        config.channels.retain(|c| c.channel_type != ch_type);
        cli_ui::section_empty();
        cli_ui::section_footer();

        // ── Run setup for selected channel ──
        let (ch_type, ch_label, _) = available_channels[idx];

        if ch_type == "discord" {
            println!();
            cli_ui::section_header("🎮", "Discord Bot Setup");
            cli_ui::section_empty();
            cli_ui::section_line(&format!(
                "{}1.{} Developer Portal → Applications → {}New Application{}",
                cli_ui::MAUVE, cli_ui::RESET, cli_ui::TEXT, cli_ui::RESET
            ));
            cli_ui::section_line(&format!(
                "{}2.{} Bot → Add Bot → {}Reset Token{} → copy token",
                cli_ui::MAUVE, cli_ui::RESET, cli_ui::TEXT, cli_ui::RESET
            ));
            cli_ui::section_line(&format!(
                "{}3.{} OAuth2 → URL Generator → scope {}\"bot\"{} → invite to your server",
                cli_ui::MAUVE, cli_ui::RESET, cli_ui::TEAL, cli_ui::RESET
            ));
            cli_ui::section_empty();
            cli_ui::section_hint(&format!(
                "Tip: enable {}Message Content Intent{} if you need message text.",
                cli_ui::YELLOW, cli_ui::OVERLAY
            ));
            cli_ui::section_hint("(Bot → Privileged Gateway Intents → Message Content Intent)");
            cli_ui::section_empty();
            cli_ui::section_line(&format!(
                "{}Portal:{} {}https://discord.com/developers/applications{}",
                cli_ui::SUBTEXT, cli_ui::RESET, cli_ui::SAPPHIRE, cli_ui::RESET
            ));
            cli_ui::section_empty();
            cli_ui::section_divider();
            cli_ui::section_empty();

            let env_var_name = "CATCLAW_DISCORD_TOKEN";
            let existing_token = std::env::var(env_var_name).ok();

            let token = if let Some(ref t) = existing_token {
                cli_ui::section_ok(&format!(
                    "{} found in environment ({}...)",
                    env_var_name,
                    &t[..t.len().min(8)]
                ));
                cli_ui::section_empty();
                cli_ui::section_footer();

                let use_existing = cli_ui::section_select(
                    &["Use existing token", "Enter a new token"],
                    0,
                );
                cli_ui::section_empty();
                if use_existing == 0 {
                    t.clone()
                } else {
                    cli_ui::section_footer();
                    Password::new()
                        .with_prompt("  Paste your Discord bot token")
                        .interact()
                        .unwrap_or_default()
                }
            } else {
                cli_ui::section_footer();
                Password::new()
                    .with_prompt("  Paste your Discord bot token")
                    .interact()
                    .unwrap_or_default()
            };

            if token.is_empty() {
                cli_ui::status_msg("⚠️", "No token provided — skipping Discord");
                println!();
                continue;
            }

            write_env_var(&mut env_lines, env_var_name, &token);

            // Guild ID
            println!();
            let guild_input: String = Input::new()
                .with_prompt("  Discord Server ID (right-click server → Copy Server ID)")
                .allow_empty(true)
                .interact_text()
                .unwrap_or_default();

            let guilds: Vec<String> = guild_input
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            if guilds.is_empty() {
                cli_ui::status_msg("ℹ️", "No server ID — bot will respond in all servers");
            }

            // Activation mode
            println!();
            cli_ui::section_header("📡", "Activation Mode");
            cli_ui::section_empty();
            cli_ui::section_hint("DMs are always responded to. This setting controls server channels:");
            cli_ui::section_empty();
            let act_idx = cli_ui::section_select(
                &["mention — respond only when @mentioned in channels", "all — respond to every message in channels"],
                0,
            );
            cli_ui::section_empty();
            cli_ui::section_footer();
            let activation = if act_idx == 0 {
                "mention".to_string()
            } else {
                "all".to_string()
            };

            config.channels.push(crate::config::ChannelConfig {
                channel_type: ch_type.to_string(),
                token_env: env_var_name.to_string(),
                guilds,
                activation,
                overrides: vec![],
                dm_policy: "open".to_string(),
                dm_allow: vec![],
                dm_deny: vec![],
                group_policy: "open".to_string(),
                group_allow: vec![],
                group_deny: vec![],
            });

            std::env::set_var(env_var_name, &token);
            println!();
            cli_ui::status_msg("✅", "Discord configured (token saved to .env)");
            println!();
        } else if ch_type == "telegram" {
            println!();
            cli_ui::section_header("📬", "Telegram Bot Setup");
            cli_ui::section_empty();
            cli_ui::section_line(&format!(
                "{}1.{} Open Telegram and message {}@BotFather{}",
                cli_ui::MAUVE, cli_ui::RESET, cli_ui::TEXT, cli_ui::RESET
            ));
            cli_ui::section_line(&format!(
                "{}2.{} Send {}/newbot{} and follow the prompts",
                cli_ui::MAUVE, cli_ui::RESET, cli_ui::TEAL, cli_ui::RESET
            ));
            cli_ui::section_line(&format!(
                "{}3.{} Copy the HTTP API token BotFather gives you",
                cli_ui::MAUVE, cli_ui::RESET
            ));
            cli_ui::section_empty();
            cli_ui::section_hint("Tip: use /setprivacy to disable privacy mode if you want the bot to see all group messages.");
            cli_ui::section_empty();
            cli_ui::section_divider();
            cli_ui::section_empty();

            let env_var_name = "CATCLAW_TELEGRAM_TOKEN";
            let existing_token = std::env::var(env_var_name).ok();

            let token = if let Some(ref t) = existing_token {
                cli_ui::section_ok(&format!(
                    "{} found in environment ({}...)",
                    env_var_name,
                    &t[..t.len().min(8)]
                ));
                cli_ui::section_empty();
                cli_ui::section_footer();

                let use_existing = cli_ui::section_select(
                    &["Use existing token", "Enter a new token"],
                    0,
                );
                cli_ui::section_empty();
                if use_existing == 0 {
                    t.clone()
                } else {
                    cli_ui::section_footer();
                    Password::new()
                        .with_prompt("  Paste your Telegram bot token")
                        .interact()
                        .unwrap_or_default()
                }
            } else {
                cli_ui::section_footer();
                Password::new()
                    .with_prompt("  Paste your Telegram bot token")
                    .interact()
                    .unwrap_or_default()
            };

            if token.is_empty() {
                cli_ui::status_msg("⚠️", "No token provided — skipping Telegram");
                println!();
                continue;
            }

            write_env_var(&mut env_lines, env_var_name, &token);

            // Activation mode
            println!();
            cli_ui::section_header("📡", "Activation Mode");
            cli_ui::section_empty();
            cli_ui::section_hint("DMs are always responded to. This setting controls group chats:");
            cli_ui::section_empty();
            let act_idx = cli_ui::section_select(
                &["mention — respond only when @bot_username is mentioned", "all — respond to every message in group chats"],
                0,
            );
            cli_ui::section_empty();
            cli_ui::section_footer();
            let activation = if act_idx == 0 {
                "mention".to_string()
            } else {
                "all".to_string()
            };

            config.channels.push(crate::config::ChannelConfig {
                channel_type: ch_type.to_string(),
                token_env: env_var_name.to_string(),
                guilds: vec![],
                activation,
                overrides: vec![],
                dm_policy: "open".to_string(),
                dm_allow: vec![],
                dm_deny: vec![],
                group_policy: "open".to_string(),
                group_allow: vec![],
                group_deny: vec![],
            });

            std::env::set_var(env_var_name, &token);
            println!();
            cli_ui::status_msg("✅", "Telegram configured (token saved to .env)");
            println!();
        } else {
            // Generic channel setup (for future adapters)
            println!();
            cli_ui::section_header("📡", &format!("{} Setup", ch_label));
            cli_ui::section_empty();

            let env_var_name = format!("CATCLAW_{}_TOKEN", ch_type.to_uppercase());
            let token: String = Password::new()
                .with_prompt(format!("  {} bot token", ch_label))
                .interact()
                .unwrap_or_default();

            if token.is_empty() {
                cli_ui::section_warn(&format!("No token — skipping {}", ch_label));
                cli_ui::section_footer();
                continue;
            }

            write_env_var(&mut env_lines, &env_var_name, &token);

            config.channels.push(crate::config::ChannelConfig {
                channel_type: ch_type.to_string(),
                token_env: env_var_name.clone(),
                guilds: vec![],
                activation: "mention".to_string(),
                overrides: vec![],
                dm_policy: "open".to_string(),
                dm_allow: vec![],
                dm_deny: vec![],
                group_policy: "open".to_string(),
                group_allow: vec![],
                group_deny: vec![],
            });

            std::env::set_var(&env_var_name, &token);
            cli_ui::section_ok(&format!("{} configured", ch_label));
            cli_ui::section_footer();
        }

        println!();
    } // end channel selection + setup loop

    if config.channels.is_empty() {
        cli_ui::status_msg("ℹ️", "No channels configured. Add later: catclaw channel add");
        println!();
    }

    // Write .env file
    if !env_lines.is_empty() {
        std::fs::write(env_path, env_lines.join("\n") + "\n")?;
    }

    // ── Create workspace structure ─────────────────────────────────────
    let workspace = &config.general.workspace;
    std::fs::create_dir_all(workspace)?;

    // Ensure log directory exists
    let log_dir = config.logging.resolve_log_dir(workspace);
    std::fs::create_dir_all(&log_dir)?;

    // Ensure all agent workspaces exist
    // Ensure shared skills pool exists and all agent workspaces are set up
    AgentLoader::install_builtin_skills(&config.general.workspace)?;
    AgentLoader::install_remote_skills(&config.general.workspace).await?;
    for agent_config in &config.agents {
        if !agent_config.workspace.join("SOUL.md").exists() {
            AgentLoader::create_workspace(&agent_config.workspace, &config.general.workspace, &agent_config.id)?;
        }
        // Ensure skills.toml exists
        if !agent_config.workspace.join("skills.toml").exists() {
            crate::agent::SkillsConfig::default().save(&agent_config.workspace)?;
        }
    }

    // Initialize state DB + run migrations (sessions are created on first message)
    let _state_db = StateDb::open(&config.general.state_db)?;

    // Save config
    config.save(config_path)?;

    println!();
    cli_ui::status_msg("✅", "Configuration saved");

    Ok(config)
}

/// Write or update an env var in the .env lines
fn write_env_var(lines: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{}=", key);
    if let Some(pos) = lines.iter().position(|l| l.starts_with(&prefix)) {
        lines[pos] = format!("{}={}", key, value);
    } else {
        lines.push(format!("{}={}", key, value));
    }
}

/// Load .env file into process environment (simple parser, no crate needed)
fn load_dotenv() {
    let env_path = std::path::Path::new(".env");
    if let Ok(content) = std::fs::read_to_string(env_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim().trim_matches('"').trim_matches('\'');
                if !key.is_empty() {
                    std::env::set_var(key, value);
                }
            }
        }
    }
}

/// Wait for the gateway WS port to become reachable.
fn cmd_gateway_stop(pid_path: &std::path::Path) {
    match pidfile::read_pid(pid_path) {
        Some(pid) if pidfile::is_running(pid) => {
            println!("Stopping gateway (PID {})...", pid);
            if pidfile::stop_process(pid) {
                for _ in 0..20 {
                    if !pidfile::is_running(pid) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                if pidfile::is_running(pid) {
                    eprintln!("Gateway did not stop in time. You may need to kill PID {} manually.", pid);
                } else {
                    pidfile::remove_pid(pid_path);
                    println!("Gateway stopped.");
                }
            } else {
                eprintln!("Failed to send SIGTERM to PID {}.", pid);
            }
        }
        Some(_) => {
            println!("Gateway is not running (stale PID file). Cleaning up.");
            pidfile::remove_pid(pid_path);
        }
        None => {
            println!("No gateway running (no PID file found).");
        }
    }
}

async fn wait_for_gateway(ws_url: &str, max_attempts: u32) -> bool {
    let addr = ws_url
        .strip_prefix("ws://")
        .unwrap_or(ws_url)
        .split('/')
        .next()
        .unwrap_or("127.0.0.1:21130");

    for i in 0..max_attempts {
        let connect = tokio::net::TcpStream::connect(addr);
        let timeout = tokio::time::timeout(std::time::Duration::from_millis(200), connect);
        if let Ok(Ok(_)) = timeout.await {
            return true;
        }
        // Print a progress dot every second (every 10 attempts at 100ms each)
        if i > 0 && i % 10 == 0 {
            eprint!(".");
            let _ = std::io::Write::flush(&mut std::io::stderr());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    eprintln!(); // newline after dots
    false
}

/// Check if a port is in use by a non-catclaw process.
/// Returns Ok(()) if the port is free or occupied by our own gateway.
/// Returns Err if occupied by another process and user declined to kill it.
///
/// `our_pid` — the PID from our own catclaw PID file (if any).
async fn check_port_conflict(port: u16, our_pid: Option<u32>) -> std::result::Result<(), String> {
    let addr = format!("127.0.0.1:{}", port);
    let occupied = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false);

    if !occupied {
        return Ok(());
    }

    // Port is in use — find out who
    let pids = find_pids_on_port(port);

    // If it's our own gateway, that's fine
    if let Some(our) = our_pid {
        if pids == vec![our] {
            return Ok(());
        }
    }

    // Unknown process — warn and ask
    let pid_str = if pids.is_empty() {
        "unknown process".to_string()
    } else {
        pids.iter()
            .map(|p| {
                let name = process_name(*p).unwrap_or_else(|| "?".to_string());
                format!("{} ({})", p, name)
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    eprintln!();
    cli_ui::status_msg("⚠️ ", &format!("Port {} is already in use by {}", port, pid_str));

    let kill = cli_ui::section_confirm(&format!("Kill {} and continue?", pid_str), false);
    if !kill {
        return Err(format!("Port {} is in use. Aborting.", port));
    }

    // Kill each PID
    for pid in &pids {
        let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
    }

    // Wait briefly for port to free
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let still_occupied = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            tokio::net::TcpStream::connect(&addr),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);
        if !still_occupied {
            cli_ui::status_msg("✓", &format!("Port {} freed", port));
            return Ok(());
        }
    }

    Err(format!("Port {} still in use after killing PID {}. Aborting.", port, pid_str))
}

/// Find PIDs listening on a TCP port (macOS/Linux via lsof).
fn find_pids_on_port(port: u16) -> Vec<u32> {
    let output = std::process::Command::new("lsof")
        .args(["-ti", &format!(":{}", port), "-sTCP:LISTEN"])
        .output();
    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .filter_map(|s| s.parse::<u32>().ok())
            .collect(),
        _ => Vec::new(),
    }
}

/// Get the process name for a PID via `ps -p PID -o comm=`.
fn process_name(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    if out.status.success() {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if name.is_empty() { None } else { Some(name) }
    } else {
        None
    }
}

/// Spawn the gateway as a background process using `catclaw --config <path> gateway`.
fn start_background_gateway(config_path: &std::path::Path) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let config_str = config_path.to_string_lossy().to_string();

    let child = std::process::Command::new(exe)
        .args(["--config", &config_str, "gateway", "start"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    cli_ui::status_msg("⏳", &format!("Gateway spawning (PID {})", child.id()));
    Ok(())
}

/// Same as start_background_gateway but doesn't print (for use with spinner).
fn start_background_gateway_quiet(config_path: &std::path::Path) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let config_str = config_path.to_string_lossy().to_string();

    std::process::Command::new(exe)
        .args(["--config", &config_str, "gateway", "start"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    Ok(())
}

async fn cmd_agent_new(config: &mut Config, config_path: &PathBuf, name: &str) -> Result<()> {
    // Check if agent already exists
    if config.agents.iter().any(|a| a.id == name) {
        println!("Agent '{}' already exists.", name);
        return Ok(());
    }

    let workspace = config.general.workspace.join("agents").join(name);
    AgentLoader::create_workspace(&workspace, &config.general.workspace, name)?;
    AgentLoader::install_remote_skills(&config.general.workspace).await?;

    config.agents.push(crate::config::AgentConfig {
        id: name.to_string(),
        workspace: workspace.clone(),
        default: false,
        model: None,
        fallback_model: None,
        approval: crate::config::ApprovalConfig::default(),
    });
    config.save(config_path)?;

    println!("Agent '{}' created at {}", name, workspace.display());
    println!("Edit SOUL.md: catclaw agent edit {} soul", name);

    Ok(())
}

fn cmd_agent_list(config: &Config) {
    if config.agents.is_empty() {
        println!("No agents configured.");
        return;
    }
    for agent in &config.agents {
        let default_marker = if agent.default { " (default)" } else { "" };
        println!(
            "  {}{} — {}",
            agent.id,
            default_marker,
            agent.workspace.display()
        );
    }
}

fn cmd_agent_edit(config: &Config, name: &str, file: &str) -> Result<()> {
    let agent = config.agents.iter().find(|a| a.id == name).ok_or_else(|| {
        crate::error::CatClawError::Agent(format!("agent '{}' not found", name))
    })?;

    let filename = match file.to_lowercase().as_str() {
        "soul" => "SOUL.md",
        "user" => "USER.md",
        "identity" => "IDENTITY.md",
        "agents" => "AGENTS.md",
        "tools" => "TOOLS.md",
        "boot" => "BOOT.md",
        "heartbeat" => "HEARTBEAT.md",
        "memory" => "MEMORY.md",
        _ => {
            return Err(crate::error::CatClawError::Agent(format!(
                "unknown file '{}'. Use: soul, user, identity, agents, tools, boot, heartbeat, memory",
                file
            )));
        }
    };

    let path = agent.workspace.join(filename);
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());

    std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .map_err(|e| {
            crate::error::CatClawError::Other(format!("failed to open editor: {}", e))
        })?;

    Ok(())
}

async fn cmd_agent_tools(
    config: &mut Config,
    _config_path: &std::path::Path,
    name: &str,
    allow: Option<String>,
    deny: Option<String>,
    approve: Option<String>,
) -> Result<()> {
    let agent = config.agents.iter().find(|a| a.id == name).ok_or_else(|| {
        crate::error::CatClawError::Agent(format!("agent '{}' not found", name))
    })?;

    let tools_path = agent.workspace.join("tools.toml");

    if allow.is_none() && deny.is_none() && approve.is_none() {
        // Show current tools
        if let Ok(content) = std::fs::read_to_string(&tools_path) {
            println!("{}", content);
        } else {
            println!("No tools.toml found for agent '{}'", name);
        }
        return Ok(());
    }

    // Read existing tools.toml, merge changes, write back
    let existing = std::fs::read_to_string(&tools_path).unwrap_or_default();
    let parsed = toml::from_str::<toml::Value>(&existing).ok();

    let cur_allowed: Vec<String> = parsed.as_ref()
        .and_then(|v| v.get("allowed")?.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()))
        .unwrap_or_default();
    let cur_denied: Vec<String> = parsed.as_ref()
        .and_then(|v| v.get("denied")?.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()))
        .unwrap_or_default();
    let cur_approval: Vec<String> = parsed.as_ref()
        .and_then(|v| v.get("require_approval")?.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()))
        .unwrap_or_default();

    let final_allowed = if let Some(ref a) = allow {
        a.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    } else { cur_allowed };
    let final_denied = if let Some(ref d) = deny {
        d.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    } else { cur_denied };
    let final_approval: Vec<String> = if let Some(ref ap) = approve {
        ap.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    } else { cur_approval };

    let fmt_list = |v: &[String]| -> String {
        v.iter().map(|t| format!("\"{}\"", t)).collect::<Vec<_>>().join(", ")
    };
    let mut content = format!("allowed = [{}]\ndenied = [{}]\n", fmt_list(&final_allowed), fmt_list(&final_denied));
    if !final_approval.is_empty() {
        content.push_str(&format!("require_approval = [{}]\n", fmt_list(&final_approval)));
    }
    std::fs::write(&tools_path, content)?;

    // Notify gateway to hot-reload (best-effort, ignore if gateway not running)
    let ws_url = format!("ws://127.0.0.1:{}/ws", config.general.port);
    if let Ok((client, _event_rx)) = crate::ws_client::GatewayClient::connect(&ws_url, &config.general.ws_token).await {
        match client.request("agents.reload_tools", serde_json::json!({"agent_id": name})).await {
            Ok(_) => println!("Updated tools for agent '{}' (applied immediately)", name),
            Err(_) => println!("Updated tools for agent '{}' (restart gateway to apply)", name),
        }
    } else {
        println!("Updated tools for agent '{}' (gateway not running, will apply on next start)", name);
    }

    Ok(())
}

fn find_agent_workspace<'a>(config: &'a Config, agent_name: &str) -> Result<&'a std::path::PathBuf> {
    config
        .agents
        .iter()
        .find(|a| a.id == agent_name)
        .map(|a| &a.workspace)
        .ok_or_else(|| crate::error::CatClawError::Agent(format!("agent '{}' not found", agent_name)))
}

fn cmd_skill_list(config: &Config, agent_name: &str) -> Result<()> {
    let workspace = find_agent_workspace(config, agent_name)?;
    let skills = AgentLoader::list_skills(workspace, &config.general.workspace);

    if skills.is_empty() {
        println!("No skills in shared pool.");
        return Ok(());
    }

    println!("Skills (agent '{}'):\n", agent_name);
    for s in &skills {
        let status = if s.is_enabled { "on " } else { "off" };
        let desc = if s.description.is_empty() { String::new() } else { format!(" — {}", s.description) };
        println!("  [{}] {}{}", status, s.name, desc);
    }
    Ok(())
}

fn cmd_skill_toggle(config: &Config, agent_name: &str, skill_name: &str, enabled: bool) -> Result<()> {
    let workspace = find_agent_workspace(config, agent_name)?;
    AgentLoader::set_skill_enabled(workspace, &config.general.workspace, skill_name, enabled)?;
    let action = if enabled { "enabled" } else { "disabled" };
    println!("Skill '{}' {} for agent '{}'.", skill_name, action, agent_name);
    Ok(())
}

fn cmd_skill_add(config: &Config, _agent_name: &str, skill_name: &str) -> Result<()> {
    AgentLoader::create_skill(&config.general.workspace, skill_name)?;
    println!("Skill '{}' created in shared pool.", skill_name);
    let skill_path = config.general.workspace.join("skills").join(skill_name).join("SKILL.md");
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    let _ = std::process::Command::new(&editor).arg(&skill_path).status();
    Ok(())
}

async fn cmd_skill_install(config: &Config, _agent_name: &str, source_str: &str) -> Result<()> {
    use crate::agent::SkillSource;
    let source = SkillSource::parse(source_str)?;
    let label = match &source {
        SkillSource::Anthropic(name) => format!("@anthropic/{}", name),
        SkillSource::GitHub { owner, repo, path } => format!("github:{}/{}/{}", owner, repo, path),
        SkillSource::Local(p) => p.display().to_string(),
    };
    println!("Installing skill from {} into shared pool ...", label);
    AgentLoader::install_skill(&config.general.workspace, &source).await?;
    let name = match &source {
        SkillSource::Anthropic(n) => n.clone(),
        SkillSource::GitHub { path, .. } => path.rsplit('/').next().unwrap_or(path).to_string(),
        SkillSource::Local(p) => p.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_string(),
    };
    println!("Skill '{}' installed.", name);
    Ok(())
}

fn cmd_skill_uninstall(config: &Config, _agent_name: &str, skill_name: &str) -> Result<()> {
    AgentLoader::uninstall_skill(&config.general.workspace, skill_name)?;
    println!("Skill '{}' removed from shared pool.", skill_name);
    Ok(())
}

fn cmd_task_list(state_db: &StateDb) -> Result<()> {
    let tasks = state_db.list_scheduled_tasks()?;
    if tasks.is_empty() {
        println!("No scheduled tasks.");
        return Ok(());
    }

    println!(
        "{:<4} {:<25} {:<10} {:<8} {:<20} {}",
        "ID", "NAME", "AGENT", "STATUS", "NEXT RUN", "SCHEDULE"
    );
    println!("{}", "-".repeat(85));

    for t in &tasks {
        let status = if t.enabled { "on" } else { "off" };
        let schedule = if let Some(ref cron) = t.cron_expr {
            format!("cron: {}", cron)
        } else if let Some(mins) = t.interval_mins {
            if mins >= 1440 {
                format!("every {}d", mins / 1440)
            } else if mins >= 60 {
                format!("every {}h", mins / 60)
            } else {
                format!("every {}m", mins)
            }
        } else {
            "one-shot".to_string()
        };

        // Format next_run_at to local-ish display
        let next = &t.next_run_at[..19].replace('T', " ");

        println!(
            "{:<4} {:<25} {:<10} {:<8} {:<20} {}",
            t.id,
            truncate_str(&t.name, 24),
            truncate_str(&t.agent_id, 9),
            status,
            next,
            schedule,
        );
    }

    Ok(())
}

fn cmd_task_add(
    state_db: &StateDb,
    name: &str,
    agent: &str,
    prompt: &str,
    in_mins: Option<u64>,
    cron: Option<String>,
    every: Option<i64>,
) -> Result<()> {
    let now = chrono::Utc::now();

    // Determine schedule type and next_run_at
    let (cron_expr, interval_mins, next_run_at) = if let Some(ref cron_str) = cron {
        // Validate cron expression
        let parsed = croner::Cron::new(cron_str).parse().map_err(|e| {
            crate::error::CatClawError::Config(format!("invalid cron expression: {}", e))
        })?;
        let next = parsed.find_next_occurrence(&now, false).map_err(|e| {
            crate::error::CatClawError::Config(format!("cannot compute next cron run: {}", e))
        })?;
        (Some(cron_str.clone()), None, next.to_rfc3339())
    } else if let Some(mins) = every {
        // Interval-based, first run = in_mins from now or immediately
        let offset = in_mins.unwrap_or(mins as u64);
        let next = now + chrono::Duration::minutes(offset as i64);
        (None, Some(mins), next.to_rfc3339())
    } else {
        // One-shot
        let offset = in_mins.unwrap_or(0);
        let next = now + chrono::Duration::minutes(offset as i64);
        (None, None, next.to_rfc3339())
    };

    let id = state_db.insert_task(&crate::state::ScheduledTaskRow {
        id: 0,
        task_type: "prompt".to_string(),
        agent_id: agent.to_string(),
        name: name.to_string(),
        description: None,
        cron_expr,
        interval_mins,
        next_run_at: next_run_at.clone(),
        last_run_at: None,
        enabled: true,
        payload: Some(prompt.to_string()),
    })?;

    let next_display = &next_run_at[..19].replace('T', " ");
    println!("Task #{} created: \"{}\"", id, name);
    println!("  Agent: {}", agent);
    println!("  Next run: {} UTC", next_display);

    if cron.is_some() {
        println!("  Schedule: cron");
    } else if every.is_some() {
        println!("  Schedule: every {} min", every.unwrap());
    } else {
        println!("  Schedule: one-shot");
    }

    Ok(())
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

fn cmd_agent_delete(config: &mut Config, config_path: &PathBuf, name: &str) -> Result<()> {
    let idx = config
        .agents
        .iter()
        .position(|a| a.id == name)
        .ok_or_else(|| {
            crate::error::CatClawError::Agent(format!("agent '{}' not found", name))
        })?;

    let agent = config.agents.remove(idx);
    config.save(config_path)?;

    println!(
        "Agent '{}' removed from config. Workspace at {} was NOT deleted.",
        name,
        agent.workspace.display()
    );
    println!("Delete manually if needed: rm -rf {}", agent.workspace.display());

    Ok(())
}
