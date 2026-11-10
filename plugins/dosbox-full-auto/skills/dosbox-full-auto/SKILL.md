---
name: dosbox-full-auto
description: "Drive DOSBox via the dosbox-full-auto MCP server (tools mcp_dosbox_full_auto__*) and verify visuals with Vision LLM + Decision API (vllm-sr/d3-ultra). Use for any task touching the running DOSBox emulator or asking Decision API questions."
---

# DOSBox Full-Auto (merged skill)

Merged from `dosbox-mcp-driving` + `decision-vllm-skill` + the vision/d3 method.
Source skills stay installed; this is the single entry point the plugin ships.

## Install / status

- `/dosbox-full-auto status` — bundle present? MCP entry registered? VLLM key state?
- `/dosbox-full-auto install` — download the release bundle, register the MCP server.
- CLI: `hermes dosbox-full-auto status|install|reinstall`.

The release ships a self-contained tarball per platform (binaries + `resources/` +
`_deps/iir1-build/libiir.so*` on Linux, DLLs on Windows). Loose binaries abort
(`libiir.so.1` missing, then RENDER fallback-shader abort) — never use them
without assets.
Bundle lives at `<plugin-data>/bin/<arch>/dosbox-full-auto-<platform>/`.

MCP config shape (tested, only via `hermes mcp add` — never hand-edit):

```yaml
mcp_servers:
  dosbox-full-auto:
    command: <bundle>/dosbox-mcp
    args: [--dosbox-bin <bundle>/dosbox-automation,
           --working-dir <bundle>,           # required: resources resolve from cwd
           --base-url http://localhost:8387,  # keeps the dev pair on 8386 separate
           --cpu-cycles 30000]
```

Tools appear as `mcp_dosbox_full_auto__*`. A new `mcp_servers` entry is picked
up mid-session; only a rebuilt server binary needs a new session.

## Rule #1 — MCP tools only

Drive DOSBox only through `mcp_dosbox_full_auto__*` (55 tools: memory, ports,
registers, capture, recording, scripts, mounts, mixer, mouse, frame analysis).
No HTTP calls to the emulator webserver, no xdotool/keystroke feeding,
no memory access outside MCP tools. Read-only OS checks (`ps`, ports) allowed.

## Rule #2 — fix the server, never work around it

A missing capability or failing call is a bug in dosbox-mcp: fix and rebuild
it (`cargo build --release` in `tools/dosbox-mcp`), never substitute a
direct-API workaround.

## Wait discipline

Never wait >20 s in one step. Poll state (registers/status) in <=20 s chunks
until the condition appears. Long fixed waits are wrong (boot countdowns vary
per run). ~59 s circuit-breaker cooldowns: poll in <=20 s chunks, not a sleep.

## Restart flow (wedged instance / clean boot)

1. `shutdown` — emulator exits; the server process SURVIVES without relaunch.
2. `kill <server_pid>` (`pgrep -af dosbox`) — the server must die to respawn.
3. Any MCP call respawns server + emulator with identical flags.
4. Mounts die with the emulator — remount and re-enter commands every respawn.

## Graphics-mode workflow (vision+d3)

1. `type_text "PROG.EXE\n"` — `\n` is Enter; bursts queue, so read after the burst settles.
2. `get_status` — authoritative `program`/`canonical_name`/`is_shell` check.
3. **Vision+d3 pipeline:**
   a. `capture_frame(mode="rendered", format="png")` → get base64 PNG
   b. **Vision LLM first pass** — `vision_analyze` on the screenshot: describe UI, locate elements, get coarse positions
   c. **Decision API precision** — `capture_frame` + Decision API (`vllm-sr/d3-ultra`) with `box`/`noul`/`choice` questions for exact coordinates/confirmation
   d. Use Vision LLM output to frame Decision API questions; use Decision API output for precise mouse/keyboard actions
4. Mouse: `send_input_sequence`; `mouse_move` requires `x_rel`/`y_rel` (one big relative move to clamp to edges/corners). `get_mouse_position` reports the last injected position — close the loop with it.
5. Pointer calibration without pixel math: relative moves + re-measure loop (d3 cursor box, delta px, move delta/scale ≈0.5 px/unit, re-measure until the noul gate "cursor tip is on X" is ~>=0.95 before committing).

Key names: `KBD_*` canonical + friendly aliases (`enter`, `esc`, `space`, `left`, `a`-`z`, `0`-`9`, `f1`-`f12`), case-insensitive, prefix optional.
`analyze_frame` gives black ratio/luma/hash (works in graphics mode).

## Black-watch loop

After launching a program, poll ~every 2 s for 10 s:
`capture_frame(mode="raw", format="png", save_path=<scratch>)` then
`python3 scripts/blackcheck.py <file>`. Any NOT_BLACK = UI alive, stop and
report. BLACK for 10 s = restart flow, remount, re-run, watch again.

## Memory reads — guardrails

- Host RAM/ROM pages only; everything else rejects with a fast HTTP 400.
  Avoid page 0x1B6000 (killed connections once).
- Reads are dispatch-free host copies (never dispatch handler/TLB logic on
  a running guest — that kills the webserver connection -> 500s until restart).
- Guest paging is dynamic while running: re-probe per boot, never assume a
  previously readable address is still mapped. For coherent multi-page
  captures: `pause`, dump, `resume`, verify with `get_pause_state`.
- Circuit breaker: ~3 consecutive MCP failures -> ~59 s cooldown. Interleave
  a known-good anchor read between probes; never fire probes back-to-back.
- `get_page_map` flags are raw 0x01/0x00, not '1'/'0'.

## Decision API (visual verification)

Full detail: references/decision-api.md. Only model: `vllm-sr/d3-ultra`.
Key: `${VLLM_API_KEY}`, verified lazily with a 24 h plugin_db() cooldown;
never create keys from a flagged IP and never print/store key text.
Batch all questions about one input into ONE request; respect 429 Retry-After
with backoff + jitter; never send sensitive data (everything may be logged).

## Conventions

Prefer MCP tools for everything. `get_prompt` is the MCP server's prompts,
not the DOS prompt. A malformed `tool_call` executes nothing — retry with
corrected arguments. Black screen while `get_status` shows the program
running = transient startup hang: check `get_cycles` + identical command
lines, then restart flow — don't chase config.
