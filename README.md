<!-- License: GPL-2.0-or-later, as in the upstream project. -->

# dosbox-automation — this is a FORK, not the original project

This repository is **not** the original dosbox-automation project. The original
lives at [github.com/dosbox-automation/dosbox-automation](https://github.com/dosbox-automation/dosbox-automation)
with its documentation at [dosbox-automation.org](https://www.dosbox-automation.org/).
This fork may be **fundamentally different** from the original: code, tooling,
defaults and behaviour here can diverge from upstream at any time. Do not assume
that what the original project describes applies to this repository, or the
other way around.

The original upstream README belongs to that repository; it is not reproduced
here. What this fork changes is documented below.

## Fork deltas — vs upstream dosbox-automation (`b9f24198a`)

- `tools/dosbox-mcp`: new 55-tool Rust MCP server (auto-starts the emulator, pins cycles).
- Webserver: token-free, no Host validation, bound to `0.0.0.0`, closes connections after each answer.
- Pause/resume API at an instruction boundary; bridge drains every loop pass.
- Memory API: page-guarded, dispatch-free host reads, guest-linear (bit-31) CR3 walk, `pagemap` endpoint.
- Input: burst-serialized with 409 retry, key aliases (`enter`, `esc`, …), `MOUSE_Inject*` + warp-as-delta.
- New endpoint `GET /input/mouse`: last injected cursor position.
- Registers: `eip`/`flags` writable, case-insensitive; cycles pinned for real+protected mode.
- Mount policy: whitelist removed (any existing dir mounts, system paths still refused); mount lock unlockable.
- Lua script load accepts JSON; `free_memory`/`allocate_memory` return JSON instead of an empty body.
- Headless launcher: repo-relative binary search, auto `libiir.so`, offscreen SDL, MT32EMU off, attaches to running instance.
- Docs: `AI_SETUP.md` (CachyOS/Arch) and `tools/dosbox-mcp/docs/`.
- Core emulation unchanged from upstream.
