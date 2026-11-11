// This file is part of the dosbox-automation-is-not-your-nanny fork
// License: GPL-2.0-or-later. This file is original work of the fork. Contact: EternalNooblet <67196335+EternalNooblet@users.noreply.github.com>.

use crate::client::DosBoxClient;
use crate::error::McpError;
use crate::types::*;
use rmcp::{
    Error as McpErrorInternal, handler::server::{
        router::tool::ToolRouter,
        tool::Parameters,
    },
    model::*,
    schemars, tool, tool_handler, tool_router,
    ServerHandler,
    model::{Resource, ResourceTemplate, Prompt, PromptArgument, PromptMessage, PromptMessageRole, ResourceContents, PaginatedRequestParam, ListResourcesResult, ListResourceTemplatesResult, ReadResourceRequestParam, ReadResourceResult, GetPromptRequestParam, GetPromptResult, ListPromptsResult, RawResource, RawResourceTemplate},
    service::{RoleServer, RequestContext},
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

/// Convert a '1'/'0' page-flag string into coalesced [start,end_excl)
/// page ranges so callers don't have to parse a ~48 KB flag string.
fn flag_ranges(s: &str) -> Vec<[u64; 2]> {
    let mut out = Vec::new();
    let mut start: Option<u64> = None;
    for (i, ch) in s.chars().enumerate() {
        // Emulator emits raw 0x01/0x00 bytes (JSON \u0001), not '1'/'0'.
        let on = ch == '1' || ch == '\u{1}';
        match (on, start) {
            (true, None) => start = Some(i as u64),
            (false, Some(st)) => {
                out.push([st, i as u64]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(st) = start {
        out.push([st, s.chars().count() as u64]);
    }
    out
}

/// Parse an emulator raw frame (header: w u32, h u32, pitch i32,
/// pixel_format u8, pal_count u16, optional palette, then pixels) and
/// compute black ratio / luma stats / a simple FNV-1a hash. Pixel
/// formats (PixelFormat enum): 8=Indexed8, 15=RGB555, 16=RGB565,
/// 24=BGR24, 32=BGRX32.
fn analyze_raw_frame(bytes: &[u8]) -> Result<FrameAnalysis, String> {
    if bytes.len() < 15 {
        return Err(format!("raw frame too short: {} bytes", bytes.len()));
    }
    let w = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let h = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let pitch = i32::from_le_bytes(bytes[8..12].try_into().unwrap()).abs() as usize;
    let pf = bytes[12];
    let pal_count = u16::from_le_bytes(bytes[13..15].try_into().unwrap()) as usize;
    let header = 15 + pal_count * 3;
    if bytes.len() < header + pitch * h {
        return Err(format!(
            "raw frame truncated: {} bytes, need {}",
            bytes.len(),
            header + pitch * h
        ));
    }
    let palette = &bytes[15..header];
    let pixels = &bytes[header..header + pitch * h];
    let bpp: usize = match pf {
        8 => 1,
        15 | 16 => 2,
        24 => 3,
        32 => 4,
        _ => return Err(format!("unknown pixel format {pf}")),
    };
    let pf_name = match pf {
        8 => "Indexed8",
        15 => "RGB555_Packed16",
        16 => "RGB565_Packed16",
        24 => "BGR24_ByteArray",
        32 => "BGRX32_ByteArray",
        _ => "Unknown",
    };
    let mut black: usize = 0;
    let mut total: usize = 0;
    let mut sum: u64 = 0;
    let mut min_l = 255u8;
    let mut max_l = 0u8;
    let mut hash: u64 = 0xcbf29ce484222325;
    for y in 0..h {
        let row = &pixels[y * pitch..y * pitch + w * bpp];
        for px in row.chunks_exact(bpp) {
            let (r, g, b): (u8, u8, u8) = match pf {
                8 => {
                    let idx = px[0] as usize * 3;
                    if idx + 2 < palette.len() {
                        (palette[idx], palette[idx + 1], palette[idx + 2])
                    } else {
                        (0, 0, 0)
                    }
                }
                15 => {
                    let v = u16::from_le_bytes([px[0], px[1]]);
                    (
                        (((v >> 10) & 0x1F) * 255 / 31) as u8,
                        (((v >> 5) & 0x1F) * 255 / 31) as u8,
                        ((v & 0x1F) * 255 / 31) as u8,
                    )
                }
                16 => {
                    let v = u16::from_le_bytes([px[0], px[1]]);
                    (
                        (((v >> 11) & 0x1F) * 255 / 31) as u8,
                        (((v >> 5) & 0x3F) * 255 / 63) as u8,
                        ((v & 0x1F) * 255 / 31) as u8,
                    )
                }
                24 => (px[2], px[1], px[0]),
                _ => (px[2], px[1], px[0]),
            };
            let luma = ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8;
            if luma == 0 {
                black += 1;
            }
            sum += luma as u64;
            min_l = min_l.min(luma);
            max_l = max_l.max(luma);
            hash ^= luma as u64;
            hash = hash.wrapping_mul(0x100000001b3);
            total += 1;
        }
    }
    let black_ratio = if total > 0 { black as f64 / total as f64 } else { 1.0 };
    Ok(FrameAnalysis {
        width: w as u32,
        height: h as u32,
        pixel_format: pf_name.to_string(),
        black_ratio,
        mean_luma: if total > 0 { sum as f64 / total as f64 } else { 0.0 },
        min_luma: min_l,
        max_luma: max_l,
        hash: format!("{hash:016x}"),
        all_black: black_ratio > 0.999,
    })
}

// Convert to error with context
fn to_mcp_error_ctx(endpoint: &str, err: McpError) -> McpErrorInternal {
    McpErrorInternal::internal_error(format!("{} - {}", endpoint, err), None)
}

/// MCP server for dosbox-automation
#[derive(Clone)]
pub struct DosBoxMcpServer {
    client: Arc<DosBoxClient>,
    tool_router: ToolRouter<Self>,
    /// Serializes input-injection bursts; the emulator rejects a new burst
    /// with HTTP 409 while its replay chain is still draining.
    input_gate: Arc<AsyncMutex<()>>,
}

impl DosBoxMcpServer {
    pub fn new(client: Arc<DosBoxClient>) -> Self {
        Self {
            client,
            tool_router: Self::tool_router(),
            input_gate: Arc::new(AsyncMutex::new(())),
        }
    }

    /// POST an input payload through the burst queue: holds the gate so
    /// concurrent tool calls serialize, and retries on 409 ("Replay already
    /// in progress") until the emulator's pending chain completes.
    async fn post_input<T: DeserializeOwned, B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, McpErrorInternal> {
        let _gate = self.input_gate.lock().await;
        const MAX_ATTEMPTS: usize = 60; // ~30s of waiting at 500ms backoff
        for attempt in 0..MAX_ATTEMPTS {
            match self.client.post(path, body).await {
                Ok(result) => return Ok(result),
                Err(McpError::Http { status: 409, .. }) => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                Err(e) => return Err(to_mcp_error_ctx(path, e)),
            }
        }
        Err(McpErrorInternal::internal_error(
            format!("{} - emulator input queue busy after {} retries", path, MAX_ATTEMPTS),
            None,
        ))
    }
}

// ============ mount_drive path confinement ============

/// Guard for the mount_drive host_path, enforced BEFORE anything is typed at
/// the shell. MOUNT runs whatever the model asks for, unapproved, so:
/// 1. control characters (newline included) are rejected outright — a
///    newline in the path would run the rest of it as extra DOS commands;
/// 2. the path must be absolute and exist (MOUNT fails on missing dirs
///    anyway, but the reason here is specific instead of shell noise);
/// 3. containment is checked AFTER resolving dot-dot chains and symlinks,
///    so neither "../" hops nor a symlinked directory can reach e.g. the
///    owner's Hermes .env through the plugin's own mount root.
/// Returns the rejection reason, or None when the path is acceptable.
pub fn mount_path_rejection(host_path: &str, root: PathBuf) -> Option<String> {
    for ch in host_path.chars() {
        // Control chars: C0 + DEL + C1. Comparisons on char code points.
        if ch < ' ' || ch == '\u{7f}' || (ch >= '\u{80}' && ch <= '\u{9f}') {
            return Some("host_path must not contain control characters (a newline would execute extra DOS shell commands)".to_string())
        }
    }
    let raw = PathBuf::from(host_path);
    if !raw.is_absolute() {
        return Some(format!("host_path must be absolute (got: {:?})", host_path));
    }
    if !raw.exists() {
        return Some(format!("host_path must be an existing directory (got: {:?})", host_path));
    }
    if host_path_outside_root(host_path, root) {
        return Some(format!(
            "host_path must resolve (after dot-dots and symlinks) under the configured mount root (got: {:?})",
            host_path,
        ));
    }
    None
}

/// True when `candidate` equals `root` or lies strictly inside it. Both
/// sides are expected fully resolved (canonicalize on the candidate, a
/// canonicalized root from DosBoxClient). Compares resolved spellings
/// character by character; the root prefix must be followed by a path
/// separator so "/usr" never matches "/usrshare". Roots spelled with a
/// drive colon or leading backslash are Windows-shaped and accept either
/// separator after the prefix; other roots require the forward slash.
pub fn under_root(candidate: PathBuf, root: PathBuf) -> bool {
    let c = candidate.to_string_lossy();
    let root_str = root.to_string_lossy();
    let r = root_str.trim_end_matches('/').trim_end_matches('\\');
    let mut ci = c.chars();
    for rc in r.chars() {
        match ci.next() {
            Some(cc) => {
                if cc != rc {
                    return false;
                }
            }
            None => { return false; }
        }
    }
    match ci.next() {
        // candidate == root: mounting the root itself is allowed
        None => true,
        Some(nch) => {
            if nch == '/' {
                return true;
            }
            // Windows spelling is decided from the root itself (drive colon
            // or leading backslash); see root_uses_windows_separators.
            let windows = root_uses_windows_separators(root);
            windows && nch == '\\'
        }
    }
}

/// True when the root's own spelling is Windows-shaped (drive prefix like
/// "C:" or UNC leading backslash). Used to decide which token shapes count
/// as absolute host paths and which separators are honored after the root
/// prefix.
pub fn root_uses_windows_separators(root: PathBuf) -> bool {
    let s = root.to_string_lossy();
    let r = s.trim_end_matches('/').trim_end_matches('\\');
    let mut it = r.chars();
    match it.next() {
        Some(c) => {
            if c == '\\' {
                true
            } else {
                match it.next() {
                    Some(c2) => c.is_ascii_alphabetic() && c2 == ':',
                    None => false,
                }
            }
        }
        None => false,
    }
}

/// True when a token (from typed shell text) looks like an absolute host
/// path. On Unix hosts only the "/" shape counts — a DOS drive path like
/// "C:\GAME" is a drive mapping inside the emulator, not a host path, and
/// rejecting those would break legitimate typed commands. On Windows hosts
/// host paths ARE drive/UNC shapes, so those count too.
pub fn looks_like_absolute_host_path(token: &str, windows_shape: bool) -> bool {
    let mut it = token.chars();
    match it.next() {
        Some(c) => {
            if c == '/' {
                true
            } else if windows_shape {
                match it.next() {
                    Some(c2) => (c.is_ascii_alphabetic() && c2 == ':') || c2 == '\\',
                    None => false,
                }
            } else {
                false
            }
        }
        None => false,
    }
}

/// Confined-path core: a token is outside the root when it does not exist
/// (a missing path cannot be proven inside anything, and both the shell
/// MOUNT and the API mount refuse missing paths anyway) or when its
/// resolved spelling — dot-dots and symlinks folded — lies outside the
/// root. Exists-then-resolved keeps the containment check honest.
pub fn host_path_outside_root(token: &str, root: PathBuf) -> bool {
    let raw = PathBuf::from(token);
    if !raw.exists() {
        return true
    }
    let resolved = raw.canonicalize().unwrap_or_else(|_| raw);
    !under_root(resolved, root)
}

/// Rejection reason for the swap_disk image: API-mounted (no shell, so
/// control characters are not a shell-smuggling hazard here), but it is
/// still an unapproved host path — same absolute/existing/inside-root
/// rules as mount_drive.
pub fn swap_image_rejection(image: &str, root: PathBuf) -> Option<String> {
    let raw = PathBuf::from(image);
    if !raw.is_absolute() {
        return Some(format!("image must be an absolute path (got: {:?})", image));
    }
    if !raw.exists() {
        return Some(format!("image path must exist on the host (got: {:?})", image));
    }
    if host_path_outside_root(image, root) {
        return Some(format!("image must resolve (after .. and symlinks) inside the configured mount root (got: {:?})", image));
    }
    None
}

/// Heuristic guard over free-form typed text: scan each line; only lines
/// whose FIRST word is MOUNT (any case) are inspected, and only tokens
/// that look like absolute host paths (see looks_like_absolute_host_path)
/// are checked — so relative paths (which the emulator resolves against
/// its own cwd) and in-game sentences are left alone. A quoted path is
/// one token even with spaces. This cannot follow drive mappings or multi
/// -line batch state: it is a shell-input guard, not a DOS parser.
pub fn typed_mount_violation(text: &str, root: PathBuf) -> Option<String> {
    let windows = root_uses_windows_separators(root.clone());
    let mut line = String::new();
    for ch in text.chars() {
        if ch == '\n' {
            let v = check_mount_line(line, root.clone(), windows);
            line = String::new();
            if v != None {
                return v
            }
        } else {
            line.push(ch);
        }
    }
    check_mount_line(line, root.clone(), windows)
}

/// One line of typed shell text -> violation reason or None.
pub fn check_mount_line(line: String, root: PathBuf, windows: bool) -> Option<String> {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quote = ' ';
    for ch in line.chars() {
        if quote != ' ' {
            if ch == quote {
                quote = ' '
            } else {
                cur.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            quote = ch;
        } else if ch == ' ' || ch == '\t' || ch == '\r' {
            if !cur.is_empty() {
                words.push(cur);
                cur = String::new();
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    if words.is_empty() {
        return None;
    }
    if words[0].to_ascii_uppercase() != "MOUNT" {
        return None;
    }
    let n = words.len();
    let mut i = 1;
    while i < n {
        let w = &words[i];
        if looks_like_absolute_host_path(w, windows) && host_path_outside_root(w, root.clone()) {
            return Some(format!("the typed line references a path outside the mount root (line: {:?}) — it must exist and resolve inside it", line));
        }
        i = i + 1;
    }
    None
}

#[tool_router]
impl DosBoxMcpServer {
    // ============ Status Tools ============

    #[tool(description = "Get current DOSBox status including program state and CPU info. is_booted is normalized: true once the DOS shell is up (the emulator's raw flag only tracks the BOOT command).")]
    async fn get_status(&self) -> Result<CallToolResult, McpErrorInternal> {
        let mut status: Status = self.client.get("/status").await.map_err(|e| to_mcp_error_ctx("/status", e))?;
        status.is_booted = status.is_booted || status.is_shell;
        let content = Content::json(status).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Get the current program execution state. is_booted is normalized: true once the DOS shell is up (the emulator's raw flag only tracks the BOOT command).")]
    async fn get_program_state(&self) -> Result<CallToolResult, McpErrorInternal> {
        let mut state: ProgramState = self.client.get("/program/state").await.map_err(|e| to_mcp_error_ctx("/program/state", e))?;
        state.is_booted = state.is_booted || state.is_shell;
        let content = Content::json(state).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Get DOSBox version and build information")]
    async fn get_dosbox_info(&self) -> Result<CallToolResult, McpErrorInternal> {
        let info: DosBoxInfo = self.client.get("/dosbox/info").await.map_err(|e| to_mcp_error_ctx("/dosbox/info", e))?;
        let content = Content::json(info).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ Control Tools ============

    #[tool(description = "Shutdown DOSBox")]
    async fn shutdown(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: ControlResponse = self.client.post("/control/shutdown", &json!({})).await.map_err(|e| to_mcp_error_ctx("/control/shutdown", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Pause DOSBox emulation")]
    async fn pause(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: ControlResponse = self.client.post("/control/pause", &json!({})).await.map_err(|e| to_mcp_error_ctx("/control/pause", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Resume DOSBox emulation")]
    async fn resume(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: ControlResponse = self.client.post("/control/resume", &json!({})).await.map_err(|e| to_mcp_error_ctx("/control/resume", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Get current pause state")]
    async fn get_pause_state(&self) -> Result<CallToolResult, McpErrorInternal> {
        // The API returns a boolean or simple object, use generic for now
        let state: serde_json::Value = self.client.get("/control/paused").await.map_err(|e| to_mcp_error_ctx("/control/paused", e))?;
        let content = Content::json(state).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ CPU Tools ============

    #[tool(description = "Get CPU register values")]
    async fn get_registers(&self) -> Result<CallToolResult, McpErrorInternal> {
        // The emulator exposes registers under GET /cpu/state as { registers: Registers }
        let response: serde_json::Value = self.client.get("/cpu/state").await.map_err(|e| to_mcp_error_ctx("/cpu/state", e))?;
        let regs = response.get("registers").cloned().unwrap_or(response);
        let content = Content::json(regs).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Get current CPU cycles setting")]
    async fn get_cycles(&self) -> Result<CallToolResult, McpErrorInternal> {
        let cycles: CpuCycles = self.client.get("/cpu/cycles").await.map_err(|e| to_mcp_error_ctx("/cpu/cycles", e))?;
        let content = Content::json(cycles).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Set CPU cycles for both real and protected mode (integer, 50-2000000)")]
    async fn set_cycles(&self, Parameters(params): Parameters<SetCyclesParams>) -> Result<CallToolResult, McpErrorInternal> {
        let body = json!({ "cycles": params.cycles });
        let result: SetCyclesResponse = self.client.put("/cpu/cycles", &body).await.map_err(|e| to_mcp_error_ctx("/cpu/cycles", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Set a CPU register value")]
    async fn set_register(&self, Parameters(params): Parameters<SetRegisterParams>) -> Result<CallToolResult, McpErrorInternal> {
        let body = json!({ "register": params.register, "value": params.value });
        let result: ControlResponse = self.client.put("/cpu/register", &body).await.map_err(|e| to_mcp_error_ctx("/cpu/register", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ Memory Tools ============

    #[tool(description = "Read memory at offset (max 128 MiB per request). Returns base64-encoded data.")]
    async fn read_memory(&self, Parameters(params): Parameters<ReadMemoryParams>) -> Result<CallToolResult, McpErrorInternal> {
        if params.length > 128 * 1024 * 1024 {
            return Err(McpErrorInternal::invalid_params("max 128 MiB per request", None));
        }
        let path = format!("/memory/{}/{}", params.offset, params.length);
        if let Some(save_path) = &params.save_path {
            // Binary mode (no accept header) — emulator streams raw bytes.
            let bytes = self.client.get_bytes(&path).await.map_err(|e| to_mcp_error_ctx(&path, e))?;
            let size = bytes.len();
            std::fs::write(save_path, bytes).map_err(|e| McpErrorInternal::internal_error(format!("write failed: {e}"), None))?;
            let content = Content::json(json!({ "addr": params.offset, "saved_path": save_path, "size": size }))
                .map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
            return Ok(CallToolResult::success(vec![content]));
        }
        let raw: EmuMemoryRead = self.client.get(&path).await.map_err(|e| to_mcp_error_ctx(&path, e))?;
        let data = MemoryReadResponse { addr: raw.memory.addr, data: raw.memory.data.unwrap_or_default() };
        let content = Content::json(data).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Write memory at offset (max 128 MiB per request). 'data' accepts either a base64 string (e.g. \"AQID\") or an array of byte values (e.g. [1,2,3]).")]
    async fn write_memory(&self, Parameters(params): Parameters<WriteMemoryParams>) -> Result<CallToolResult, McpErrorInternal> {
        let data_bytes = params.data_bytes().map_err(|e| McpErrorInternal::invalid_params(e, None))?;
        if data_bytes.len() > 128 * 1024 * 1024 {
            return Err(McpErrorInternal::invalid_params("max 128 MiB per request", None));
        }
        let path = format!("/memory/{}", params.offset);
        let body = json!({ "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &data_bytes) });
        let raw: EmuMemoryWrite = self.client.put(&path, &body).await.map_err(|e| to_mcp_error_ctx(&path, e))?;
        let result = MemoryWriteResponse { addr: raw.memory.addr };
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Search memory for a value (byte/word/dword). Max 16 MiB per request.")]
    async fn search_memory(&self, Parameters(params): Parameters<SearchMemoryParams>) -> Result<CallToolResult, McpErrorInternal> {
        let end = params.start.unwrap_or(0) + params.length.unwrap_or(16 * 1024 * 1024) as u64;
        let body = json!({
            "value": params.value,
            "width": params.width,
            "start": params.start.unwrap_or(0),
            "end": end,
        });
        let result: MemorySearchResponse = self.client.post("/memory/search", &body).await.map_err(|e| to_mcp_error_ctx("/memory/search", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Get the physical 4 KB page map: which pages the memory API may read/write (plain host RAM/ROM only). Returns 'readable'/'writable' strings of '1'/'0' indexed by page number, plus total_pages and page_size.")]
    async fn get_page_map(&self) -> Result<CallToolResult, McpErrorInternal> {
        let mut result: serde_json::Value = self.client.get("/memory/pagemap").await.map_err(|e| to_mcp_error_ctx("/memory/pagemap", e))?;
        // Add coalesced ranges alongside the raw flag strings so the
        // readable/writable page sets are usable without parsing ~48 KB.
        if let Some(obj) = result.as_object_mut() {
            if let Some(r) = obj.get("readable").and_then(|v| v.as_str()).map(|s| s.to_string()) {
                let ranges = flag_ranges(&r);
                obj.insert("readable_ranges".into(), json!(ranges));
            }
            if let Some(w) = obj.get("writable").and_then(|v| v.as_str()).map(|s| s.to_string()) {
                let ranges = flag_ranges(&w);
                obj.insert("writable_ranges".into(), json!(ranges));
            }
        }
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Allocate memory in DOSBox")]
    async fn allocate_memory(&self, Parameters(params): Parameters<AllocateMemoryParams>) -> Result<CallToolResult, McpErrorInternal> {
        let body = json!({ "size": params.size, "area": params.area, "strategy": params.strategy });
        let result: MemoryAllocateResponse = self.client.post("/memory/allocate", &body).await.map_err(|e| to_mcp_error_ctx("/memory/allocate", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Free previously allocated memory")]
    async fn free_memory(&self, Parameters(params): Parameters<FreeMemoryParams>) -> Result<CallToolResult, McpErrorInternal> {
        let body = json!({ "addr": params.addr });
        let result: MemoryFreeResponse = self.client.post("/memory/free", &body).await.map_err(|e| to_mcp_error_ctx("/memory/free", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ Freeze Memory Tools ============

    #[tool(description = "Set a trainer-style value freeze (pins memory to fixed value each frame)")]
    async fn freeze_memory(&self, Parameters(params): Parameters<FreezeMemoryParams>) -> Result<CallToolResult, McpErrorInternal> {
        let body = json!({ "address": params.address, "value": params.value, "width": params.width });
        let result: FreezeResponse = self.client.post("/memory/freeze", &body).await.map_err(|e| to_mcp_error_ctx("/memory/freeze", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "List active value freezes")]
    async fn list_freezes(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: ListFreezesResponse = self.client.get("/memory/freeze").await.map_err(|e| to_mcp_error_ctx("/memory/freeze", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Remove freezes. With no address, clears all freezes.")]
    async fn clear_freezes(&self, Parameters(params): Parameters<ClearFreezesParams>) -> Result<CallToolResult, McpErrorInternal> {
        let body = if let Some(addr) = params.address {
            json!({ "address": addr })
        } else {
            json!({})
        };
        let result: DeleteFreezesResponse = self.client.delete_with_body("/memory/freeze", &body).await.map_err(|e| to_mcp_error_ctx("/memory/freeze", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ Input Tools ============

    #[tool(description = "Type text into DOSBox at specified characters per second (default: 30 cps). Lines typed as MOUNT with an absolute host path are root-confined like mount_drive (default root: the plugin data dir); relative paths are left to the emulator's own cwd. Bursts are queued internally: if a previous burst is still playing, this call waits for it instead of failing.")]
    async fn type_text(&self, Parameters(params): Parameters<TypeTextParams>) -> Result<CallToolResult, McpErrorInternal> {
        let root = self.client.mount_root();
        if let Some(reason) = typed_mount_violation(&params.text, root.clone()) {
            return Err(McpErrorInternal::invalid_params(
                format!("typed MOUNT rejected: {} (allowed root: {})", reason, root.to_string_lossy()), None));
        }
        let body = json!({ "text": params.text, "cps": params.cps });
        let result: TypeTextResponse = self.post_input("/input/type", &body).await?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Send a sequence of input events (keys, mouse, wheel) with optional timing. Bursts are queued internally: if a previous burst is still playing, this call waits for it instead of failing.")]
    async fn send_input_sequence(&self, Parameters(params): Parameters<SendInputSequenceParams>) -> Result<CallToolResult, McpErrorInternal> {
        let body = json!({ "events": params.events });
        let result: InputSequenceResponse = self.post_input("/input/sequence", &body).await?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ Video/Frame Tools ============

    #[tool(description = "Capture a frame (raw or rendered) as PNG/JPEG/raw. Returns base64-encoded image, or with save_path writes the file to disk on the MCP server host and returns saved_path instead of data.")]
    async fn capture_frame(&self, Parameters(params): Parameters<CaptureFrameParams>) -> Result<CallToolResult, McpErrorInternal> {
        let path = format!("/video/frame?mode={}&format={}&quality={}", params.mode, params.format, params.quality);
        let bytes = self.client.get_bytes(&path).await.map_err(|e| to_mcp_error_ctx(&path, e))?;

        let saved_path = match params.save_path.as_ref() {
            Some(p) => {
                std::fs::write(p, &bytes)
                    .map_err(|e| McpErrorInternal::internal_error(format!("write failed for {}: {}", p, e), None))?;
                Some(p.clone())
            }
            None => None,
        };

        let data = if saved_path.is_some() {
            String::new()
        } else {
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes)
        };
        // Best-effort dimensions from the frame-info endpoint; a failure
        // here must not fail the capture itself.
        let (width, height) = match self.client.get::<FrameInfo>("/video/frame/info").await {
            Ok(info) => (Some(info.width), Some(info.height)),
            Err(_) => (None, None),
        };
        let result = CaptureFrameResult {
            data,
            size: bytes.len(),
            format: params.format,
            mode: params.mode,
            width,
            height,
            saved_path,
        };
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Get frame dimensions and format info, including the video-mode block (graphics vs text, BIOS mode, double-scan).")]
    async fn get_frame_info(&self) -> Result<CallToolResult, McpErrorInternal> {
        let info: FrameInfoFull = self.client.get("/video/frame/info").await.map_err(|e| to_mcp_error_ctx("/video/frame/info", e))?;
        let content = Content::json(info).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Read text-mode screen content (useful for driving installers)")]
    async fn read_screen_text(&self) -> Result<CallToolResult, McpErrorInternal> {
        let text: ScreenText = self.client.get("/video/text").await.map_err(|e| to_mcp_error_ctx("/video/text", e))?;
        let content = Content::json(text).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ Recording Tools ============

    #[tool(description = "Start recording input sequence")]
    async fn start_recording(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: ControlResponse = self.client.post("/input/record/start", &json!({})).await.map_err(|e| to_mcp_error_ctx("/input/record/start", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Pause/resume input recording")]
    async fn pause_recording(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: ControlResponse = self.client.post("/input/record/pause", &json!({})).await.map_err(|e| to_mcp_error_ctx("/input/record/pause", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Stop input recording and return recorded data")]
    async fn stop_recording(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: RecordingResult = self.client.post("/input/record/stop", &json!({})).await.map_err(|e| to_mcp_error_ctx("/input/record/stop", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Get current recording status")]
    async fn get_recording_status(&self) -> Result<CallToolResult, McpErrorInternal> {
        let status: RecordingStatus = self.client.get("/input/record/status").await.map_err(|e| to_mcp_error_ctx("/input/record/status", e))?;
        let content = Content::json(status).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ Drive Tools ============

    #[tool(description = "Swap floppy/CD image in a drive. The image path is confined like mount_drive: absolute, existing, and inside the configured mount root (default: the plugin data directory).")]
    async fn swap_disk(&self, Parameters(params): Parameters<SwapDiskParams>) -> Result<CallToolResult, McpErrorInternal> {
        let root = self.client.mount_root();
        if let Some(reason) = swap_image_rejection(&params.image, root.clone()) {
            return Err(McpErrorInternal::invalid_params(
                format!("image rejected: {} (allowed root: {})", reason, root.to_string_lossy()), None));
        }
        let body = json!({ "drive": params.drive, "image": params.image });
        let result: DriveSwapResponse = self.client.post("/drive/swap", &body).await.map_err(|e| to_mcp_error_ctx("/drive/swap", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Lock or unlock drive mounts. lock=true refuses all further mounts; lock=false re-allows them (automation endpoint only).")]
    async fn lock_mounts(&self, Parameters(params): Parameters<LockMountsParams>) -> Result<CallToolResult, McpErrorInternal> {
        let body = json!({ "locked": params.lock });
        let result: MountLockResponse = self.client.post("/mount/lock", &body).await.map_err(|e| to_mcp_error_ctx("/mount/lock", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Get current mount lock state")]
    async fn get_mount_lock(&self) -> Result<CallToolResult, McpErrorInternal> {
        let state: MountLockStatus = self.client.get("/mount/lock").await.map_err(|e| to_mcp_error_ctx("/mount/lock", e))?;
        let content = Content::json(state).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ Script Tools ============

    #[tool(description = "Load a Lua script")]
    async fn load_script(&self, Parameters(params): Parameters<LoadScriptParams>) -> Result<CallToolResult, McpErrorInternal> {
        // The API takes the script as text/plain body with query params.
        // Sending it as JSON (the old behaviour) is rejected with 415.
        let path = format!("/script/load{}", params.name.as_ref().map(|n| format!("?name={}", urlencoding::encode(n))).unwrap_or_default());
        let result: ScriptLoadResponse = self.client.post_text(&path, &params.script).await.map_err(|e| to_mcp_error_ctx(&path, e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Start executing loaded Lua script")]
    async fn start_script(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: ControlResponse = self.client.post("/script/start", &json!({})).await.map_err(|e| to_mcp_error_ctx("/script/start", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Stop running Lua script")]
    async fn stop_script(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: ControlResponse = self.client.post("/script/stop", &json!({})).await.map_err(|e| to_mcp_error_ctx("/script/stop", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Get Lua script execution status")]
    async fn get_script_status(&self) -> Result<CallToolResult, McpErrorInternal> {
        let status: ScriptStatus = self.client.get("/script/status").await.map_err(|e| to_mcp_error_ctx("/script/status", e))?;
        let content = Content::json(status).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ Capture Tools ============

    #[tool(description = "Start video capture (ZMBV)")]
    async fn start_video_capture(&self, Parameters(params): Parameters<StartVideoCaptureParams>) -> Result<CallToolResult, McpErrorInternal> {
        let body = json!({
            "mode": params.mode,
        });
        let result: CaptureStartResponse = self.client.post("/capture/video/start", &body).await.map_err(|e| to_mcp_error_ctx("/capture/video/start", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Stop video capture")]
    async fn stop_video_capture(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: ControlResponse = self.client.post("/capture/video/stop", &json!({})).await.map_err(|e| to_mcp_error_ctx("/capture/video/stop", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Get video capture status")]
    async fn get_capture_status(&self) -> Result<CallToolResult, McpErrorInternal> {
        let status: CaptureStatus = self.client.get("/capture/video/status").await.map_err(|e| to_mcp_error_ctx("/capture/video/status", e))?;
        let content = Content::json(status).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Get video capture compression settings")]
    async fn get_compression(&self) -> Result<CallToolResult, McpErrorInternal> {
        let comp: CaptureCompression = self.client.get("/capture/video/compression").await.map_err(|e| to_mcp_error_ctx("/capture/video/compression", e))?;
        let content = Content::json(comp).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Set video capture compression settings")]
    async fn set_compression(&self, Parameters(params): Parameters<SetCompressionParams>) -> Result<CallToolResult, McpErrorInternal> {
        let mut body = serde_json::Map::new();
        if let Some(raw) = params.raw {
            body.insert("raw".to_string(), json!(raw));
        }
        if let Some(rendered) = params.rendered {
            body.insert("rendered".to_string(), json!(rendered));
        }
        let result: CaptureCompression = self.client.put("/capture/video/compression", &body).await.map_err(|e| to_mcp_error_ctx("/capture/video/compression", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ I/O Tools ============

    #[tool(description = "Read I/O port (byte or word)")]
    async fn read_port(&self, Parameters(params): Parameters<ReadPortParams>) -> Result<CallToolResult, McpErrorInternal> {
        let path = format!("/io/port?port={}&width={}", params.port, params.width);
        let result: PortReadResponse = self.client.get(&path).await.map_err(|e| to_mcp_error_ctx(&path, e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Write I/O port (byte or word)")]
    async fn write_port(&self, Parameters(params): Parameters<WritePortParams>) -> Result<CallToolResult, McpErrorInternal> {
        let body = json!({ "port": params.port, "value": params.value, "width": params.width });
        let result: PortWriteResponse = self.client.put("/io/port", &body).await.map_err(|e| to_mcp_error_ctx("/io/port", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ DOS Tools ============

    #[tool(description = "Get DOS internals (MCB map, etc.)")]
    async fn get_dos_internals(&self) -> Result<CallToolResult, McpErrorInternal> {
        let internals: DosInternals = self.client.get("/dos/internals").await.map_err(|e| to_mcp_error_ctx("/dos/internals", e))?;
        let content = Content::json(internals).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ Mixer Tools (previously missing from MCP) ============

    #[tool(description = "Get mixer status: master volume, mute state, sample rate, and all channels")]
    async fn get_mixer_status(&self) -> Result<CallToolResult, McpErrorInternal> {
        let status: MixerStatus = self.client.get("/mixer").await.map_err(|e| to_mcp_error_ctx("/mixer", e))?;
        let content = Content::json(status).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Set master mixer volume (left/right, non-negative; 1.0 = unity)")]
    async fn set_mixer_volume(&self, Parameters(params): Parameters<MixerVolumeParams>) -> Result<CallToolResult, McpErrorInternal> {
        let body = json!({ "left": params.left, "right": params.right });
        let result: MixerResult = self.client.put("/mixer/volume", &body).await.map_err(|e| to_mcp_error_ctx("/mixer/volume", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Mute the mixer")]
    async fn mute_mixer(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: MixerResult = self.client.post("/mixer/mute", &json!({})).await.map_err(|e| to_mcp_error_ctx("/mixer/mute", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Unmute the mixer")]
    async fn unmute_mixer(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: MixerResult = self.client.post("/mixer/unmute", &json!({})).await.map_err(|e| to_mcp_error_ctx("/mixer/unmute", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Set a single mixer channel volume by name (e.g. \"PC Speaker\", \"AdLib\"); get names from get_mixer_status")]
    async fn set_mixer_channel_volume(&self, Parameters(params): Parameters<MixerChannelParams>) -> Result<CallToolResult, McpErrorInternal> {
        let path = format!("/mixer/channel/{}", urlencoding::encode(&params.name));
        let body = json!({ "left": params.left, "right": params.right });
        let result: MixerResult = self.client.put(&path, &body).await.map_err(|e| to_mcp_error_ctx(&path, e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ Audio Capture Tools (previously missing from MCP) ============

    #[tool(description = "Start audio capture (WAV). Optional host path to write to; empty means the emulator default.")]
    async fn start_audio_capture(&self, Parameters(params): Parameters<AudioCaptureParams>) -> Result<CallToolResult, McpErrorInternal> {
        let body = match params.path.as_ref() {
            Some(p) if !p.is_empty() => json!({ "path": p }),
            _ => json!({}),
        };
        let result: AudioCaptureResult = self.client.post("/capture/audio/start", &body).await.map_err(|e| to_mcp_error_ctx("/capture/audio/start", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Stop audio capture")]
    async fn stop_audio_capture(&self) -> Result<CallToolResult, McpErrorInternal> {
        let result: AudioCaptureResult = self.client.post("/capture/audio/stop", &json!({})).await.map_err(|e| to_mcp_error_ctx("/capture/audio/stop", e))?;
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Get audio capture status")]
    async fn get_audio_capture_status(&self) -> Result<CallToolResult, McpErrorInternal> {
        let status: AudioCaptureStatus = self.client.get("/capture/audio/status").await.map_err(|e| to_mcp_error_ctx("/capture/audio/status", e))?;
        let content = Content::json(status).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    // ============ Mouse / Mount / Frame-analysis Tools (new) ============

    #[tool(description = "Get the last injected mouse cursor position in API coordinates. Exact for injected moves; best-effort if the guest clamps/recentres the cursor.")]
    async fn get_mouse_position(&self) -> Result<CallToolResult, McpErrorInternal> {
        let pos: MousePosition = self.client.get("/input/mouse").await.map_err(|e| to_mcp_error_ctx("/input/mouse", e))?;
        let content = Content::json(pos).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Mount a host directory as a DOS drive by typing MOUNT at the shell, then verify via read_screen_text. host_path must be an absolute, existing directory that resolves (after dot-dots and symlinks) under the configured mount root (default: the plugin data directory); control characters are rejected so a newline cannot smuggle extra DOS commands. Returns the screen text after the command so callers can confirm success. Drive letter like \"C\" or \"D\".")]
    async fn mount_drive(&self, Parameters(params): Parameters<MountDriveParams>) -> Result<CallToolResult, McpErrorInternal> {
        let drive = params.drive.trim().to_ascii_uppercase();
        if drive.len() != 1 || !drive.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
            return Err(McpErrorInternal::invalid_params("drive must be a single letter, e.g. \"C\"", None));
        }
        // Confine the host path before anything is typed: the root comes from
        // owner config (default: plugin data dir); containment is checked
        // after resolving .. and symlinks.
        let root = self.client.mount_root();
        if let Some(reason) = mount_path_rejection(&params.host_path, root.clone()) {
            return Err(McpErrorInternal::invalid_params(
                format!("host_path rejected: {} (allowed root: {})", reason, root.to_string_lossy()), None));
        }
        let cmd_line = format!("MOUNT {} {}\n", drive, params.host_path);
        let _typed: TypeTextResponse = self.post_input("/input/type", &json!({ "text": cmd_line, "cps": params.cps.unwrap_or(30.0) })).await?;
        // Give the shell a moment and read the screen back for verification.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let screen: ScreenText = self.client.get("/video/text").await.map_err(|e| to_mcp_error_ctx("/video/text", e))?;
        let result = json!({
            "drive": drive,
            "host_path": params.host_path,
            "screen_text": screen.text,
            "is_text_mode": screen.is_text_mode,
            "note": "check screen_text for the MOUNT result line; in graphics mode screen_text is empty",
        });
        let content = Content::json(result).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }

    #[tool(description = "Analyse the current frame without returning pixels: black ratio, luma stats, and a stable hash so callers can detect change / implement the black-watch loop in one call. Works in graphics and text modes.")]
    async fn analyze_frame(&self) -> Result<CallToolResult, McpErrorInternal> {
        let path = "/video/frame?mode=raw&format=raw&quality=85";
        let bytes = self.client.get_bytes(path).await.map_err(|e| to_mcp_error_ctx(path, e))?;
        let analysis = analyze_raw_frame(&bytes).map_err(|e| McpErrorInternal::internal_error(e, None))?;
        let content = Content::json(analysis).map_err(|e| McpErrorInternal::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![content]))
    }
}

// ============ Parameter Types ============

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MixerVolumeParams {
    /// Left volume (non-negative; 1.0 = unity)
    pub left: f32,
    /// Right volume (non-negative; 1.0 = unity)
    pub right: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MixerChannelParams {
    /// Channel name as returned by get_mixer_status
    pub name: String,
    /// Left volume (non-negative; 1.0 = unity)
    pub left: f32,
    /// Right volume (non-negative; 1.0 = unity)
    pub right: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AudioCaptureParams {
    /// Optional host path for the WAV file; omit for the emulator default
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MountDriveParams {
    /// DOS drive letter, e.g. "C" or "D"
    pub drive: String,
    /// Host directory to mount, e.g. "/home/papa/games/doom". Must be an
    /// absolute, existing directory that resolves (after .. and symlinks)
    /// under the configured mount root (default: the plugin data dir); no
    /// control characters.
    pub host_path: String,
    /// Typing speed in characters per second (default 30)
    #[serde(default)]
    pub cps: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetCyclesParams {
    /// CPU cycles value applied to both real and protected mode (integer, 50-2000000)
    pub cycles: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetRegisterParams {
    /// Register name (e.g., "eax", "ebx", "eip")
    pub register: String,
    /// Value to set
    pub value: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadMemoryParams {
    /// Physical memory offset
    pub offset: u64,
    /// Number of bytes to read (max 128 MiB)
    pub length: u32,
    /// Optional absolute path: write raw bytes to this file instead of
    /// returning base64. Returns saved_path + size instead of data.
    #[serde(default)]
    pub save_path: Option<String>,
}

/// Bytes to write: either a base64 string or an array of byte values.
/// The emulator wire format is base64; accepting both shapes keeps the
/// tool usable from prompt-style JSON (where a string is natural) and
/// from callers that already hold a byte array.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum ByteData {
    Base64(String),
    Bytes(Vec<u8>),
}

impl ByteData {
    fn to_bytes(&self) -> Result<Vec<u8>, String> {
        match self {
            ByteData::Bytes(b) => Ok(b.clone()),
            ByteData::Base64(s) => base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                s,
            )
            .map_err(|e| format!("data is not valid base64: {e}")),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WriteMemoryParams {
    /// Physical memory offset
    pub offset: u64,
    /// Data to write: base64 string (e.g. "AQID") or byte array ([1,2,3])
    pub data: ByteData,
}

impl WriteMemoryParams {
    fn data_bytes(&self) -> Result<Vec<u8>, String> {
        self.data.to_bytes()
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchMemoryParams {
    /// Value to search for
    pub value: u64,
    /// Width in bytes: 1 (byte), 2 (word), 4 (dword)
    #[serde(default = "default_width")]
    pub width: u8,
    /// Start address (default: 0)
    #[serde(default)]
    pub start: Option<u64>,
    /// Number of bytes to scan (max 16 MiB)
    #[serde(default)]
    pub length: Option<u32>,
}

fn default_width() -> u8 {
    1
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AllocateMemoryParams {
    /// Size in bytes
    pub size: u32,
    /// Memory area: "CONV", "UMB", "XMS", "EMS"
    #[serde(default = "default_area")]
    pub area: String,
    /// Allocation strategy: "BEST_FIT", "FIRST_FIT", "LAST_FIT"
    #[serde(default = "default_strategy")]
    pub strategy: String,
}

fn default_area() -> String {
    "CONV".to_string()
}

fn default_strategy() -> String {
    "BEST_FIT".to_string()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FreeMemoryParams {
    /// Physical address of allocated block
    pub addr: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FreezeMemoryParams {
    /// Physical address to freeze
    pub address: u64,
    /// Value to freeze
    pub value: u64,
    /// Width in bytes: 1, 2, or 4
    #[serde(default = "default_width")]
    pub width: u8,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ClearFreezesParams {
    /// Specific address to clear (omit to clear all)
    pub address: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TypeTextParams {
    /// Text to type
    pub text: String,
    /// Characters per second (default: 30)
    #[serde(default = "default_cps")]
    pub cps: f32,
}

fn default_cps() -> f32 {
    30.0
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SendInputSequenceParams {
    /// Array of input events
    pub events: Vec<InputEvent>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CaptureFrameParams {
    /// Frame source: "raw" (emulator framebuffer) or "rendered" (post-shader)
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Output format: "png", "jpeg", "raw"
    #[serde(default = "default_format")]
    pub format: String,
    /// JPEG quality 1-100 (default: 85)
    #[serde(default = "default_quality")]
    pub quality: u8,
    /// Optional absolute path: write the image bytes to this file instead of
    /// returning base64. When set, `data` is empty and `saved_path` is set.
    #[serde(default)]
    pub save_path: Option<String>,
}

fn default_mode() -> String {
    "raw".to_string()
}

fn default_format() -> String {
    "png".to_string()
}

fn default_quality() -> u8 {
    85
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SwapDiskParams {
    /// Drive letter (e.g., "A", "C")
    pub drive: String,
    /// Path to disk image. Must be absolute, exist on the host, and resolve
    /// (after .. and symlinks) inside the configured mount root (default:
    /// the plugin data directory).
    pub image: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LockMountsParams {
    /// Lock state
    pub lock: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LoadScriptParams {
    /// Lua script source code
    pub script: String,
    /// Optional name for the script
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StartVideoCaptureParams {
    /// Capture mode: "raw" or "rendered"
    #[serde(default = "default_mode")]
    pub mode: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetCompressionParams {
    /// Compression level for raw mode (0-9)
    pub raw: Option<u8>,
    /// Compression level for rendered mode (0-9)
    pub rendered: Option<u8>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadPortParams {
    /// Port number (0-65535)
    pub port: u16,
    /// Width: 1 for byte, 2 for word
    #[serde(default = "default_width")]
    pub width: u8,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WritePortParams {
    /// Port number (0-65535)
    pub port: u16,
    /// Value to write
    pub value: u32,
    /// Width: 1 for byte, 2 for word
    #[serde(default = "default_width")]
    pub width: u8,
}

// ============ ServerHandler Implementation ============

// ----- Documentation Resources -----
const API_REFERENCE_MD: &str = include_str!("../docs/api-reference.md");
const CONFIG_GUIDE_MD: &str = include_str!("../docs/configuration.md");
const WORKFLOWS_MD: &str = include_str!("../docs/workflows.md");
const QUICKSTART_MD: &str = include_str!("../docs/quickstart.md");

fn doc_resource(uri: &str, name: &str, description: &str, content: &str) -> Resource {
    let raw = RawResource {
        uri: uri.to_string(),
        name: name.to_string(),
        description: Some(description.to_string()),
        mime_type: Some("text/markdown".to_string()),
        size: Some(content.len() as u32),
    };
    Resource::new(raw, None)
}

fn doc_resource_template(uri_template: &str, name: &str, description: &str) -> ResourceTemplate {
    let raw = RawResourceTemplate {
        uri_template: uri_template.to_string(),
        name: name.to_string(),
        description: Some(description.to_string()),
        mime_type: Some("text/markdown".to_string()),
    };
    ResourceTemplate::new(raw, None)
}

#[tool_handler]
impl ServerHandler for DosBoxMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "MCP server for dosbox-automation REST API. Provides tools for controlling DOSBox instances including input injection, memory access, video capture, drive management, and Lua scripting.\n\n\
                **Documentation Resources** (read via `resources/read`):\n\
                - `docs://dosbox-mcp/api-reference` — Complete API reference for all 55 tools\n\
                - `docs://dosbox-mcp/configuration` — Server configuration guide\n\
                - `docs://dosbox-mcp/workflows` — Common workflow examples (game install, memory trainer, testing)\n\
                - `docs://dosbox-mcp/quickstart` — Quick start guide\n\n\
                **Prompts** (invoke via `prompts/get`):\n\
                - `install-game` — Step-by-step game installation from CD/ISO\n\
                - `memory-trainer` — Find and freeze memory values (lives, ammo, etc.)\n\
                - `automated-test` — Record/replay deterministic input sequences\n\
                - `lua-script` — Load and execute Lua automation scripts".into()
            ),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
            ..Default::default()
        }
    }

    // ---- Resources ----
    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpErrorInternal>> + Send + '_ {
        let resources = vec![
            doc_resource(
                "docs://dosbox-mcp/api-reference",
                "API Reference",
                "Complete API reference for all 45 MCP tools with parameters, examples, and error codes",
                API_REFERENCE_MD,
            ),
            doc_resource(
                "docs://dosbox-mcp/configuration",
                "Configuration Guide",
                "Server configuration: config file, CLI options, environment variables, multi-instance setup",
                CONFIG_GUIDE_MD,
            ),
            doc_resource(
                "docs://dosbox-mcp/workflows",
                "Workflow Examples",
                "Common workflows: game installation, memory trainer, automated testing, Lua scripting",
                WORKFLOWS_MD,
            ),
            doc_resource(
                "docs://dosbox-mcp/quickstart",
                "Quick Start Guide",
                "Get running in 3 steps: install, configure MCP client, start server",
                QUICKSTART_MD,
            ),
        ];
        std::future::ready(Ok(ListResourcesResult {
            resources,
            next_cursor: None,
        }))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, McpErrorInternal>> + Send + '_ {
        let templates = vec![
            doc_resource_template(
                "docs://dosbox-mcp/tool/{tool_name}",
                "Individual Tool Reference",
                "Get detailed reference for a specific tool by name (e.g., 'type_text', 'read_memory')",
            ),
        ];
        std::future::ready(Ok(ListResourceTemplatesResult {
            resource_templates: templates,
            next_cursor: None,
        }))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, McpErrorInternal>> + Send + '_ {
        let uri = request.uri.as_str();
        let contents = match uri {
            "docs://dosbox-mcp/api-reference" => ResourceContents::text(API_REFERENCE_MD, uri),
            "docs://dosbox-mcp/configuration" => ResourceContents::text(CONFIG_GUIDE_MD, uri),
            "docs://dosbox-mcp/workflows" => ResourceContents::text(WORKFLOWS_MD, uri),
            "docs://dosbox-mcp/quickstart" => ResourceContents::text(QUICKSTART_MD, uri),
            uri if uri.starts_with("docs://dosbox-mcp/tool/") => {
                let tool_name = uri.strip_prefix("docs://dosbox-mcp/tool/").unwrap_or("");
                let content = self.get_tool_doc(tool_name);
                ResourceContents::text(content, uri)
            }
            _ => {
                return std::future::ready(Err(McpErrorInternal::resource_not_found(
                    format!("Resource not found: {}", uri),
                    None,
                )));
            }
        };
        std::future::ready(Ok(ReadResourceResult {
            contents: vec![contents],
        }))
    }

    // ---- Prompts ----
    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, McpErrorInternal>> + Send + '_ {
        let prompts = vec![
            Prompt::new(
                "install-game",
                Some("Install a DOS game from CD/ISO image step by step"),
                Some(vec![
                    PromptArgument { name: "drive".into(), description: Some("Drive letter to mount (e.g., 'D')".into()), required: Some(true) },
                    PromptArgument { name: "image_path".into(), description: Some("Path to ISO/IMG disk image".into()), required: Some(true) },
                    PromptArgument { name: "install_cmd".into(), description: Some("Install command to run (e.g., 'd:\\install.exe')".into()), required: Some(false) },
                ]),
            ),
            Prompt::new(
                "memory-trainer",
                Some("Find a value in memory and freeze it (infinite lives, ammo, etc.)"),
                Some(vec![
                    PromptArgument { name: "value".into(), description: Some("Value to search for (e.g., current lives count)".into()), required: Some(true) },
                    PromptArgument { name: "width".into(), description: Some("Search width: 1=byte, 2=word, 4=dword".into()), required: Some(false) },
                    PromptArgument { name: "freeze_value".into(), description: Some("Value to freeze at (e.g., 99 for infinite lives)".into()), required: Some(true) },
                ]),
            ),
            Prompt::new(
                "automated-test",
                Some("Record input sequence, replay deterministically, verify frame"),
                Some(vec![
                    PromptArgument { name: "test_name".into(), description: Some("Name for this test".into()), required: Some(true) },
                    PromptArgument { name: "record_duration_secs".into(), description: Some("How long to record (seconds)".into()), required: Some(false) },
                ]),
            ),
            Prompt::new(
                "lua-script",
                Some("Load and execute a Lua automation script"),
                Some(vec![
                    PromptArgument { name: "script".into(), description: Some("Lua script source code".into()), required: Some(true) },
                    PromptArgument { name: "name".into(), description: Some("Optional name for the script".into()), required: Some(false) },
                ]),
            ),
        ];
        std::future::ready(Ok(ListPromptsResult {
            prompts,
            next_cursor: None,
        }))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResult, McpErrorInternal>> + Send + '_ {
        let name = request.name.as_str();
        let args = request.arguments.unwrap_or_default();

        let result = match name {
            "install-game" => self.prompt_install_game(&args),
            "memory-trainer" => self.prompt_memory_trainer(&args),
            "automated-test" => self.prompt_automated_test(&args),
            "lua-script" => self.prompt_lua_script(&args),
            _ => {
                return std::future::ready(Err(McpErrorInternal::method_not_found::<GetPromptRequestMethod>()));
            }
        };
        std::future::ready(Ok(result))
    }
}

impl DosBoxMcpServer {
    /// Get documentation for a specific tool by name
    fn get_tool_doc(&self, tool_name: &str) -> String {
        match tool_name {
            // Status
            "get_status" => r#"## get_status
Get current DOSBox status including program state and CPU info.

**Parameters:** None

**Returns:** `Status` object with emulator state, CPU info, and program info.

**Example:**
```json
{ "tool": "get_status", "arguments": {} }
```"#,
            "get_program_state" => r#"## get_program_state
Get the current program execution state.

**Parameters:** None

**Returns:** `ProgramState` with running program name, path, and state.

**Example:**
```json
{ "tool": "get_program_state", "arguments": {} }
```"#,
            "get_dosbox_info" => r#"## get_dosbox_info
Get DOSBox version and build information.

**Parameters:** None

**Returns:** `DosBoxInfo` with version, feature flags, binary path.

**Example:**
```json
{ "tool": "get_dosbox_info", "arguments": {} }
```"#,
            // Control
            "shutdown" => r#"## shutdown
Request graceful shutdown of DOSBox.

**Parameters:** None

**Returns:** `ControlResponse` with status.

**Example:**
```json
{ "tool": "shutdown", "arguments": {} }
```"#,
            "pause" => r#"## pause
Pause DOSBox emulation.

**Parameters:** None

**Returns:** `ControlResponse`.

**Example:**
```json
{ "tool": "pause", "arguments": {} }
```"#,
            "resume" => r#"## resume
Resume DOSBox emulation.

**Parameters:** None

**Returns:** `ControlResponse`.

**Example:**
```json
{ "tool": "resume", "arguments": {} }
```"#,
            "get_pause_state" => r#"## get_pause_state
Get current pause state.

**Parameters:** None

**Returns:** Boolean or object indicating paused state.

**Example:**
```json
{ "tool": "get_pause_state", "arguments": {} }
```"#,
            // CPU
            "get_registers" => r#"## get_registers
Get all CPU register values.

**Parameters:** None

**Returns:** `Registers` object with all general-purpose, segment, flag, and FPU registers.

**Example:**
```json
{ "tool": "get_registers", "arguments": {} }
```"#,
            "get_cycles" => r#"## get_cycles
Get current CPU cycles setting.

**Parameters:** None

**Returns:** `CpuCycles` with current cycles value.

**Example:**
```json
{ "tool": "get_cycles", "arguments": {} }
```"#,
            "set_cycles" => r#"## set_cycles
Set CPU cycles.

**Parameters:**
- `cycles` (string): Cycles value — numeric (e.g., "5000"), "auto", or "max"

**Returns:** `SetCyclesResponse`.

**Example:**
```json
{ "tool": "set_cycles", "arguments": { "cycles": "auto" } }
```"#,
            "set_register" => r#"## set_register
Set a CPU register value.

**Parameters:**
- `register` (string): Register name (e.g., "eax", "ebx", "eip", "cs", "ds")
- `value` (u64): Value to set

**Returns:** `ControlResponse`.

**Example:**
```json
{ "tool": "set_register", "arguments": { "register": "eax", "value": 0 } }
```"#,
            // Memory
            "read_memory" => r#"## read_memory
Read memory at offset (max 128 MiB per request). Returns base64-encoded data.

**Parameters:**
- `offset` (u64): Physical memory offset
- `length` (u32): Number of bytes to read (max 134,217,728)

**Returns:** `MemoryReadResponse` with base64 data.

**Example:**
```json
{ "tool": "read_memory", "arguments": { "offset": 0x100000, "length": 4096 } }
```"#,
            "write_memory" => r#"## write_memory
Write memory at offset (max 128 MiB per request). Data is base64-encoded.

**Parameters:**
- `offset` (u64): Physical memory offset
- `data` (Vec<u8>): Base64-encoded data to write (max 134,217,728 bytes)

**Returns:** `MemoryWriteResponse`.

**Example:**
```json
{ "tool": "write_memory", "arguments": { "offset": 0x100000, "data": "AQIDBA==" } }
```"#,
            "search_memory" => r#"## search_memory
Search memory for a value (byte/word/dword). Max 16 MiB per request.

**Parameters:**
- `value` (u64): Value to search for
- `width` (u8, default=1): Width in bytes: 1 (byte), 2 (word), 4 (dword)
- `start` (u64, optional): Start address (default: 0)
- `length` (u32, optional): Bytes to scan (max 16,777,216, default: 16 MiB)

**Returns:** `MemorySearchResponse` with array of matching addresses.

**Example:**
```json
{ "tool": "search_memory", "arguments": { "value": 3, "width": 1 } }
```"#,
            "allocate_memory" => r#"## allocate_memory
Allocate memory in DOSBox (CONV, UMB, XMS, EMS).

**Parameters:**
- `size` (u32): Size in bytes
- `area` (string, default="CONV"): Memory area — "CONV", "UMB", "XMS", "EMS"
- `strategy` (string, default="BEST_FIT"): Allocation strategy — "BEST_FIT", "FIRST_FIT", "LAST_FIT"

**Returns:** `MemoryAllocateResponse` with allocated address.

**Example:**
```json
{ "tool": "allocate_memory", "arguments": { "size": 65536, "area": "XMS" } }
```"#,
            "free_memory" => r#"## free_memory
Free previously allocated memory.

**Parameters:**
- `addr` (u64): Physical address of allocated block

**Returns:** `MemoryFreeResponse`.

**Example:**
```json
{ "tool": "free_memory", "arguments": { "addr": 0x100000 } }
```"#,
            "freeze_memory" => r#"## freeze_memory
Set a trainer-style value freeze (pins memory to fixed value each frame).

**Parameters:**
- `address` (u64): Physical address to freeze
- `value` (u64): Value to freeze
- `width` (u8, default=1): Width in bytes: 1, 2, or 4

**Returns:** `FreezeResponse`.

**Example:**
```json
{ "tool": "freeze_memory", "arguments": { "address": 65536, "value": 99, "width": 1 } }
```"#,
            "list_freezes" => r#"## list_freezes
List active value freezes.

**Parameters:** None

**Returns:** `ListFreezesResponse` with array of active freezes.

**Example:**
```json
{ "tool": "list_freezes", "arguments": {} }
```"#,
            "clear_freezes" => r#"## clear_freezes
Remove freezes. With no address, clears all freezes.

**Parameters:**
- `address` (u64, optional): Specific address to clear (omit to clear all)

**Returns:** `DeleteFreezesResponse`.

**Example:**
```json
{ "tool": "clear_freezes", "arguments": {} }
```"#,
            // Input
            "type_text" => r#"## type_text
Type text into DOSBox at specified characters per second (default: 30 cps).
Bursts are queued server-side: concurrent calls serialize, and a call that
arrives while a previous burst is still playing waits until the emulator
accepts it instead of returning an error.

**Parameters:**
- `text` (string): Text to type (supports escape sequences like \r, \n, \t)
- `cps` (f32, default=30): Characters per second

**Returns:** `TypeTextResponse`.

**Example:**
```json
{ "tool": "type_text", "arguments": { "text": "dir\r", "cps": 20 } }
```"#,
            "send_input_sequence" => r#"## send_input_sequence
Send a sequence of input events (keys, mouse, wheel) with optional timing.

**Parameters:**
- `events` (InputEvent[]): Array of input events

**InputEvent types:**
```json
{ "type": "key", "key": "KBD_enter", "pressed": true, "t": 0 }
{ "type": "mouse_move", "x_rel": 5.0, "y_rel": -3.0 }
{ "type": "mouse_move", "x_abs": 400.0, "y_abs": 300.0 }
{ "type": "mouse_button", "button": "left", "pressed": true }
{ "type": "mouse_wheel", "delta": 1.0 }
```

**Returns:** `InputSequenceResponse`.

**Example:**
```json
{ "tool": "send_input_sequence", "arguments": { "events": [{ "type": "key", "key": "KBD_enter", "pressed": true, "t": 0 }] } }
```"#,
            // Video
            "capture_frame" => r#"## capture_frame
Capture a frame (raw or rendered) as PNG/JPEG/raw. Returns base64-encoded image.

**Parameters:**
- `mode` (string, default="raw"): "raw" (emulator framebuffer) or "rendered" (post-shader)
- `format` (string, default="png"): "png", "jpeg", "raw"
- `quality` (u8, default=85): JPEG quality 1-100

**Returns:** `CaptureFrameResult` with base64 data, size, format, mode.

**Example:**
```json
{ "tool": "capture_frame", "arguments": { "mode": "rendered", "format": "png" } }
```"#,
            "get_frame_info" => r#"## get_frame_info
Get frame dimensions and format info.

**Parameters:** None

**Returns:** `FrameInfo` with width, height, bpp, pitch.

**Example:**
```json
{ "tool": "get_frame_info", "arguments": {} }
```"#,
            "read_screen_text" => r#"## read_screen_text
Read text-mode screen content (useful for driving installers).

**Parameters:** None

**Returns:** `ScreenText` with rows, cols, and text content.

**Example:**
```json
{ "tool": "read_screen_text", "arguments": {} }
```"#,
            // Recording
            "start_recording" => r#"## start_recording
Start recording input sequence.

**Parameters:** None

**Returns:** `ControlResponse`.

**Example:**
```json
{ "tool": "start_recording", "arguments": {} }
```"#,
            "pause_recording" => r#"## pause_recording
Pause/resume input recording.

**Parameters:** None

**Returns:** `ControlResponse`.

**Example:**
```json
{ "tool": "pause_recording", "arguments": {} }
```"#,
            "stop_recording" => r#"## stop_recording
Stop input recording and return recorded data.

**Parameters:** None

**Returns:** `RecordingResult` with events array.

**Example:**
```json
{ "tool": "stop_recording", "arguments": {} }
```"#,
            "get_recording_status" => r#"## get_recording_status
Get current recording status.

**Parameters:** None

**Returns:** `RecordingStatus` with recording state.

**Example:**
```json
{ "tool": "get_recording_status", "arguments": {} }
```"#,
            // Drive
            "swap_disk" => r#"## swap_disk
Swap floppy/CD image in a drive.

**Parameters:**
- `drive` (string): Drive letter (e.g., "A", "C", "D")
- `image` (string): Path to disk image

**Returns:** `DriveSwapResponse`.

**Example:**
```json
{ "tool": "swap_disk", "arguments": { "drive": "D", "image": "/path/to/game.iso" } }
```"#,
            "lock_mounts" => r#"## lock_mounts
Lock or unlock drive mounts.

**Parameters:**
- `lock` (boolean): Lock state

**Returns:** `MountLockResponse`.

**Example:**
```json
{ "tool": "lock_mounts", "arguments": { "lock": true } }
```"#,
            "get_mount_lock" => r#"## get_mount_lock
Get current mount lock state.

**Parameters:** None

**Returns:** `MountLockStatus`.

**Example:**
```json
{ "tool": "get_mount_lock", "arguments": {} }
```"#,
            // Script
            "load_script" => r#"## load_script
Load a Lua script.

**Parameters:**
- `script` (string): Lua script source code
- `name` (string, optional): Optional name for the script

**Returns:** `ScriptLoadResponse`.

**Example:**
```json
{ "tool": "load_script", "arguments": { "script": "print('hello')", "name": "test" } }
```"#,
            "start_script" => r#"## start_script
Start executing loaded Lua script.

**Parameters:** None

**Returns:** `ControlResponse`.

**Example:**
```json
{ "tool": "start_script", "arguments": {} }
```"#,
            "stop_script" => r#"## stop_script
Stop running Lua script.

**Parameters:** None

**Returns:** `ControlResponse`.

**Example:**
```json
{ "tool": "stop_script", "arguments": {} }
```"#,
            "get_script_status" => r#"## get_script_status
Get Lua script execution status.

**Parameters:** None

**Returns:** `ScriptStatus`.

**Example:**
```json
{ "tool": "get_script_status", "arguments": {} }
```"#,
            // Capture
            "start_video_capture" => r#"## start_video_capture
Start video capture (ZMBV).

**Parameters:**
- `mode` (string, default="raw"): "raw" or "rendered"

**Returns:** `CaptureStartResponse`.

**Example:**
```json
{ "tool": "start_video_capture", "arguments": { "mode": "rendered" } }
```"#,
            "stop_video_capture" => r#"## stop_video_capture
Stop video capture.

**Parameters:** None

**Returns:** `ControlResponse`.

**Example:**
```json
{ "tool": "stop_video_capture", "arguments": {} }
```"#,
            "get_capture_status" => r#"## get_capture_status
Get video capture status.

**Parameters:** None

**Returns:** `CaptureStatus`.

**Example:**
```json
{ "tool": "get_capture_status", "arguments": {} }
```"#,
            "get_compression" => r#"## get_compression
Get video capture compression settings.

**Parameters:** None

**Returns:** `CaptureCompression` with raw/rendered levels.

**Example:**
```json
{ "tool": "get_compression", "arguments": {} }
```"#,
            "set_compression" => r#"## set_compression
Set video capture compression settings.

**Parameters:**
- `raw` (u8, optional): Compression level for raw mode (0-9)
- `rendered` (u8, optional): Compression level for rendered mode (0-9)

**Returns:** `CaptureCompression`.

**Example:**
```json
{ "tool": "set_compression", "arguments": { "raw": 5, "rendered": 7 } }
```"#,
            // I/O
            "read_port" => r#"## read_port
Read I/O port (byte or word).

**Parameters:**
- `port` (u16): Port number (0-65535)
- `width` (u8, default=1): 1 for byte, 2 for word

**Returns:** `PortReadResponse`.

**Example:**
```json
{ "tool": "read_port", "arguments": { "port": 0x3F8, "width": 1 } }
```"#,
            "write_port" => r#"## write_port
Write I/O port (byte or word).

**Parameters:**
- `port` (u16): Port number (0-65535)
- `value` (u32): Value to write
- `width` (u8, default=1): 1 for byte, 2 for word

**Returns:** `PortWriteResponse`.

**Example:**
```json
{ "tool": "write_port", "arguments": { "port": 0x3F8, "value": 0x80, "width": 1 } }
```"#,
            // DOS
            "get_dos_internals" => r#"## get_dos_internals
Get DOS internals (MCB map, etc.).

**Parameters:** None

**Returns:** `DosInternals` with MCB chain, DOS structures.

**Example:**
```json
{ "tool": "get_dos_internals", "arguments": {} }
```"#,
            _ => "No documentation available for this tool",
        }.to_string()
    }

    // ---- Prompt Implementations ----
    fn prompt_install_game(&self, args: &serde_json::Map<String, serde_json::Value>) -> GetPromptResult {
        let drive = args.get("drive").and_then(|v| v.as_str()).unwrap_or("D");
        let image_path = args.get("image_path").and_then(|v| v.as_str()).unwrap_or("/path/to/game.iso");
        let install_cmd = args.get("install_cmd").and_then(|v| v.as_str()).unwrap_or("d:\\\\install.exe");

        let messages = vec![
            PromptMessage::new_text(PromptMessageRole::User, format!(
                "Install a DOS game from CD/ISO using the dosbox-mcp tools.\n\n\
                **Steps:**\n\
                1. Mount the disk image to drive {}\n\
                2. Run the installer\n\
                3. Navigate installer menus via text input\n\
                4. Launch the game\n\n\
                **Tools to use:**\n\
                - `swap_disk` — mount the ISO\n\
                - `type_text` — send keystrokes\n\
                - `read_screen_text` — read installer prompts\n\
                - `capture_frame` — verify visual state\n\n\
                **Example sequence:**",
                drive
            )),
            PromptMessage::new_text(PromptMessageRole::Assistant, format!(
                "{{\"tool\": \"swap_disk\", \"arguments\": {{\"drive\": \"{}\", \"image\": \"{}\" }}}}",
                drive, image_path
            )),
            PromptMessage::new_text(PromptMessageRole::Assistant, format!(
                "{{\"tool\": \"type_text\", \"arguments\": {{\"text\": \"{}\\r\", \"cps\": 20 }}}}",
                install_cmd
            )),
            PromptMessage::new_text(PromptMessageRole::Assistant, "{\"tool\": \"read_screen_text\", \"arguments\": {}}"),
            PromptMessage::new_text(PromptMessageRole::User, "Continue sending keystrokes based on installer prompts. Use `read_screen_text` after each step to see the current screen."),
        ];
        GetPromptResult {
            description: Some("Install a DOS game from CD/ISO".into()),
            messages,
        }
    }

    fn prompt_memory_trainer(&self, args: &serde_json::Map<String, serde_json::Value>) -> GetPromptResult {
        let value = args.get("value").and_then(|v| v.as_u64()).unwrap_or(3);
        let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(1) as u8;
        let freeze_value = args.get("freeze_value").and_then(|v| v.as_u64()).unwrap_or(99);

        let messages = vec![
            PromptMessage::new_text(PromptMessageRole::User, format!(
                "Set up a memory trainer (freeze a value like lives, ammo, score).\n\n\
                **Steps:**\n\
                1. Search memory for the current value ({})\n\
                2. Identify the correct address (may need multiple searches)\n\
                3. Freeze that address to the desired value ({})\n\
                4. Verify it works\n\
                5. Clear freezes when done\n\n\
                **Tools:** `search_memory`, `freeze_memory`, `list_freezes`, `clear_freezes`",
                value, freeze_value
            )),
            PromptMessage::new_text(PromptMessageRole::Assistant, format!(
                "{{\"tool\": \"search_memory\", \"arguments\": {{\"value\": {}, \"width\": {}}}}}",
                value, width
            )),
            PromptMessage::new_text(PromptMessageRole::User, format!(
                "Review the returned addresses. If multiple matches, change the value in-game (e.g., lose a life) and search again with the new value to narrow down. Once you have the correct address, freeze it:"
            )),
            PromptMessage::new_text(PromptMessageRole::Assistant, format!(
                "{{\"tool\": \"freeze_memory\", \"arguments\": {{\"address\": <ADDRESS_FROM_SEARCH>, \"value\": {}, \"width\": {}}}}}",
                freeze_value, width
            )),
            PromptMessage::new_text(PromptMessageRole::User, "Verify the freeze works in-game. When finished, clear all freezes:"),
            PromptMessage::new_text(PromptMessageRole::Assistant, "{\"tool\": \"clear_freezes\", \"arguments\": {}}"),
        ];
        GetPromptResult {
            description: Some("Memory trainer: find and freeze a value".into()),
            messages,
        }
    }

    fn prompt_automated_test(&self, args: &serde_json::Map<String, serde_json::Value>) -> GetPromptResult {
        let test_name = args.get("test_name").and_then(|v| v.as_str()).unwrap_or("test");
        let _duration = args.get("record_duration_secs").and_then(|v| v.as_u64()).unwrap_or(30);

        let messages = vec![
            PromptMessage::new_text(PromptMessageRole::User, format!(
                "Create an automated test: record input, replay, verify.\n\n\
                **Test:** {}\n\n\
                **Workflow:**\n\
                1. Start recording\n\
                2. Perform actions in DOSBox\n\
                3. Stop recording (save the events)\n\
                4. Reset game state\n\
                5. Replay with `send_input_sequence`\n\
                6. Capture frame and compare\n\n\
                **Tools:** `start_recording`, `stop_recording`, `send_input_sequence`, `capture_frame`",
                test_name
            )),
            PromptMessage::new_text(PromptMessageRole::Assistant, "{\"tool\": \"start_recording\", \"arguments\": {}}"),
            PromptMessage::new_text(PromptMessageRole::User, "Perform your test actions in DOSBox now..."),
            PromptMessage::new_text(PromptMessageRole::Assistant, "{\"tool\": \"stop_recording\", \"arguments\": {}}"),
            PromptMessage::new_text(PromptMessageRole::User, "Save the returned `events` array. Then reset the game state and replay:"),
            PromptMessage::new_text(PromptMessageRole::Assistant, "{\"tool\": \"send_input_sequence\", \"arguments\": {\"events\": \"<SAVED_EVENTS_ARRAY>\"}}}"),
            PromptMessage::new_text(PromptMessageRole::User, "Verify the result:"),
            PromptMessage::new_text(PromptMessageRole::Assistant, "{\"tool\": \"capture_frame\", \"arguments\": {\"mode\": \"raw\", \"format\": \"png\"}}}"),
        ];
        GetPromptResult {
            description: Some("Automated test: record/replay/verify".into()),
            messages,
        }
    }

    fn prompt_lua_script(&self, args: &serde_json::Map<String, serde_json::Value>) -> GetPromptResult {
        let script = args.get("script").and_then(|v| v.as_str()).unwrap_or("print('Hello from Lua!')");
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("script");

        let messages = vec![
            PromptMessage::new_text(PromptMessageRole::User, format!(
                "Load and execute a Lua automation script.\n\n\
                **Script name:** {}\n\n\
                **Script:**\n```lua\n{}\n```\n\n\
                **Tools:** `load_script`, `start_script`, `get_script_status`, `stop_script`",
                name, script
            )),
            PromptMessage::new_text(PromptMessageRole::Assistant, format!(
                "{{\"tool\": \"load_script\", \"arguments\": {{\"script\": \"{}\", \"name\": \"{}\" }}}}",
                script.replace('\n', "\\n").replace('"', "\\\""),
                name
            )),
            PromptMessage::new_text(PromptMessageRole::Assistant, "{\"tool\": \"start_script\", \"arguments\": {}}"),
            PromptMessage::new_text(PromptMessageRole::User, "Monitor execution:"),
            PromptMessage::new_text(PromptMessageRole::Assistant, "{\"tool\": \"get_script_status\", \"arguments\": {}}"),
        ];
        GetPromptResult {
            description: Some("Load and run a Lua script".into()),
            messages,
        }
    }
}
#[cfg(test)]
mod mount_tests {
    use crate::server::mount_path_rejection;
    use crate::server::swap_image_rejection;
    use crate::server::typed_mount_violation;
    use std::path::PathBuf;

    // Uses real paths (/usr, /usr/share, /etc): all exist on any system
    // this crate is checked out and tested on.
    #[test]
    fn mount_exact_root_allowed() {
        let r = mount_path_rejection("/usr", PathBuf::from("/usr"));
        assert!(r == None);
    }

    #[test]
    fn mount_inside_root_allowed() {
        let r = mount_path_rejection("/usr/share", PathBuf::from("/usr"));
        assert!(r == None);
    }

    #[test]
    fn mount_dotdot_escape_back_in_allowed() {
        // resolves to /usr/share, inside the root: accepted
        let r = mount_path_rejection("/usr/share/../share", PathBuf::from("/usr"));
        assert!(r == None);
    }

    #[test]
    fn mount_dotdot_out_of_root_rejected() {
        // resolves to /etc, outside the root: rejected
        let r = mount_path_rejection("/usr/../etc", PathBuf::from("/usr"));
        assert!(r != None);
    }

    #[test]
    fn mount_other_tree_rejected() {
        let r = mount_path_rejection("/etc", PathBuf::from("/usr"));
        assert!(r != None);
    }

    #[test]
    fn mount_relative_path_rejected() {
        let r = mount_path_rejection("games/doom", PathBuf::from("/usr"));
        assert!(r != None);
    }

    #[test]
    fn mount_control_chars_rejected() {
        // the guard runs before the exists() check, so the reason is the
        // control-character one even though the dir does not exist
        let r = mount_path_rejection("/usr/do\nom", PathBuf::from("/usr"));
        assert!(r != None);
    }

    #[test]
    fn mount_missing_directory_rejected() {
        let r = mount_path_rejection("/usr/definitely-not-here-9f3a1c", PathBuf::from("/usr"));
        assert!(r != None);
    }

    #[test]
    fn mount_unrestricted_root_allows_any_absolute() {
        let r = mount_path_rejection("/etc", PathBuf::from("/"));
        assert!(r == None);
    }

    // ---- swap_disk image confinement ----
    #[test]
    fn swap_inside_root_allowed() {
        let r = swap_image_rejection("/usr/share", PathBuf::from("/usr"));
        assert!(r == None);
    }

    #[test]
    fn swap_outside_root_rejected() {
        let r = swap_image_rejection("/etc", PathBuf::from("/usr"));
        assert!(r != None);
    }

    #[test]
    fn swap_relative_rejected() {
        let r = swap_image_rejection("games/doom.img", PathBuf::from("/usr"));
        assert!(r != None);
    }

    #[test]
    fn swap_missing_image_rejected() {
        let r = swap_image_rejection("/usr/no-such-image-9f3a1c", PathBuf::from("/usr"));
        assert!(r != None);
    }

    #[test]
    fn swap_unrestricted_root_allows_absolute_existing() {
        let r = swap_image_rejection("/etc", PathBuf::from("/"));
        assert!(r == None);
    }

    // ---- typed MOUNT lines through type_text ----
    #[test]
    fn typed_mount_outside_root_rejected() {
        let r = typed_mount_violation("MOUNT C /etc\nDIR", PathBuf::from("/usr"));
        assert!(r != None);
    }

    #[test]
    fn typed_mount_case_insensitive_detected() {
        let r = typed_mount_violation("mount c /etc", PathBuf::from("/usr"));
        assert!(r != None);
    }

    #[test]
    fn typed_mount_second_line_detected() {
        let r = typed_mount_violation("DIR\nMOUNT D /etc", PathBuf::from("/usr"));
        assert!(r != None);
    }

    #[test]
    fn typed_mount_inside_root_allowed() {
        let r = typed_mount_violation("MOUNT C /usr/share\nDIR", PathBuf::from("/usr"));
        assert!(r == None);
    }

    #[test]
    fn typed_mount_quoted_inside_root_allowed() {
        let r = typed_mount_violation("MOUNT C \"/usr/share\"", PathBuf::from("/usr"));
        assert!(r == None);
    }

    #[test]
    fn typed_mount_relative_path_skipped() {
        // relative goes to the emulator's own cwd: the guard does not see it
        let r = typed_mount_violation("mount c games", PathBuf::from("/usr"));
        assert!(r == None);
    }

    #[test]
    fn typed_mount_unmount_form_allowed() {
        let r = typed_mount_violation("MOUNT -U C", PathBuf::from("/usr"));
        assert!(r == None);
    }

    #[test]
    fn typed_word_mount_not_a_command_ignored() {
        // first word is ECHO: the MOUNT text is not a shell command
        let r = typed_mount_violation("ECHO mount C /etc", PathBuf::from("/usr"));
        assert!(r == None);
    }

    #[test]
    fn typed_drive_shaped_path_untouched_on_unix() {
        // on Unix hosts "C:\GAMES" is a DOS drive mapping, not a host path
        let r = typed_mount_violation("MOUNT C C:\\GAMES", PathBuf::from("/usr"));
        assert!(r == None);
    }

    #[test]
    fn typed_mount_dotdot_out_of_root_rejected() {
        let r = typed_mount_violation("MOUNT C /usr/../etc", PathBuf::from("/usr"));
        assert!(r != None);
    }
}
