// This file is part of the dosbox-automation-is-not-your-nanny fork
// License: GPL-2.0-or-later. This file is original work of the fork. Contact: EternalNooblet <67196335+EternalNooblet@users.noreply.github.com>.

use anyhow::Result;
use clap::Parser;
use config::{Config, File, Environment};
use dirs::config_dir;
use rmcp::{ServiceExt, transport::io::stdio};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, fmt};

mod client;
mod cfg;
mod dosbox_launcher;
mod error;
mod server;
mod types;

use crate::cfg::McpConfig;
use crate::server::DosBoxMcpServer;
use crate::dosbox_launcher::DosBoxLauncher;

#[derive(Parser, Debug, Clone)]
#[command(name = "dosbox-mcp", version, about = "MCP server for dosbox-automation REST API")]
struct Args {
    /// Base URL of dosbox-automation API (e.g., http://localhost:8386)
    #[arg(long, env = "DOSBOX_API_URL")]
    base_url: Option<String>,

    /// Request timeout in seconds (default: 30)
    #[arg(long, default_value = "30")]
    timeout_secs: u64,

    /// Disable TLS verification (for self-signed certs)
    #[arg(long)]
    no_verify_tls: bool,

    /// Config file path (default: ~/.config/dosbox-mcp/config.toml)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Path to dosbox-automation binary (auto-detect if not specified)
    #[arg(long, env = "DOSBOX_AUTOMATION_BIN")]
    dosbox_bin: Option<PathBuf>,

    /// Auto-start dosbox-automation if not running (default: true)
    #[arg(long, default_value = "true")]
    auto_start: bool,

    /// Don't auto-start dosbox-automation
    #[arg(long, conflicts_with = "auto_start")]
    no_auto_start: bool,

    /// Working directory for dosbox-automation
    #[arg(long)]
    working_dir: Option<PathBuf>,

    /// Extra arguments to pass to dosbox-automation
    #[arg(last = true)]
    dosbox_args: Vec<String>,

    /// CPU cycles setting (e.g., "30000", "auto", "max")
    #[arg(long, env = "DOSBOX_CPU_CYCLES")]
    cpu_cycles: Option<String>,

    /// Enable secure mode (blocks MOUNT, default: false)
    #[arg(long, default_value = "false")]
    secure_mode: bool,

    /// X11 display to use (e.g., ":99" for Xvfb). If not set, uses offscreen SDL driver.
    #[arg(long, env = "DOSBOX_DISPLAY")]
    display: Option<String>,

    /// Root that mount_drive host paths must resolve under after dot-dots
    /// and symlinks. Default when unset: the plugin data directory
    /// (<HERMES_HOME or HOME/.hermes>/plugin-data/dosbox-full-auto).
    #[arg(long, env = "DOSBOX_MCP_MOUNT_ROOT")]
    mount_root: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Determine auto-start setting
    let auto_start = args.auto_start && !args.no_auto_start;

    // Initialize tracing
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(args.log_level.clone()));
    // Logs MUST go to stderr: stdout is the MCP stdio transport and any
    // log line there corrupts JSON-RPC framing for the client.
    fmt().with_env_filter(filter).with_writer(std::io::stderr).init();

    info!("Starting dosbox-mcp server");

    // Load configuration
    let config = load_config(args.clone())?;

    // Create dosbox launcher if auto-start is enabled
    let dosbox_launcher = if auto_start {
        let launcher = DosBoxLauncher::new(
            args.dosbox_bin.clone(),
            args.working_dir.clone(),
            args.dosbox_args.clone(),
            config.base_url.clone(),
            config.cpu_cycles.clone(),
            config.secure_mode,
            config.display.clone(),
        )?;
        Some(launcher)
    } else {
        None
    };

    // Start dosbox-automation if launcher is configured
    let base_url = if let Some(ref launcher) = dosbox_launcher {
        info!("Starting dosbox-automation...");
        launcher.start().await?
    } else {
        config.base_url.clone()
    };

    // Create the dosbox client
    let client = Arc::new(client::DosBoxClient::new(McpConfig {
        base_url: base_url.clone(),
        timeout_secs: config.timeout_secs,
        verify_tls: config.verify_tls,
        cpu_cycles: config.cpu_cycles.clone(),
        secure_mode: config.secure_mode,
        display: config.display.clone(),
        mount_root: config.mount_root.clone(),
    })?);

    // Create and run the MCP server with stdio transport
    let server = DosBoxMcpServer::new(client);
    let service = server.serve(stdio()).await?;

    // Wait for the server to finish
    service.waiting().await?;

    // Stop dosbox-automation if we started it
    if let Some(launcher) = dosbox_launcher {
        info!("Stopping dosbox-automation...");
        launcher.stop().await?;
    }

    Ok(())
}

fn load_config(args: Args) -> Result<McpConfig> {
    let mut builder = Config::builder();

    // 1. Config file (lowest priority)
    let config_path = args.config.or_else(|| {
        config_dir().map(|dir| dir.join("dosbox-mcp").join("config.toml"))
    });

    if let Some(path) = &config_path {
        if path.exists() {
            builder = builder.add_source(File::from(path.clone()).required(false));
            info!("Loading config from: {:?}", path);
        }
    }

    // 2. Environment variables
    builder = builder.add_source(Environment::with_prefix("DOSBOX_MCP").separator("_"));

    // 3. CLI args (highest priority) - manually override after building
    let mut config: McpConfig = builder.build()?.try_deserialize()?;

    // Apply CLI overrides
    if let Some(url) = args.base_url {
        config.base_url = url;
    }
    config.timeout_secs = args.timeout_secs;
    config.verify_tls = !args.no_verify_tls;
    if let Some(cycles) = args.cpu_cycles {
        config.cpu_cycles = Some(cycles);
    }
    if args.secure_mode {
        config.secure_mode = true;
    }
    if let Some(display) = args.display {
        config.display = Some(display);
    }
    if let Some(root) = args.mount_root {
        config.mount_root = Some(root);
    }

    // Validate
    if config.base_url.trim().is_empty() {
        anyhow::bail!("base_url is required (use --base-url or DOSBOX_API_URL env var)");
    }

    info!("Configuration loaded: base_url={}, timeout={}s, verify_tls={}",
          config.base_url, config.timeout_secs, config.verify_tls);

    Ok(config)
}