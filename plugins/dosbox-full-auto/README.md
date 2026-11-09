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
```

Or: `hermes dosbox-full-auto status|install|reinstall`.

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
  (`decision.vllm-sr.ai`) for visual checks, key verified lazily via
  `GET /api/keys/self` (24 h cooldown, key never stored by the plugin).
- Files: writes only under `<hermes home>/plugin-data/dosbox-full-auto/`
  (binaries, VERSION, plugin_db). No reads outside the plugin's own data and
  the one MCP config entry it registers via `hermes mcp add`.
- Processes: registers a local stdio MCP server on port 8387; Hermes spawns
  it on demand and it spawns the emulator as its child. No background
  daemons, no telemetry.
- Credentials: `VLLM_API_KEY` from the Hermes secret scope / environment,
  sent as a bearer token to the Decision API only.

## Why a bundle, not loose binaries

v1.0.0 shipped loose binaries and aborted twice over: `libiir.so.1: cannot
open shared object file`, then `ABORT: RENDER: Error setting fallback
shaders`. The tarball ships `dosbox-mcp`, `dosbox-automation`, `resources/`
(shaders/mapping/fonts/webserver) and `_deps/iir1-build/libiir.so*` together;
the emulator resolves resources from its cwd (hence `--working-dir`) and the
launcher sets `LD_LIBRARY_PATH` from the bundle dir.

## Requirements

- Linux x86_64 or Windows x86_64.
- `VLLM_API_KEY` in the Hermes secret scope / environment (Decision API key;
  verified lazily, 24 h cooldown in `plugin_db()`).
- Ports: MCP on 8387 (the dev pair keeps 8386).

## Deviations from KNOWLEDGE_AND_PLAN.md (deliberate, verified against the loader)

- `register(ctx)` does NOT download on load: the capability probe runs it in
  isolation, so it registers skill + commands only. Install is explicit via
  `/dosbox-full-auto install` or the CLI (`src/install.py`).
- MCP config is written via `hermes mcp add` (piped `Y`; `--args` last) and
  verified by readback — the supported CLI writer — rather than
  `hermes config set` on a nested dict.
- No `capabilities:` block: valid capability ids are the trust-gated list
  (`tools.override`, `llm.*`, …); this plugin needs none of them.
- No Python dependencies declared: the plugin is stdlib-only (`urllib`, `tarfile`, `subprocess`). The only third-party imports in the tree are `numpy`/`PIL` in `skills/dosbox-full-auto/scripts/blackcheck.py`, an agent-run helper that uses the host's existing environment.

## Later

- VLLM key rotation (lazy verify + 24 h cooldown exists; rotation never
  creates keys from a flagged IP).
