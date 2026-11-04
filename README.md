<!-- This file is part of the dosbox-automation Project. -->
<!-- License: GPL-2.0-or-later. Contact: dosbox-automation-project@trinity2k.net -->

<p align="center">
  <a href="https://www.dosbox-automation.org/">
    <img src="https://www.dosbox-automation.org/imgs/logo-splash.png" alt="dosbox-automation" width="420">
  </a>
</p>

<h2 align="center">
dosbox-automation
</h2>

<h3 align="center">
  A DOS emulator you can play with <em>and</em> fully remotely control.
</h3>

<p align="center">
  A safer DOSBox staging fork with an HTTP REST API, sandbox Lua scripting, frame-accurate input recording, and games that run straight from their zip files - for gamers, game recordings, game launchers, AI agents, and CI pipelines alike.
</p>

<p align="center">
  Releases for <strong>Linux, Windows</strong>, builds from source on <strong>macOS, FreeBSD</strong>&ensp;|&ensp;
  <a href="https://github.com/dosbox-automation/dosbox-automation/releases">Downloads</a>&ensp;|&ensp;
  <a href="https://www.dosbox-automation.org/">Documentation</a>&ensp;|&ensp;
  <a href="https://www.dosbox-automation.org/0.85.1/automation/security/">Security</a>
</p>

---

## Fork deltas — how this fork differs from upstream dosbox-automation

