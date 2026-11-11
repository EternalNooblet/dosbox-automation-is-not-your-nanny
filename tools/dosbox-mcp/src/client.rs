use crate::cfg::McpConfig;
use crate::error::{McpError, Result};
use reqwest::{Client, ClientBuilder, Response};
use serde::de::DeserializeOwned;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub struct DosBoxClient {
    client: Client,
    base_url: String,
    timeout: Duration,
    // Canonicalized root confining mount_drive host paths (see cfg.rs).
    mount_root: PathBuf,
}

impl DosBoxClient {
    pub fn new(config: McpConfig) -> Result<Self> {
        let timeout = Duration::from_secs(config.timeout_secs);

        let client = ClientBuilder::new()
            .timeout(timeout)
            .danger_accept_invalid_certs(!config.verify_tls)
            // dosbox-automation's httplib server closes connections after
            // responding; pooled reuse then fails on send ("error sending
            // request"). Disable reuse: one fresh connection per request.
            .pool_max_idle_per_host(0)
            .build()
            .map_err(McpError::Connection)?;

        let base_url = config.base_url.trim_end_matches('/').to_string();
        let mount_root = effective_mount_root(config.mount_root);
        info!("mount_drive host paths confined to: {:?}", mount_root);

        Ok(Self {
            client,
            base_url,
            timeout,
            mount_root,
        })
    }

    /// The root mount_drive host paths must resolve under. Resolved once
    /// here (dot-dot chains and symlinks folded) so per-call checks only
    /// need to resolve the candidate.
    pub fn mount_root(&self) -> PathBuf {
        self.mount_root.clone()
    }

    /// Build the full URL for an API endpoint
    fn url(&self, path: &str) -> Result<String> {
        let path = path.trim_start_matches('/');
        let url = format!("{}/api/v1/{}", self.base_url, path);
        Ok(url)
    }

    /// Execute a GET request and parse JSON response
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = self.url(path)?;
        debug!("GET {}", url);

        // dosbox-automation returns JSON only when accept: application/json
        // is set; without it GET endpoints reply raw binary and deserialization
        // fails ("error decoding response body").
        let response = self.client.get(&url).header("accept", "application/json").send().await?;
        self.handle_response(response).await
    }

    /// Execute a POST request with JSON body and parse JSON response
    pub async fn post<T: DeserializeOwned, B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = self.url(path)?;
        debug!("POST {} with body: {}", url, serde_json::to_string(body).unwrap_or_default());

        let response = self.client.post(&url).json(body).send().await?;
        self.handle_response(response).await
    }

    /// Execute a PUT request with JSON body and parse JSON response
    pub async fn put<T: DeserializeOwned, B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = self.url(path)?;
        debug!("PUT {} with body: {}", url, serde_json::to_string(body).unwrap_or_default());

        let response = self.client.put(&url).json(body).send().await?;
        self.handle_response(response).await
    }

    /// Execute a DELETE request (no body)
    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = self.url(path)?;
        debug!("DELETE {}", url);

        let response = self.client.delete(&url).send().await?;
        self.handle_response(response).await
    }

    /// Execute a DELETE request with JSON body
    pub async fn delete_with_body<T: DeserializeOwned, B: serde::Serialize>(&self, path: &str, body: &B) -> Result<T> {
        let url = self.url(path)?;
        debug!("DELETE {} with body: {}", url, serde_json::to_string(body).unwrap_or_default());

        let response = self.client.delete(&url).json(body).send().await?;
        self.handle_response(response).await
    }

    /// Execute a POST request with a raw text body (for Lua scripts,
    /// which the emulator expects as text/plain).
    pub async fn post_text<T: DeserializeOwned>(&self, path: &str, body: &str) -> Result<T> {
        let url = self.url(path)?;
        debug!("POST {} (text, {} bytes)", url, body.len());
        let response = self.client.post(&url)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Accept", "application/json")
            .body(body.to_string())
            .send().await?;
        self.handle_response(response).await
    }

    /// Execute a POST request that returns raw bytes (for frame capture)
    pub async fn post_bytes<B: serde::Serialize>(&self, path: &str, body: &B) -> Result<Vec<u8>> {
        let url = self.url(path)?;
        debug!("POST {} (bytes)", url);

        let response = self.client.post(&url).json(body).send().await?;
        let status = response.status();

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            error!("API error {}: {}", status, text);
            return Err(McpError::Http {
                status: status.as_u16(),
                message: text,
            });
        }

        let bytes = response.bytes().await.map_err(McpError::Connection)?;
        Ok(bytes.to_vec())
    }

    /// Execute a GET request that returns raw bytes
    pub async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let url = self.url(path)?;
        debug!("GET {} (bytes)", url);

        let response = self.client.get(&url).send().await?;
        let status = response.status();

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            error!("API error {}: {}", status, text);
            return Err(McpError::Http {
                status: status.as_u16(),
                message: text,
            });
        }

        let bytes = response.bytes().await.map_err(McpError::Connection)?;
        Ok(bytes.to_vec())
    }

    async fn handle_response<T: DeserializeOwned>(&self, response: Response) -> Result<T> {
        let status = response.status();

        if status.is_success() {
            let body = response.json().await?;
            Ok(body)
        } else {
            let text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            error!("API error {}: {}", status, text);
            Err(McpError::Http {
                status: status.as_u16(),
                message: text,
            })
        }
    }

    /// Health check - ping the server
    pub async fn health_check(&self) -> Result<()> {
        self.get::<serde_json::Value>("/status").await?;
        Ok(())
    }
}

/// Root that confines mount_drive host paths: the owner-configured root
/// (config file / DOSBOX_MCP_MOUNT_ROOT / --mount-root) if present,
/// otherwise the plugin data directory. Canonicalized once here so the
/// per-call check only resolves the candidate.
pub fn effective_mount_root(configured: Option<PathBuf>) -> PathBuf {
    let raw = if let Some(path) = configured {
        path
    } else {
        return plugin_data_dir_or_unrestricted()
    };
    // A root that does not exist yet keeps its unresolved form; containment
    // against it is still well-defined and MOUNT refuses missing directories
    // anyway.
    raw.canonicalize().unwrap_or_else(|_| raw)
}

/// Default mount root: the companion plugin's data directory, mirroring
/// data_root()'s fallback chain — HERMES_HOME wins, else the OS HOME env
/// plus "/.hermes" — then plugin-data/dosbox-full-auto. With neither
/// variable set there is no plugin data dir to confine to: return "/"
/// (unrestricted) and say so once.
fn plugin_data_dir_or_unrestricted() -> PathBuf {
    let hermes_home = std::env::var("HERMES_HOME").unwrap_or_default();
    let plain = std::env::var("HOME").unwrap_or_default();
    let home = if !hermes_home.is_empty() {
        hermes_home
    } else if !plain.is_empty() {
        plain + "/.hermes"
    } else {
        "".to_string()
    };
    if home.is_empty() {
        warn!("mount_root not configured and no HERMES_HOME/HOME to locate the plugin data dir; mount_drive is unrestricted");
        return PathBuf::from("/");
    }
    PathBuf::from(home).join("plugin-data").join("dosbox-full-auto")
}