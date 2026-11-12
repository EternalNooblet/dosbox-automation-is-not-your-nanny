<!-- License: GPL-2.0-or-later, as in the upstream project. -->
<!-- This README was changed from the upstream README in this fork: last
change 2026-11-11, fork base b9f24198a; full history: git log. Every file
changed from upstream in this fork carries its own dated change note. -->

# dosbox-automation — this is a custom FORK

The original
lives at [github.com/dosbox-automation/dosbox-automation](https://github.com/dosbox-automation/dosbox-automation)
with its documentation at [dosbox-automation.org](https://www.dosbox-automation.org/).
This fork may be **fundamentally different** from the original: code, tooling,
defaults and behaviour here can diverge from upstream at any time. Do not assume
that what the original project describes applies to this repository, or the
other way around.

The original upstream README belongs to that repository; it is not reproduced
here. What this fork changes is documented below.

## Fork deltas — vs upstream dosbox-automation

- `tools/dosbox-full-auto-mcp`: new 55-tool Rust MCP server (auto-starts the emulator, pins cycles).
- Webserver: token-free, no Host validation, bound to `0.0.0.0`, closes connections after each answer.
- Pause/resume API at an instruction boundary; bridge drains every loop pass.
- Memory API: page-guarded, dispatch-free host reads, guest-linear (bit-31) CR3 walk, `pagemap` endpoint.
- Input: burst-serialized with 409 retry, key aliases (`enter`, `esc`, …), `MOUSE_Inject*` + warp-as-delta.
- New endpoint `GET /input/mouse`: last injected cursor position.
- Registers: `eip`/`flags` writable, case-insensitive; cycles pinned for real+protected mode.
- Mount policy: whitelist re-enabled for webserver mode — paths must be under conf anchor or `mount_allowed_bases`/`mount_allowed_image_roots` from primary config; mount lock unlockable.
- Default mount root for MCP server is `~/.hermes/plugin-data/dosbox-full-auto/drives/` (created at install), keeping binaries outside the mountable tree.
- Lua script load accepts JSON; `free_memory`/`allocate_memory` return JSON instead of an empty body.
- Headless launcher: repo-relative binary search, auto `libiir.so`, offscreen SDL, MT32EMU off, attaches to running instance.
- Docs: `AI_SETUP.md` (CachyOS/Arch) and `tools/dosbox-full-auto-mcp/docs/`.
- Core emulation unchanged from upstream.
