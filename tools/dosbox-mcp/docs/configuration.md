<!--
This file is part of the dosbox-automation-is-not-your-nanny fork
License: GPL-2.0-or-later. This file is original work of the fork. Contact: EternalNooblet <67196335+EternalNooblet@users.noreply.github.com>.
-->

# dosbox-mcp Configuration Guide

Complete configuration reference for the dosbox-mcp server.

## Configuration Sources (Priority Order)

1. **Config file** — `~/.config/dosbox-mcp/config.toml` (lowest priority)
2. **Environment variables** — `DOSBOX_MCP_*` prefix
3. **CLI arguments** — Highest priority

---

## Config File (`~/.config/dosbox-mcp/config.toml`)

```toml
[default]
base_url = "http://localhost:8386"
timeout_secs = 30
verify_tls = true

# Multi-instance support
[[instances]]
name = "main"
base_url = "http://localhost:8386"

[[instances]]
name = "ci-runner"
base_url = "http://192.168.1.50:8386"
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `base_url` | string | required | dosbox-automation REST API URL |
| `timeout_secs` | integer | 30 | Request timeout in seconds |
| `verify_tls` | boolean | true | Verify TLS certificates |
| `cpu_cycles` | string | `30000` | CPU cycles, pinned for both real and protected mode |
| `secure_mode` | boolean | false | Enable secure mode (blocks MOUNT) |
| `mount_root` | path | (plugin data dir) | Root `mount_drive` host paths must resolve under — see [Mount root](#mount-root-mount_drive) |
| `display` | string | (unset) | X11 display (e.g., ":99" for Xvfb). If unset, uses offscreen SDL driver. |
| `instances` | array | [] | Named instances for multi-server setups |

---

### CLI Options

```bash
dosbox-mcp [OPTIONS] [-- <DOSBOX_ARGS>...]

Options:
  --base-url <URL>           dosbox-automation API URL [env: DOSBOX_API_URL]
  --timeout-secs <N>         Request timeout in seconds (default: 30)
  --no-verify-tls            Disable TLS verification
  --config <PATH>            Config file path
  --log-level <LEVEL>        trace/debug/info/warn/error (default: info)

  --dosbox-bin <PATH>        dosbox-automation binary path [env: DOSBOX_AUTOMATION_BIN]
  --auto-start               Auto-start dosbox-automation (default: true)
  --no-auto-start            Don't auto-start, connect to existing
  --working-dir <DIR>        Working directory for dosbox-automation
  --cpu-cycles <CYCLES>      CPU cycles pinned for both modes (default 30000) [env: DOSBOX_CPU_CYCLES]
  --secure-mode              Enable secure mode (blocks MOUNT, default: false)
  --display <DISPLAY>        X11 display (e.g., ":99" for Xvfb). If unset, uses offscreen SDL driver [env: DOSBOX_DISPLAY]
  --mount-root <ROOT>        Root mount_drive host paths must resolve under (default: plugin data dir) [env: DOSBOX_MCP_MOUNT_ROOT]

  <DOSBOX_ARGS>...           Extra args passed to dosbox-automation
```

### Examples

```bash
# The default — run with zero arguments. Auto-start finds the binary,
# sets libiir LD_LIBRARY_PATH for the child, and serves on localhost:8386.
dosbox-mcp

# Debug logging (only when diagnosing startup)
dosbox-mcp --log-level debug

# Pass extra args to dosbox-automation
dosbox-mcp -- -conf /custom/dosbox.conf
```

Prefer zero arguments. The options below exist for special setups only.

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `DOSBOX_API_URL` | Base URL for dosbox-automation API |
| `DOSBOX_AUTOMATION_BIN` | Path to dosbox-automation binary |
| `DOSBOX_CPU_CYCLES` | CPU cycles pinned for both modes (default 30000) |
| `DOSBOX_DISPLAY` | X11 display (e.g., ":99" for Xvfb) |
| `DOSBOX_MCP_BASE_URL` | Config override for base_url |
| `DOSBOX_MCP_TIMEOUT_SECS` | Config override for timeout |
| `DOSBOX_MCP_VERIFY_TLS` | Config override for TLS verification |
| `DOSBOX_MCP_CPU_CYCLES` | Config override for cpu_cycles |
| `DOSBOX_MCP_SECURE_MODE` | Config override for secure_mode |
| `DOSBOX_MCP_DISPLAY` | Config override for display |
| `DOSBOX_MCP_LOG_LEVEL` | Config override for log level |

### Example

```bash
export DOSBOX_API_URL=http://localhost:8386
export DOSBOX_MCP_TIMEOUT_SECS=60
export DOSBOX_MCP_CPU_CYCLES=30000
export DOSBOX_MCP_DISPLAY=:99
dosbox-mcp
```

### Graphics Mode with Xvfb

For graphical DOS applications (not text-mode), you need a real X11 display. Use Xvfb:

```bash
# Start Xvfb on display :99 (run once as persistent infrastructure)
Xvfb :99 -screen 0 1024x768x24 &

