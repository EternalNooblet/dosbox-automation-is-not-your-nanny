<!--
This file is part of the dosbox-automation-is-not-your-nanny fork
License: GPL-2.0-or-later. This file is original work of the fork. Contact: EternalNooblet <67196335+EternalNooblet@users.noreply.github.com>.
-->

# dosbox-mcp AI Setup Document

This document enables an AI agent to install and configure `dosbox-mcp` (MCP server for dosbox-automation) from scratch on a fresh Linux machine.

---

## 1. Prerequisites

### System Dependencies (CachyOS / Arch)

Check what's already installed before installing — this prints only the missing packages:
```bash
for p in base-devel cmake git pkgconf sdl3 sdl3_image freetype2 libpng mesa \
  alsa-lib libarchive bzip2 zlib speexdsp gtest python xorg-server-xvfb \
  xorg-xdpyinfo curl; do
  pacman -Qi "$p" >/dev/null 2>&1 || echo "MISSING: $p"
done
```

Install (idempotent — `--needed` skips packages already present):
```bash
sudo pacman -S --needed \
  base-devel cmake git pkgconf \
  sdl3 sdl3_image \
  freetype2 libpng mesa \
  alsa-lib libarchive bzip2 zlib \
  speexdsp gtest \
  python \
  xorg-server-xvfb xorg-xdpyinfo \
  curl
```

Notes:
- The build targets **SDL3** (>= 3.4.0), not SDL2 — `sdl3` and `sdl3_image` are the correct Arch packages.
- `gtest` ships both gtest and gmock; tests require the system GoogleTest.
- Optional: `fluidsynth` (system FluidSynth instead of fetching it; `-DOPT_FETCH_DEPS=ON` builds it from source if missing).
- SDL3, SDL3_image, MT32Emu, iir1, zlib-ng and FluidSynth can all be resolved by `-DOPT_FETCH_DEPS=ON`; libpng, OpenGL (mesa), ALSA, SpeexDSP, GTest, LibArchive, BZip2, zlib must be system packages.

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
cmake --preset release-linux -DOPT_FETCH_DEPS=ON
cmake --build build/release-linux -- -j$(nproc)
```

**Binary location**: `build/release-linux/dosbox-automation`

---

## 4. Build dosbox-mcp (Rust)

```bash
cd tools/dosbox-mcp
cargo build --release
```

**Binary location**: `tools/dosbox-mcp/target/release/dosbox-mcp`

---

## 5. Configure Primary Config (Mount Policy)

There is no mount whitelist. The mount policy refuses system paths and symlinks to them, and non-directories; any other existing directory may be mounted. The primary config file at:
```
~/.config/dosbox-automation/dosbox-automation.conf
```

Create it:
```bash
mkdir -p ~/.config/dosbox-automation
cat > ~/.config/dosbox-automation/dosbox-automation.conf << 'EOF'
[webserver]
[cpu]
cpu_cycles = 30000
cpu_cycles_protected = 30000
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
cpu_cycles = "30000"

# For graphical apps (not text-mode):
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

Run it with zero arguments — auto-start handles everything:

```bash
# From the repo root (text mode; binary found in ./build/release-linux/,
# libiir LD_LIBRARY_PATH set for the child automatically):
/home/papa/dosbox-automation-is-not-your-nanny/tools/dosbox-mcp/target/release/dosbox-mcp
```

For graphical DOS apps, start Xvfb and set `display = ":99"` in `~/.config/dosbox-mcp/config.toml` (section 6) — no CLI flags needed.

The server also attaches to an already-running instance at its `base_url`, so launching a second server is never required for a second agent.

---

## 9. Register with Hermes

Add to Hermes MCP config (e.g., `~/.hermes/mcp_servers.json`) with no arguments:
```json
{
  "mcpServers": {
    "dosbox-mcp": {
      "command": "/home/papa/dosbox-automation-is-not-your-nanny/tools/dosbox-mcp/target/release/dosbox-mcp",
      "args": []
    }
  }
}
```

