# dosbox-mcp AI Setup Document

This document enables an AI agent to install and configure `dosbox-mcp` (MCP server for dosbox-automation) from scratch on a fresh Linux machine.

---

## 1. Prerequisites

### System Dependencies (Ubuntu/Debian)
```bash
apt-get update && apt-get install -y \
  build-essential cmake git pkg-config \
  libsdl2-dev libsdl2-net-dev libsdl2-image-dev libsdl2-mixer-dev \
  libfreetype6-dev libpng-dev libjpeg-dev libasound2-dev \
  libwayland-dev libxkbcommon-dev libvulkan-dev \
  libarchive-dev libbz2-dev zlib1g-dev \
  libspeexdsp-dev libopusfile-dev \
  python3 python3-pip \
  xvfb \
  curl
```

### Rust Toolchain
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup component add rustfmt clippy
```

### Verify
```bash
cmake --version    # >= 3.20
rustc --version    # >= 1.75
cargo --version
```

---

## 2. Clone Repository

```bash
git clone https://github.com/EternalNooblet/dosbox-automation-is-not-your-nanny.git
cd dosbox-automation-is-not-your-nanny
```

---

## 3. Build dosbox-automation (C++)

```bash
cmake --preset debug-linux -DOPT_FETCH_DEPS=ON
cmake --build build/debug-linux -- -j$(nproc)
```

**Binary location**: `build/debug-linux/dosbox-automation`

---

## 4. Build dosbox-mcp (Rust)

```bash
cd tools/dosbox-mcp
cargo build --release
```

**Binary location**: `tools/dosbox-mcp/target/release/dosbox-mcp`

---

## 5. Configure Primary Config (Mount Policy)

The mount policy **only reads** `mount_allowed_bases` and `mount_allowed_image_roots` from the primary config file at:
```
~/.config/dosbox-automation/dosbox-automation.conf
```

Create it:
```bash
mkdir -p ~/.config/dosbox-automation
cat > ~/.config/dosbox-automation/dosbox-automation.conf << 'EOF'
[webserver]
mount_allowed_bases = /path/to/your/project/root
mount_allowed_image_roots = /path/to/your/project/root;/path/to/disk/images
EOF
```

**Replace `/path/to/your/project/root`** with the actual directory containing your DOS programs/disk images. Multiple paths separated by `;`.

---

## 6. Configure dosbox-mcp (Optional)

Create `~/.config/dosbox-mcp/config.toml`:
```bash
mkdir -p ~/.config/dosbox-mcp
cat > ~/.config/dosbox-mcp/config.toml << 'EOF'
[default]
base_url = "http://localhost:8386"
timeout_secs = 30
verify_tls = true

# For graphical apps (not text-mode):
# cpu_cycles = "30000"
# display = ":99"

# For secure mode (blocks MOUNT from guest/API):
# secure_mode = false
EOF
```

---

## 7. Start Xvfb (For Graphics Mode)

If running graphical DOS apps (not text-mode), start Xvfb **once** as persistent infrastructure:
```bash
Xvfb :99 -screen 0 1024x768x24 &
# Verify: xdpyinfo -display :99
```

---

## 8. Run dosbox-mcp

### With Auto-Start (Recommended)
```bash
# Text-mode only (default, no Xvfb needed):
~/.cargo/bin/dosbox-mcp --dosbox-bin /full/path/to/dosbox-automation-is-not-your-nanny/build/debug-linux/dosbox-automation

# Graphics mode (requires Xvfb :99 running):
~/.cargo/bin/dosbox-mcp --dosbox-bin /full/path/to/dosbox-automation-is-not-your-nanny/build/debug-linux/dosbox-automation --display :99 --cpu-cycles 30000
```

### Without Auto-Start (Connect to Existing)
```bash
# Terminal 1: Start dosbox-automation manually
cd /full/path/to/dosbox-automation-is-not-your-nanny
./build/debug-linux/dosbox-automation --noprimaryconf --noautoexec \
  --set webserver_enabled=on \
  --set webserver_bind_address=0.0.0.0 \
  --set webserver_port=8386 \
  --set webserver_allow_remote=on \
  --set webserver_token_file=off \
  --securemode

