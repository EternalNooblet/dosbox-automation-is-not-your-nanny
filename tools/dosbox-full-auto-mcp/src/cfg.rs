// This file is part of the dosbox-automation-is-not-your-nanny fork
// License: GPL-2.0-or-later. This file is original work of the fork. Contact: EternalNooblet <67196335+EternalNooblet@users.noreply.github.com>.

use config::{Config, File, Environment};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    /// Base URL of dosbox-automation API (e.g., "http://localhost:8386")
    #[serde(default = "default_base_url")]
    pub base_url: String,

    /// Request timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// Verify TLS certificates
    #[serde(default = "default_verify_tls")]
    pub verify_tls: bool,

    /// CPU cycles, pinned for both real and protected mode (default "30000").
    /// Protected mode must be pinned explicitly: the emulator's own default
    /// protected rate is the hardcoded 60000 constant, which silently takes
    /// over once a DOS extender switches to pmode.
    #[serde(default = "default_cpu_cycles")]
    pub cpu_cycles: Option<String>,

    /// Enable secure mode (blocks MOUNT)
    #[serde(default)]
    pub secure_mode: bool,

    /// X11 display to use (e.g., ":99" for Xvfb). If not set, uses offscreen SDL driver.
    #[serde(default)]
    pub display: Option<String>,

    /// Root that mount_drive host paths must live under. The host path is
    /// checked after resolving ".." and symlinks, so it cannot escape via
    /// a symlink or a "../" chain; control characters (newlines included)
    /// are rejected outright because the path is typed into the DOS shell.
    /// Default when unset: the plugin data directory under the Hermes home
    /// (see plugin_data_dir_or_unrestricted() in client.rs).
    #[serde(default)]
    pub mount_root: Option<PathBuf>,
}

fn default_base_url() -> String {
    "http://localhost:8386".to_string()
}

fn default_timeout() -> u64 {
    30
}

fn default_verify_tls() -> bool {
    true
}

fn default_cpu_cycles() -> Option<String> {
    Some("30000".to_string())
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            timeout_secs: default_timeout(),
            verify_tls: default_verify_tls(),
            cpu_cycles: default_cpu_cycles(),
            secure_mode: false,
            display: None,
            mount_root: None,
        }
    }
}

impl McpConfig {
    pub fn load(config_path: Option<PathBuf>) -> anyhow::Result<Self> {
        let mut builder = Config::builder();

        if let Some(path) = config_path {
            if path.exists() {
                builder = builder.add_source(File::from(path).required(false));
            }
        } else if let Some(config_dir) = dirs::config_dir() {
            let default_path = config_dir.join("dosbox-full-auto-mcp").join("config.toml");
            if default_path.exists() {
                builder = builder.add_source(File::from(default_path).required(false));
            }
        }

        builder = builder.add_source(Environment::with_prefix("DOSBOX_FULL_AUTO_MCP").separator("_"));

        let config: Self = builder.build()?.try_deserialize()?;
        Ok(config)
    }
}