If Hermes spawns the server outside the repo root, install the binary where auto-start can find it from any working directory:

```bash
ln -s /home/papa/dosbox-automation-is-not-your-nanny/build/release-linux/dosbox-automation ~/.local/bin/dosbox-automation
```

---

## 10. Verify Installation

### MCP Tools Available (55 total)
```bash
# List tools via MCP
# Resources: docs://dosbox-mcp/api-reference
# Prompts: install-game, memory-trainer, automated-test, lua-script
```

### Key Drive Tools
- `swap_disk` — Mount floppy/CD images
- `lock_mounts` / `get_mount_lock` — Mount lock latch
- `mount_drive` — Mount a host directory (types MOUNT, verifies via screen text)
- `get_mixer_status` / `set_mixer_volume` / `mute_mixer` / `unmute_mixer` / `set_mixer_channel_volume` — Mixer control
- `start_audio_capture` / `stop_audio_capture` / `get_audio_capture_status` — Audio capture
- `get_mouse_position` — Last injected cursor position
- `analyze_frame` — Frame black ratio / luma / hash (black-watch loop in one call)

### Quick Test
```json
{
  "tool": "mount_drive",
  "arguments": { "drive": "D", "host_path": "/tmp" }
}
```
Should return the MOUNT result line in `screen_text`.

---

## 11. Common Issues

| Issue | Fix |
|-------|-----|
| `dosbox-automation binary not found` | No binary in the searched locations (`./build/release-linux/`, `/usr/local/bin/`, `/usr/bin/`, `~/.local/bin/`, `PATH`) — build it or install/symlink it there. Flags and env vars are not the fix. |
| `Connection refused` | Port 8386 is owned by another process, or boot is still in progress (`lsof -i :8386`) |
| Black frames in graphics apps | Start Xvfb `:99`, set `display = ":99"` in `~/.config/dosbox-mcp/config.toml` |
| 3DS slow start or black screen | Pin both rates: launcher sets `cpu_cycles_protected` alongside `cpu_cycles` (30000); unprotected pmode defaults to 60000 and slows DOS-extender apps |
| MOUNT blocked | Path is a system path / symlink to one, or does not exist; ensure `--secure-mode` is off |
| `libiir.so not found` | Launcher auto-detects libiir relative to the binary (following symlinks) and sets `LD_LIBRARY_PATH` for the child; only a system install with libs elsewhere needs manual handling |

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

| Component | Path (relative to repo root) |
|-----------|------|
| dosbox-automation binary | `build/release-linux/dosbox-automation` |
| dosbox-mcp binary | `tools/dosbox-mcp/target/release/dosbox-mcp` |
| Primary mount config | `~/.config/dosbox-automation/dosbox-automation.conf` |
| dosbox-mcp config | `~/.config/dosbox-mcp/config.toml` |
| Xvfb display | `:99` (if used) |
| MCP server URL | `http://localhost:8386` |

---

## 14. One-Command Setup (After Prerequisites)

```bash
cd dosbox-automation-is-not-your-nanny && \
cmake --preset release-linux -DOPT_FETCH_DEPS=ON && \
cmake --build build/release-linux -- -j$(nproc) && \
cd tools/dosbox-mcp && cargo build --release && \
mkdir -p ~/.config/dosbox-automation && \
echo -e "[cpu]\ncpu_cycles = 30000\ncpu_cycles_protected = 30000" > ~/.config/dosbox-automation/dosbox-automation.conf && \
mkdir -p ~/.config/dosbox-mcp && \
echo -e '[default]\nbase_url = "http://localhost:8386"\ndisplay = ":99"\ncpu_cycles = "30000"' > ~/.config/dosbox-mcp/config.toml
```

Then start Xvfb and run (from the repo root):
```bash
cd ../.. && Xvfb :99 -screen 0 1024x768x24 &
"$PWD/tools/dosbox-mcp/target/release/dosbox-mcp"
```