# Terminal 2: Connect MCP server
~/.cargo/bin/dosbox-mcp --no-auto-start --base-url http://localhost:8386
```

---

## 9. Register with Hermes

Add to Hermes MCP config (e.g., `~/.hermes/mcp_servers.json`):
```json
{
  "mcpServers": {
    "dosbox-mcp": {
      "command": "/home/papa/.cargo/bin/dosbox-mcp",
      "args": [
        "--dosbox-bin",
        "/home/papa/dosbox-automation-is-not-your-nanny/build/debug-linux/dosbox-automation",
        "--display",
        ":99",
        "--cpu-cycles",
        "30000"
      ],
      "env": {
        "DOSBOX_MCP_BASE_URL": "http://localhost:8386"
      }
    }
  }
}
```

**Adjust paths** to match your actual installation locations.

---

## 10. Verify Installation

### MCP Tools Available (45 total)
```bash
# List tools via MCP
# Resources: docs://dosbox-mcp/api-reference
# Prompts: install-game, memory-trainer, automated-test, lua-script
```

### Key Drive Tools
- `swap_disk` — Mount floppy/CD images
- `lock_mounts` / `get_mount_lock` — Mount lock latch
- `get_mount_allowed_bases` — Query allowed MOUNT bases
- `get_mount_allowed_image_roots` — Query allowed image roots

### Quick Test
```json
{
  "tool": "get_mount_allowed_bases",
  "arguments": {}
}
```
Should return your configured `mount_allowed_bases` paths.

---

## 11. Common Issues

| Issue | Fix |
|-------|-----|
| `dosbox-automation binary not found` | Use `--dosbox-bin /full/path/to/binary` |
| `Connection refused` | Check port 8386 free; increase `--timeout-secs` |
| Black frames in graphics apps | Start Xvfb `:99`, use `--display :99` |
| 3DS black screen on auto cycles | Use `--cpu-cycles 30000` |
| MOUNT blocked | Check `mount_allowed_bases` in primary config; ensure `--secure-mode` is off |
| `libiir.so not found` | Build with `-DOPT_FETCH_DEPS=ON` or set `LD_LIBRARY_PATH` |

---

## 12. Environment Variable Reference

| Variable | Purpose |
|----------|---------|
| `DOSBOX_AUTOMATION_BIN` | Path to dosbox-automation binary |
| `DOSBOX_API_URL` | API base URL |
| `DOSBOX_CPU_CYCLES` | CPU cycles (e.g., "30000") |
| `DOSBOX_DISPLAY` | X11 display (e.g., ":99") |
| `DOSBOX_MCP_BASE_URL` | Config override |
| `DOSBOX_MCP_CPU_CYCLES` | Config override |
| `DOSBOX_MCP_DISPLAY` | Config override |
| `DOSBOX_MCP_SECURE_MODE` | Config override |
| `DOSBOX_MCP_TIMEOUT_SECS` | Config override |

---

## 13. File Locations Summary

| Component | Path |
|-----------|------|
| dosbox-automation binary | `~/dosbox-automation-is-not-your-nanny/build/debug-linux/dosbox-automation` |
| dosbox-mcp binary | `~/dosbox-automation-is-not-your-nanny/tools/dosbox-mcp/target/release/dosbox-mcp` |
| Primary mount config | `~/.config/dosbox-automation/dosbox-automation.conf` |
| dosbox-mcp config | `~/.config/dosbox-mcp/config.toml` |
| Xvfb display | `:99` (if used) |
| MCP server URL | `http://localhost:8386` |

---

## 14. One-Command Setup (After Prerequisites)

```bash
cd ~/dosbox-automation-is-not-your-nanny && \
cmake --preset debug-linux -DOPT_FETCH_DEPS=ON && \
cmake --build build/debug-linux -- -j$(nproc) && \
cd tools/dosbox-mcp && cargo build --release && \
mkdir -p ~/.config/dosbox-automation && \
echo -e "[webserver]\nmount_allowed_bases = $PWD/../..\nmount_allowed_image_roots = $PWD/../.." > ~/.config/dosbox-automation/dosbox-automation.conf && \
mkdir -p ~/.config/dosbox-mcp && \
echo -e '[default]\nbase_url = "http://localhost:8386"\ndisplay = ":99"\ncpu_cycles = "30000"' > ~/.config/dosbox-mcp/config.toml
```

Then start Xvfb and run:
```bash
Xvfb :99 -screen 0 1024x768x24 &
~/dosbox-automation-is-not-your-nanny/tools/dosbox-mcp/target/release/dosbox-mcp \
  --dosbox-bin ~/dosbox-automation-is-not-your-nanny/build/debug-linux/dosbox-automation
```