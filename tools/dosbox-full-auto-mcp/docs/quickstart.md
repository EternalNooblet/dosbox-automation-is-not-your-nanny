<!--
This file is part of the dosbox-full-auto fork
License: GPL-2.0-or-later. This file is original work of the fork. Contact: EternalNooblet <67196335+EternalNooblet@users.noreply.github.com>.
-->

# dosbox-full-auto-mcp Quick Start Guide

Get running in 3 steps.

---

## Prerequisites

- **Rust 1.70+** — [Install Rust](https://rustup.rs/)
- **dosbox-automation** — Built from source or installed at one of the locations the server searches automatically: `./build/release-linux/dosbox-automation` (repo build dir), `/usr/local/bin/`, `/usr/bin/`, `~/.local/bin/`, or `PATH`.

---

## Step 1: Build the Server

```bash
git clone https://github.com/EternalNooblet/dosbox-full-auto
cd dosbox-full-auto/tools/dosbox-full-auto-mcp
cargo build --release
```

Binary: `target/release/dosbox-full-auto-mcp`

---

## Step 2: Configure Your MCP Client

### Claude Desktop (`claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "dosbox": {
      "command": "/full/path/to/dosbox-full-auto-mcp",
      "args": []
    }
  }
}
```

**Config file locations:**
- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Linux: `~/.config/Claude/claude_desktop_config.json`
- Windows: `%APPDATA%\Claude\claude_desktop_config.json`

### Cursor (`.cursor/mcp.json`)

```json
{
  "mcpServers": {
    "dosbox": {
      "command": "/full/path/to/dosbox-full-auto-mcp",
      "args": []
    }
  }
}
```

### Continue.dev (`~/.continue/config.yaml`)

```yaml
mcpServers:
  dosbox:
    command: "/full/path/to/dosbox-full-auto-mcp"
    args: []
```

Pass no arguments. The server reads `~/.config/dosbox-full-auto-mcp/config.toml` on its own and everything else has sensible defaults.

---

## Step 3: Start the Server

```bash
dosbox-full-auto-mcp
```

That single command is the whole startup. With zero arguments the server:

- Auto-starts `dosbox-automation`, finding the binary at the locations listed in Prerequisites
- Auto-locates `libiir.so` next to the binary's build directory and sets `LD_LIBRARY_PATH` for the child process itself — you never need to export it
- Configures the automation webserver (`localhost:8386`) and waits for it to become ready

The server runs via stdio — your MCP client will start it automatically when needed.

---

## Verify It Works

In your AI assistant, ask:

> "What's the DOSBox version and what features are available?"

The assistant should call `get_dosbox_info` and return something like:
```
dosbox-automation 0.85.1
Features: memory, input, cpu_registers, video, recording, drives, scripting, capture, io, dos
Binary: /path/to/dosbox
```

---

## First Commands to Try

```json
# Get status
{ "tool": "get_status", "arguments": {} }

# Read text-mode screen
{ "tool": "read_screen_text", "arguments": {} }

# Type a command
{ "tool": "type_text", "arguments": { "text": "dir\r", "cps": 20 } }

# Capture a frame
{ "tool": "capture_frame", "arguments": { "mode": "rendered", "format": "png" } }
```

---

## Common Issues

### "dosbox-automation binary not found"
No binary exists in any searched location. Build dosbox-automation from source (repo root: `cmake --preset release-linux -DOPT_FETCH_DEPS=ON && cmake --build build/release-linux`) or install it into `~/.local/bin/` or `/usr/local/bin/`. No CLI flags or environment variables are needed once a binary is in a searched location.

### "Connection refused" / "Timeout"
Another process owns port 8386, or the emulator is still booting. Check what holds the port (`lsof -i :8386`); if an instance is already running, the server attaches to it instead of starting a new one.

### Black frames in graphics-mode apps
Text mode works with zero arguments. Graphical DOS apps need a real X11 display: start `Xvfb :99` and set `display = ":99"` in `~/.config/dosbox-full-auto-mcp/config.toml`.

---

## Next Steps

- **Read the full API reference**: `docs://dosbox-full-auto-mcp/api-reference`
- **See workflow examples**: `docs://dosbox-full-auto-mcp/workflows`
- **Configure multi-instance**: `docs://dosbox-full-auto-mcp/configuration`
- **Use prompt templates**: `prompts/get` with `install-game`, `memory-trainer`, etc.

---

## Resource Access (for AI Assistants)

```json
# Quickstart
{ "method": "resources/read", "params": { "uri": "docs://dosbox-full-auto-mcp/quickstart" } }

# Full API reference
{ "method": "resources/read", "params": { "uri": "docs://dosbox-full-auto-mcp/api-reference" } }

# Individual tool
{ "method": "resources/read", "params": { "uri": "docs://dosbox-full-auto-mcp/tool/type_text" } }

# Prompt for game install
{ "method": "prompts/get", "params": { "name": "install-game", "arguments": { "drive": "D", "image_path": "/path/to/game.iso" } } }
```