# Run dosbox-mcp with Xvfb display
dosbox-mcp --display :99 --cpu-cycles 30000
```

The `--display :99` (or `DOSBOX_DISPLAY=:99` / `DOSBOX_MCP_DISPLAY=:99`) tells dosbox-automation to use Xvfb instead of the offscreen SDL driver, enabling frame capture for graphics-mode applications.

---

## Auto-Start Behavior

When `--auto-start` (default), the server:

1. **Detects binary** — Searches `./build/release-linux/dosbox-automation`, `/usr/local/bin/`, `/usr/bin/`, `~/.local/bin/`, `PATH`
2. **Finds libraries** — Auto-locates `libiir.so`, sets `LD_LIBRARY_PATH`
3. **Configures webserver** — Applies defaults:
   ```ini
   webserver_enabled=on
   webserver_bind_address=127.0.0.1
   webserver_port=8386
   webserver_token_file=off
   ```
4. **Health checks** — Waits up to 30s for API readiness
5. **Cleanup** — Graceful shutdown on MCP server exit

### Customizing Auto-Start

Auto-start needs no configuration: binary detection, `LD_LIBRARY_PATH` for `libiir.so`, webserver settings, and CPU cycles (30000, pinned for both modes) are all handled automatically. Special cases only:

```bash
# Graphics mode with Xvfb (for graphical apps, not text-mode):
# prefer display = ":99" in ~/.config/dosbox-mcp/config.toml over CLI flags
Xvfb :99 -screen 0 1024x768x24 &

# Enable secure mode (blocks MOUNT from guest/API)
dosbox-mcp --secure-mode

# Connect to an already-running remote instance (multi-instance setups)
dosbox-mcp --no-auto-start --base-url http://remote-host:8386
```

**Note:** By default (`--secure-mode` off), MOUNT is allowed from both guest commands and API. There is no mount whitelist: any existing directory may be mounted (system paths, symlinks to system paths, and non-directories are still refused). The old `mount_allowed_bases` / `mount_allowed_image_roots` settings and their MCP tools were removed.

### Mount root (`mount_drive`)

`mount_drive` types `MOUNT <drive> <host_path>` at the DOS shell, and the path
it gets is the model's — unapproved. Left unrestricted, one call can reach the
whole host: your Hermes `.env`, work folders, anything. So `host_path` is
checked **before anything is typed**:

- absolute and existing (else a specific rejection, not shell noise);
- no control characters — a newline in the path would run the rest of it as
  extra DOS commands;
- inside the configured root **after resolving `..` and symlinks**, so neither
  a `../` chain nor a symlinked directory escapes.

```toml
# ~/.config/dosbox-mcp/config.toml — pick a root the owner trusts
mount_root = "/home/papa/games"
```

Same via CLI (`--mount-root /home/papa/games`) or env
(`DOSBOX_MCP_MOUNT_ROOT=/home/papa/games`).

Unset default: the plugin data directory
`<HERMES_HOME, or HOME/.hermes>/plugin-data/dosbox-full-auto` — the plugin
itself passes this explicitly when it registers the server. Set `mount_root =
"/"` to disable the confinement.

The same root confines `swap_disk` image paths and MOUNT lines typed through
`type_text` (that last one is a text-scanning guard: lines whose first word
is `mount`, absolute host-path tokens only — relative paths and in-game text
untouched). MOUNTs performed inside the guest (batch files, self-booting
games) only face the emulator's own rules — no MCP-side check can see those.
And `MOUNT` typed with a DOS drive shape (`C:\GAME`) is a drive mapping on
Unix hosts, deliberately left alone.

---

## Multi-Instance Setup

Run multiple DOSBox instances by defining named instances in config:

```toml
[default]
base_url = "http://localhost:8386"
timeout_secs = 30

[[instances]]
name = "game-server"
base_url = "http://localhost:8386"

[[instances]]
name = "test-runner-1"
base_url = "http://localhost:8387"

[[instances]]
name = "test-runner-2"
base_url = "http://192.168.1.100:8386"
```

**Note:** Each instance needs its own dosbox-automation process on a different port.

---

## TLS Configuration

For production/remote setups with TLS:

```toml
[default]
base_url = "https://dosbox.example.com:8386"
timeout_secs = 30
verify_tls = true  # Set false for self-signed certs (--no-verify-tls)
```

---

## Troubleshooting Configuration

### "dosbox-automation binary not found"
No binary exists in any searched location (`./build/release-linux/`, `/usr/local/bin/`, `/usr/bin/`, `~/.local/bin/`, `PATH`). Build dosbox-automation from source or install the binary into one of those locations. Do not work around it with flags or env vars — once the binary is in a searched location, zero-argument startup works.

### "Connection refused" / "Timeout waiting for webserver"
Another process owns port 8386, or the emulator is still booting. Check what holds the port (`lsof -i :8386`). If an instance is already running, the server attaches to it.

### libiir.so
The launcher auto-detects `libiir.so` relative to the binary's build directory (following symlinks) and sets `LD_LIBRARY_PATH` for the child process. No manual export is needed for build-dir or symlinked installs.

### Permission denied on disk swap
Mounts must be existing directories (system paths and symlinks to them are refused). There is no whitelist config for the emulator itself: `mount_drive`, `swap_disk` images and MOUNT lines typed through `type_text` all confine to the configured mount root (default: the plugin data dir — see the Mount root section) and a rejection names the allowed root; only MOUNTs issued from inside the guest (batch files, game code) face the emulator's rules alone.

---

## Resource Access

Configuration docs available as MCP Resource:
```json
{ "method": "resources/read", "params": { "uri": "docs://dosbox-mcp/configuration" } }
```