This repository is a fork of [dosbox-automation](https://github.com/dosbox-automation/dosbox-automation)
at upstream commit `b9f24198a` (0.85.1 era, 2026-10-28). Everything below is **not** in upstream;
core emulation (CPU, DOS, video, audio, archive mounts, TrueType text, Cheat Workbench) is unchanged
from that upstream point. All deltas are in the webserver/API layer, the mount policy, mouse
injection, and the new `tools/dosbox-mcp` MCP server. Commits: `962bfcf97` … `f42c2ec43`.

### 1. `tools/dosbox-mcp` — new MCP server (entirely new, ~9,100 lines)

A Rust MCP server (stdio transport) that auto-starts dosbox-automation with the webserver on,
pins CPU cycles, and exposes **55 tools**:

- Status/control: `get_status`, `get_program_state`, `get_dosbox_info`, `shutdown`, `pause`, `resume`, `get_pause_state`
- CPU: `get_registers`, `get_cycles`, `set_cycles`, `set_register`
- Memory: `read_memory`, `write_memory`, `search_memory`, `get_page_map`, `allocate_memory`, `free_memory`, `freeze_memory`, `list_freezes`, `clear_freezes`
- Input: `type_text`, `send_input_sequence`, `get_mouse_position`
- Video: `capture_frame`, `get_frame_info`, `read_screen_text`, `analyze_frame` (black ratio / luma / hash in one call, works in graphics mode)
- Recording: `start_recording`, `pause_recording`, `stop_recording`, `get_recording_status`
- Drive: `swap_disk`, `lock_mounts`, `get_mount_lock`, `mount_drive` (types MOUNT and verifies via screen text)
- Script: `load_script`, `start_script`, `stop_script`, `get_script_status`
- Capture: `start_video_capture`, `stop_video_capture`, `get_capture_status`, `get_compression`, `set_compression`, `start_audio_capture`, `stop_audio_capture`, `get_audio_capture_status`
- Mixer: `get_mixer_status`, `set_mixer_volume`, `mute_mixer`, `unmute_mixer`, `set_mixer_channel_volume`
- I/O: `read_port`, `write_port` — DOS: `get_dos_internals`

Also: 4 embedded doc resources (`docs://dosbox-mcp/api-reference`, `configuration`, `workflows`, `quickstart`),
a `docs://dosbox-mcp/tool/{name}` resource template, 4 prompts (`install-game`, `memory-trainer`,
`automated-test`, `lua-script`), config via `~/.config/dosbox-mcp/config.toml` / env / CLI
(`cpu_cycles`, `display`, `secure_mode`), logs on stderr (stdout is the JSON-RPC transport),
and `AI_SETUP.md` for from-scratch CachyOS/Arch setup. See [tools/dosbox-mcp/README.md](tools/dosbox-mcp/README.md).

### 2. Webserver: token-free, no host validation, bound to 0.0.0.0

Upstream requires a bearer token (generated file, `Authorization: Bearer …`) and validates the
Host header. This fork removes both: the API is token-free, any Host is accepted, and the
launcher binds `0.0.0.0` with `webserver_allow_remote=on` and `webserver_token_file=off`.
The server also closes each connection after its answer (`set_keep_alive_max_count(1)`) so pooled
clients don't reuse a dead socket. The Security section below describes upstream; in this fork
the effective policy is: loopback tool pages still announce themselves, payload cap (10 MB) kept,
system-path mount policy kept (see §7).

### 3. Pause / resume API (new endpoints)

`POST /api/v1/control/pause`, `POST /api/v1/control/resume`, `GET /api/v1/control/paused`.
Pause freezes the guest at an instruction boundary (no CPU decode, no ticks) while the bridge
keeps pumping the API queue, so memory/freeze commands answer while paused.
`src/dosbox.cpp` now drains the bridge on every loop pass (not only on PIC events), and
`WaitForCompletion` timeout went 250 ms → 1000 ms.

### 4. Memory API hardening + guest-linear reads

- Page guard: `read_memory`/`write_memory`/`search_memory` refuse any range covering a page
  whose handler is not plain host RAM/ROM (`MEM_PageReadable`/`MEM_PageWritable`), returning a
  fast 400 instead of faulting into an illegal-page handler mid-scan and aborting the engine.
- Dispatch-free host copies: `MEM_BlockReadHost`, `MEM_PageReadHostPt`, `MEM_PhysicalReadHost`
  copy straight from host page backing after re-checking handler identity. API reads never
  dispatch handler logic or initialise the TLB — that used to kill the webserver connection
  on a running VCPI guest.
- Guest-linear addressing: offset bit 31 (`addr + 0x80000000`) translates through the guest's
  active page directory via raw CR3 walk (`PAGING_TranslateLinearRead` in `src/cpu/paging.cpp`);
  unmapped linear addresses fail fast with 400.
- New endpoint `GET /api/v1/memory/pagemap` (`PageMapCommand`): per-page readable/writable
  flags; the MCP server adds coalesced `readable_ranges`/`writable_ranges` (raw flags are
  `0x01`/`0x00`, not `'1'`/`0'` — `flag_ranges()` handles both).
- Bounded command bridge (`src/webserver/bridge.cpp`): a stalled emulator thread now surfaces
  `500 "Command bridge stalled"` instead of hanging the whole API.
- `free_memory`/`allocate_memory` now return JSON (`{"addr","success"}`) instead of an empty
  body (which made MCP fail with "error decoding response body").

### 5. Input: burst serialization, key aliases, mouse injection + position

- MCP serializes input bursts and retries 409 "Replay already in progress" (up to ~30 s)
  instead of failing; `is_booted` is normalized from `is_shell`.
- Key aliases in `send_input_sequence`: canonical `KBD_*` names plus friendly aliases
  (`enter`, `esc`, `space`, `left`, `a-z`, `0-9`, `f1-f12`), case-insensitive, with or without
  the `KBD_` prefix; unknown keys now error with the accepted forms listed.
- Mouse API injection (`MOUSE_InjectMoved/Warp/Button/Wheel` in `src/hardware/input/mouse.cpp`)
  bypasses capture/focus/drop guards and reaches all `IsUsingEvents()` interfaces (DOS driver,
  PS/2, serial, ManyMouse). Warp is implemented as a tracked-position delta plus `InjectWarp`,
  so PS/2/serial (whose `InjectWarp` is a no-op) still move.
- New endpoint `GET /api/v1/input/mouse`: last injected cursor position in API coordinates.
- DOS-driver relative-mode fix (`mouseif_dos_driver.cpp`): injection forces relative mode so
  repeated relative moves aren't dropped when the dummy absolute position doesn't change.

### 6. CPU registers: eip/flags writable, case-insensitive

`set_register` now accepts `eip` and `flags`/`eflags` (previously "Unknown register", despite
docs promising eip) and register names are case-insensitive (`EAX` works).
The MCP launcher pins **both** `cpu_cycles` and `cpu_cycles_protected` to 30000 (DOS extenders
otherwise silently take the hardcoded 60000 protected-mode rate and run slow), and
`set_cycles` sends the integer the API accepts.

### 7. Mount policy: whitelist removed, unlock added

Upstream enforces a `mount_allowed_bases` / `mount_allowed_image_roots` whitelist.
This fork removes the whitelist entirely: any existing directory may be mounted (system paths,
symlinks to system paths, nonexistent paths and non-directories are still refused).
`ParsePolicyConfig`/`AllowedBases`/`AllowedImageRoots` are no-ops kept for API compatibility;
the config settings and the `/mount/allowed-*` endpoints no longer exist.
`POST /api/v1/mount/lock` now honours `{"locked": false}` via `MountPolicy::Unlock()`
(automation endpoint only; guest-side callers cannot unlock). The temporary
`get_mount_allowed_bases`/`get_mount_allowed_image_roots` MCP tools were added and then removed.

### 8. Lua script load: JSON accepted

`POST /api/v1/script/load` rejects `application/json` with 415 upstream. The validator now
accepts `application/json`, and the handler unwraps a JSON string or `{"script": "…"}` to the
raw source; the MCP server additionally sends `text/plain` via `client::post_text`.

### 9. Headless / agent launcher

`tools/dosbox-mcp/src/dosbox_launcher.rs`: finds the build binary in repo-relative
`./build/release-linux/` (not a machine-specific path), resolves the symlinked
`~/.local/bin/dosbox-automation`, auto-detects `libiir.so` from the resolved binary path and
sets `LD_LIBRARY_PATH` for the child, uses offscreen SDL + dummy audio when no display is set,
keeps autoexec and kills the child without awaiting on stop, MT32EMU defaults off, and
attaches to an already-running instance instead of starting a second one.

### 10. Docs and tests

- `AI_SETUP.md` (new): from-scratch install/configure/build for CachyOS/Arch (SDL3, not SDL2).
- `tools/dosbox-mcp/docs/{api-reference,configuration,workflows,quickstart}.md`, `tools/dosbox-mcp/README.md`.
- `tests/integration/test_api_contract.py`: host test accepts any Host (was loopback-only).
- `CMakeLists.txt`: MT32EMU default off for headless runs.

### What is intentionally not changed

CPU/DOS/video/audio emulation, archive mounts, TrueType text output, Cheat Workbench,
Lua engine semantics, and the upstream security model for guest mounts (system-path denylist,
symlink checks) are as upstream at `b9f24198a`. Known pre-existing test failures in
`dosbox_tests` (archive-helper and stale mount-policy parse tests) are upstream/fork leftovers
and are not addressed by these deltas.

## Getting started

Download from the [releases page](https://github.com/dosbox-automation/dosbox-automation/releases), unpack, run. Linux ships as a portable tarball and AppImage, Windows as a zip and installer.

With a build from the current sources you can also hand it a game archive directly: `dosbox-automation "Commander Keen 4.zip"` mounts the zip as drive C: and starts the game. See [Games straight from their archives](#games-straight-from-their-archives) below.

To use the API, add `webserver_enabled = true` to the `[webserver]` section in your config. The Swagger UI is then at `http://localhost:8386` with every endpoint documented and testable.

The [project manual](https://www.dosbox-automation.org/) has the rest.

## Platform support

The table applies to 0.85.0 and to builds from the current sources.

| Distribution | Environment | Icon | Decoration |
|---|---|---|---|
| Debian 13 | X11 / KDE | yes | yes |
| Ubuntu 22.04 LTS | Wayland / GNOME | yes (1) | yes (2) |
| Ubuntu 26.04 LTS | Wayland / GNOME | yes (1) | yes (2) |
| Fedora 44 GNOME | Wayland / GNOME | yes (1) | yes (2) |
| Pop!_OS 22.04 | X11 / GNOME | yes (1) | yes |
| Linux Mint 21 | X11 / XFCE | yes | yes |
| Arch (vanilla) | XLibre / LXQt | yes | yes |
| Windows 10 | Windows DWM | yes | yes |

(1) User will be asked for permission to install the application icon for the dock.
(2) CSD decoration, light/dark aware, with XWayland native decoration without fallback.

The compat AppImage (glibc 2.35, for FUSE-based systems) will not have window decorations on GNOME on newer distributions due to a glibc version mismatch with the system's pangocairo. Use the standard AppImage there.

Running a combination not listed here? Open an issue with a screenshot showing the dock icon and title bar, plus the log lines where dosbox-automation prints the detected session type and display server. We will add verified setups to the table.

**macOS and FreeBSD:** dosbox-automation builds from source on both, and the full unit test suite passes on each: macOS on an Apple Silicon Mac mini, with Homebrew or vcpkg libraries, and FreeBSD 15 against its own packages. There are no release packages for either yet. The build pages for [macOS](docs/build/macos.md) and [FreeBSD](docs/build/freebsd.md) have the steps.

### Runtime dependencies (Linux tarball)

The AppImage is self-contained. The portable tarball links statically against SDL3 and its dependencies, but the display and audio backends are loaded at runtime from the system. Any desktop installation with a graphical session has these already.

On a minimal or headless system, install the following:

**Debian / Ubuntu:**
```
sudo apt install libgl1 libdecor-0-0 libxkbcommon0 libwayland-client0 \
  libx11-6 libxcursor1 libxrandr2 libxi6 \
  libasound2t64 libpulse0 libpipewire-0.3-0t64
```

**Fedora:**
```
sudo dnf install mesa-libGL libdecor libxkbcommon libwayland-client \
  libX11 libXcursor libXrandr libXi \
  alsa-lib pulseaudio-libs pipewire-libs
```

---

<p align="center">
  <img src="https://www.dosbox-automation.org/imgs/gallery/vga/budokan-vga.png" width="48%" alt="Budokan (VGA)">
  <img src="https://www.dosbox-automation.org/imgs/gallery/ega/commander-keen-4-ega.png" width="48%" alt="Commander Keen 4 (EGA)">
</p>
<p align="center">
  <img src="https://www.dosbox-automation.org/imgs/gallery/cga/popcorn-cga.png" width="48%" alt="Popcorn (CGA)">
  <img src="https://www.dosbox-automation.org/imgs/gallery/svga/simcity-2000-svga.png" width="48%" alt="SimCity 2000 (SVGA)">
</p>

<p align="center">
  <em>All screenshots captured from dosbox-automation's own rendering, with adaptive CRT shaders and correct aspect ratios - zero configuration.</em>
</p>

---

## Playing games

dosbox-automation runs DOS games with good defaults for graphics and sound. The automation features are additional, not a replacement for normal use.

<p align="center">
  <img src="https://www.dosbox-automation.org/imgs/gallery/svga/warcraft-2-svga.png" width="48%" alt="Warcraft 2 (SVGA)">
  <img src="https://www.dosbox-automation.org/imgs/gallery/tandy/grand-prix-circuit-tandy.png" width="48%" alt="Grand Prix Circuit (Tandy)">
</p>

**Better sound:** dosbox-automation includes the FluidR3 soundfont. General MIDI games sound right on the first run. Sound Blaster 16, AdLib, Gravis UltraSound and MT-32 emulation are included, as well as the traditional PC beeper for old games.

**CRT shaders:** A combination of shaders simulate the look of old CRT monitors of yesteryear. The right shader settings for each graphics mode are picked automatically across CGA, EGA, VGA, SVGA, Tandy, and Hercules, with correct aspect ratios and integer scaling.

**Sharp text with TrueType:** For long sessions in text mode, `ttf_output = true` draws the DOS screen with a scalable TrueType font instead of scaling up the 8x16 bitmap font. The bundled font is VileR's Flexi IBM VGA, which keeps the look of the VGA text mode and covers around 75 DOS code pages; any monospace font with the needed glyphs works too. Programs that load their own screen font keep it, because the output steps aside while the font is changed. FeralChild64 wrote the feature and tells the whole story, with pictures, in [TrueType text output for dosbox-automation](https://feralchild64.github.io/Articles/2026.10.21%20-%20TrueType%20text%20output%20for%20dosbox-automation.html). It is in the current sources and comes with the next release.

**A DOS that behaves like DOS:** APPEND works as it did on MS-DOS, `/X` and `/PATH` included, so the games and tools of the time that look for their data through it find it. SUBST and JOIN work too. The Y: drive carries a set of tools on the path: EDIT (Mateusz Viste's MSEDIT), our own VIEW for reading a file of any size as text or hex, down to an 8086 with DOS 2.0, plus FIND, SORT, XCOPY, DELTREE, DEBUG, ZIP and UNZIP.

**Video and image capture:** You can record your gameplay with CTRL+F7, dosboxctl (included helper script in source distribution) or programmatic API calls, either in the original screen resolution or as they are rendered on screen.  

**Safer to run:** DOS games you download can be set up to only access folders where your dosbox configuration lives and it is harder for malicious setups to do harm to your computer.

<p align="center">
  <img src="https://www.dosbox-automation.org/imgs/gallery/vga/legend-of-kyrandia-2-hand-of-fate-vga.png" width="32%" alt="Legend of Kyrandia 2: Hand of Fate (VGA)">
  <img src="https://www.dosbox-automation.org/imgs/gallery/hercules/blockout-hercules.png" width="32%" alt="Blockout (Hercules)">
  <img src="https://www.dosbox-automation.org/imgs/gallery/ega/railroad-tycoon-ega.png" width="32%" alt="Railroad Tycoon (EGA)">
</p>
<p align="center">
  <em>Left to right: Legend of Kyrandia 2: Hand of Fate on VGA, Blockout on Hercules, Railroad Tycoon on EGA.</em>
</p>

For the full feature set see the [documentation](https://www.dosbox-automation.org/).

---

## Games straight from their archives

Most DOS games reach you today as a zip file, and the usual routine is to unpack it into a folder, mount the folder, and keep track of which folder belonged to which game. With a build from the current sources you can skip that. dosbox-automation mounts the archive itself and the game runs from it. The archive stays exactly as it is, byte for byte.

```bash
dosbox-automation "Commander Keen 4.zip"
dosbox-automation "Keen Collection.zip" --run 'KEEN4\KEEN4E.EXE'
```

Given an archive on the command line, dosbox-automation mounts it as C:, starts the one program at its root and quits when that program ends. If there is more than one program, `--run` picks it. Without `--run`, a small menu comes up in DOS and lets you choose; ESC leaves you at the prompt. Whoever packs an archive can name the entries by hand: a `CHOOSER.TML` at its root gives the menu its title, labels and the order, and can mark the one entry that starts on its own.

Inside DOS the same works with MOUNT:

```
mount c d:\games\doom.zip
mount c doom.zip -run doom.exe -exit
mount c collection.zip/doom2.zip
```

ZIP, RAR, 7z and tar archives mount, and an archive inside an archive is reached by naming the path to it, as in the last line. Disk and CD images inside an archive mount like images on the host: a floppy inside a zip boots, a floppy set inside a 7z swaps with Ctrl+F4, and a CUE sheet finds the files it names next to it in the archive. Compressed single images (`.imz`, `.isz`, `disk1.img.xz`, `game.iso.gz`) mount as the image they contain.

The game can still write. Saved games, high scores and configuration files go to a save folder of the game's own, outside the archive, and the game sees them on its drive next to its original files. Delete that folder and the game is back to the state it came in. `-scratch` keeps the writes for that mount only, which suits an installer CD, and `-nosidecar` makes the drive read-only.

A few things are good to know. 7z, RAR 5 and tar archives cannot hand out one file without unpacking everything before it, so they are refused until you set `archive_sequential = cache`; then they are unpacked once into a cache and served from there. Solid RAR 4 archives are refused either way. The cache has a size cap and keeps unused entries for a week, and `mount -cache` shows what is in it. Archives that ask for a password, and archives split into volumes, are refused with a message that says so.

---

## Automate your DOS

dosbox-automation has an HTTP REST API and a Lua scripting engine built in. You can install games, type commands, swap disks, grab screenshots, and shut down the emulator without ever touching the keyboard.

To get started, enable the web server in your config. No token is required.

```ini
[webserver]
webserver_enabled = true
```

Start dosbox-automation, and the Swagger UI is at `http://localhost:8386` once the server is running.

```bash
# grab a screenshot
curl -s http://localhost:8386/api/v1/video/frame --output screenshot.png

# read what is on screen (text modes)
curl -s http://localhost:8386/api/v1/video/text

# type a command
curl -X POST http://localhost:8386/api/v1/input/type -d '{"text": "dir c:\\n"}'
```

Lua scripts run on the emulation thread with frame-accurate timing. They can wait for specific text on screen, navigate setup menus, swap floppy disks, and report results back through the API.

Games can also be installed from TOML recipe files. The Python reference harness in `tests/integration/` reads the recipe, fetches the media, runs the installer unattended, answers its questions and swaps the floppies when it asks. This is the DOOM shareware recipe, cut down to the parts that matter; the 7z with both floppy images is mounted as it is:

```toml
[game]
name = "DOOM Shareware"
media = "floppy"

[discs]
archive = "Doom Shareware Floppy (v1.666) [2 Disk].7z"
images = [
    "Doom Shareware Floppy (v1.666) [Disk 1 of 2].ima",
    "Doom Shareware Floppy (v1.666) [Disk 2 of 2].ima",
]

[installer]
path = "SETUP.EXE"
source_drive = "A"
target_drive = "C"

[prompts]
sequence = [
    { wait = "Available Drives", type = "C" },
    { wait = "insert disk DOOM 2", action = "swap:2", key = "enter" },
]
```

The [recipes page](https://www.dosbox-automation.org/0.85.1/automation/recipes/) of the manual has the full format.

Keyboard and mouse input can be recorded as JSON and replayed frame-accurately on another machine. The recording includes the CPU speed and keyboard layout so the replay is deterministic.

The API also gives you read and write access to guest memory, CPU registers and I/O ports, with memory search and value freezes on top. Useful for debugging DOS programs, building trainers, or writing test assertions that check what the game actually did.

**AI agents:** the bundled [dosbox-mcp](tools/dosbox-mcp/README.md) in this fork (`tools/dosbox-mcp`, 55 tools) connects any agent that speaks the Model Context Protocol to a running emulator. The agent types into DOS programs, reads the screen, takes frames and video, runs Lua scripts and inspects memory, all through the same token-free API. See [AI_SETUP.md](AI_SETUP.md) for from-scratch setup.

The test suite has over 2300 C++ unit tests and about 300 Python integration tests. Several of those are full game installations that run end to end through the API.

---

## The Cheat Workbench

The Cheat Workbench is a web page that dosbox-automation serves itself whenever the API is on, and a browser on the same machine connects to it without a token. Every download carries a small launcher that starts the emulator with the API on and a fresh token, and prints the page address, also for a browser on another machine. In the current sources you can open it from inside, too: type `WORKBENCH` at the DOS prompt or press Ctrl+Alt+W.

<p align="center">
  <img src="https://www.dosbox-automation.org/docs-images/manual/workbench-opened.jpg" width="80%" alt="The Cheat Workbench, connected, with Commander Keen 4 running">
</p>

The page puts everything the API can do with memory into one place. The scanner finds a value, say your ammo, and narrows it down scan by scan as the value changes. The cheat table keeps what you found, writes it, and freezes it so the health stays put. Beside them sit a hex viewer with bookmarks and write watches, the CPU registers, the DOS memory blocks and the program's PSP. The page can also capture video, screenshots and audio, set the mixer, type into the game, record your input and trim the recording on a timeline, and export it as JSON or as a Lua script you can replay.

The [Cheat Workbench page](https://www.dosbox-automation.org/0.85.1/automation/cheat-workbench/) of the manual walks through it panel by panel, and its [two-minute video](https://youtu.be/FeRsGde-AH4) finds, writes and freezes the ammo counter in Commander Keen 4.

---

## Video showcase

Recorded and scripted with the included reference harness, driving the emulator through the REST API.

| | |
|:---:|:---:|
| [**Quest for Glory IV**](https://www.dosbox-automation.org/#video-showcase) | [**System Shock**](https://www.dosbox-automation.org/#video-showcase) |
| Floppy install with automated disk swaps | CD-ROM install, fully unattended |
| [**DOOM shareware**](https://www.dosbox-automation.org/#video-showcase) | [**Epic Pinball**](https://www.dosbox-automation.org/#video-showcase) |
| Install and launch | Installation replay |
| [**The Incredible Machine**](https://www.dosbox-automation.org/#video-showcase) | [**Turbo Pascal 5.5**](https://www.dosbox-automation.org/#video-showcase) |
| Demo install by recipe | Compiler install, scripted end to end |

[Watch the videos on dosbox-automation.org](https://www.dosbox-automation.org/#video-showcase)

---

## Security

Upstream treats the HTTP server as an attack surface with bearer-token auth on every endpoint, host header validation, and mount restrictions. **This fork removes the bearer-token and host-header checks** (see [Fork deltas §2](#fork-deltas--how-this-fork-differs-from-upstream-dosbox-automation)): the API is token-free and bound to `0.0.0.0`, so anyone who can reach port 8386 can drive the emulator, read/write guest memory, and mount host directories (subject to the system-path denylist). Run it only on trusted networks/machines. The upstream [security policy](https://www.dosbox-automation.org/0.85.1/automation/security/) still describes the guest mount restrictions, which this fork keeps.

Security reports go to the contact address in the policy. Everything else goes on the [issue tracker](https://github.com/dosbox-automation/dosbox-automation/issues) (upstream) or this fork's repository.

---

## Build from source

- [General instructions](docs/build/README.md)
- [Linux](docs/build/linux.md)
- [Windows](docs/build/windows.md)
- [macOS](docs/build/macos.md)
- [FreeBSD](docs/build/freebsd.md)

---

## License

dosbox-automation is licensed under the GNU General Public License v2.0 or later, like the DOSBox Staging code it builds on. Bundled third-party libraries and tools keep their own licenses; [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt) lists them. UniMenu, the program menu on the bundled Y: drive, ships as a binary under its own freeware licence (resources/drives/y/dos/unimenu.txt).

---

<p align="center">
  <sub>dosbox-automation has been written with the help of tooled assistance, but has been designed, reviewed and thoroughly tested and signed off by its developer.</sub>
</p>
