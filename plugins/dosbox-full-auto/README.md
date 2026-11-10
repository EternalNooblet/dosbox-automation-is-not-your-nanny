# dosbox-full-auto

One-directory DOSBox full automation for Hermes Agent (Linux x86_64, Windows x86_64).

Downloads the self-contained release bundle from
[dosbox-automation-is-not-your-nanny](https://github.com/EternalNooblet/dosbox-automation-is-not-your-nanny)
releases, extracts it into the plugin data dir, registers it as the
`dosbox-full-auto` MCP server on port 8387, and ships the merged driving skill.

## Use

```bash
hermes plugins install <path-or-git-url>   # from this directory or a pushed repo
```

In a session:

```
/dosbox-full-auto status        bundle + MCP entry + VLLM key state
/dosbox-full-auto install       download bundle, register MCP server
/dosbox-full-auto reinstall     force a fresh download
/dosbox-full-auto create-key    create new VLLM API key via PoW (respects 5/hr limit)
/dosbox-full-auto key-status    show VLLM key status (verifies with API)
```

Or: `hermes dosbox-full-auto status|install|reinstall|create-key|key-status`.

After install, tools appear as `mcp_dosbox_full_auto__*` in the same session
(a new `mcp_servers` entry is discovered mid-session). Restart only if the
server binary itself was rebuilt.

## What it downloads and runs (read this before installing)

Setup (`install`) fetches ONE native bundle — about 16.5 MB — for the current
platform from the official GitHub Releases of the upstream project
[EternalNooblet/dosbox-automation-is-not-your-nanny](https://github.com/EternalNooblet/dosbox-automation-is-not-your-nanny/releases)
(tag `v1.0.2`), verifies its SHA256 against the pinned digest in
`src/install.py` (`PINNED_DIGESTS`, the digest GitHub publishes for the asset),
and only then extracts and executes it. Unpinned release tags are refused —
nothing installs unless its checksum is in the table. The bundle contains
native executables (`dosbox-mcp`, `dosbox-automation`), `resources/` (shaders,
fonts, webserver assets) and `libiir.so` (Linux) / equivalent DLLs (Windows).
Nothing is bundled in the plugin repo itself — this is a setup-time fetch, not
a shipped payload (Hermes catalog rule 17), disclosed here per rule 13:

- Network: GitHub API + release asset download at install time; Decision API
  (`decision.vllm-sr.ai`) for visual checks. **VLLM API key managed automatically
  via Hermes secret scope.** Key creation via PoW respects strict rate limits:
  5 keys/IP/hour, 20/day (12 min cooldown between creations).
- Files: writes only under `<hermes home>/plugin-data/dosbox-full-auto/`
  (binaries, VERSION, plugin_db). No reads outside the plugin's own data and
  the one MCP config entry it registers via `hermes mcp add`.
- Processes: registers a local stdio MCP server on port 8387; Hermes spawns
  it on demand and it spawns the emulator as its child. No background
  daemons, no telemetry.
- Credentials: `VLLM_API_KEY` managed automatically via Hermes secret scope
  (auto-created via PoW if missing/dead), sent as bearer token to Decision API only.

## Why a bundle, not loose binaries

v1.0.0 shipped loose binaries and aborted twice over: `libiir.so.1: cannot
open shared object file`, then `ABORT: RENDER: Error setting fallback
shaders`. The tarball ships `dosbox-mcp`, `dosbox-automation`, `resources/`
(shaders/mapping/fonts/webserver) and `_deps/iir1-build/libiir.so*` together;
the emulator resolves resources from its cwd (hence `--working-dir`) and the
launcher sets `LD_LIBRARY_PATH` from the bundle dir.

## Requirements

- Linux x86_64 or Windows x86_64.
- **VLLM API key managed automatically** — created via PoW when needed, stored in Hermes secret scope. Strict rate limits: 5 keys/IP/hour, 20/day.
- Ports: MCP on 8387 (the dev pair keeps 8386).
