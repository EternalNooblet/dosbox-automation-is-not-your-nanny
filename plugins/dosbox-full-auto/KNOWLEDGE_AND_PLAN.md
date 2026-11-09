# dosbox-full-auto Plugin — Clean Plan

## What Exists Now (Verified 2026-11-09)
- **Dev pair**: `dosbox-automation` built at `~/dosbox-automation-is-not-your-nanny/build/release-linux` (+ `~/.local/bin` symlink), `dosbox-mcp` at `tools/dosbox-mcp/target/release/dosbox-mcp`; registered in `~/.hermes/config.yaml` as `dosbox-mcp`, port 8386.
- **GitHub release workflow**: `.github/workflows/release.yml` exists (commit 134626dc9), triggers on `v*` tags. Linux x86_64 only; Windows/macOS jobs disabled (`if: false`).
- **Release v1.0.1 ships a self-contained tarball**, not loose binaries:
  `dosbox-full-auto-linux-x86_64.tar.gz` → `dosbox-full-auto-linux-x86_64/` containing `dosbox-mcp`, `dosbox-automation`, `resources/` (shaders, mapping, fonts, webserver), `_deps/iir1-build/libiir.so*`.
  Verified end-to-end: extracted bundle + MCP entry spawns server + emulator on **8387**, DOS shell boots — screen showed `Z:\>` prompt, banner `(13462)` matching the built commit.
  v1.0.0 (loose binaries only) was broken: `libiir.so.1: cannot open shared object file`, then `ABORT: RENDER: Error setting fallback shaders` — assets must ship beside the binaries.
- **MCP config shape (tested, correct)**: stdio server with args, NOT env `DOSBOX_AUTOMATION_PATH` (that env var does not exist; the launcher env var is `DOSBOX_AUTOMATION_BIN`):
  ```yaml
  mcp_servers:
    dosbox-full-auto:
      command: <extracted-dir>/dosbox-mcp
      args: [--dosbox-bin <extracted-dir>/dosbox-automation,
             --working-dir <extracted-dir>,
             --base-url http://localhost:8387,
             --cpu-cycles 30000]
  ```
  `--working-dir` is required: resources lookup resolves from cwd; libiir auto-detect resolves from the binary dir (`<dir>/_deps/iir1-build` → launcher sets LD_LIBRARY_PATH). Port 8387 keeps the release pair from colliding with the dev pair on 8386.
- **Registration fact**: adding a new `mcp_servers` entry is picked up mid-session by the gateway — tools register as `mcp_dosbox_full_auto_*` without restart (verified 2026-11-09). Only rebuilding a server binary needs a new session.
- **Skills loaded**: `dosbox-mcp-driving` + `decision-vllm-skill`; VLLM key automation (lazy verify, 24h cooldown, PoW); vision/d3 method.

## Plugin Goal
Package all of the above into **one directory plugin** called `dosbox-full-auto` that installs everything automatically.

## Plugin Structure
```
dosbox-full-auto/
├── plugin.yaml              # manifest, capabilities, python_deps
├── __init__.py              # register(ctx): download bundle + write MCP config + register skill
├── src/
│   └── install.py           # download tarball from GitHub Releases, extract to plugin_data_dir()/bin/linux-x86_64/
├── skills/
│   └── dosbox-full-auto/    # single merged skill
│       ├── SKILL.md
│       └── references/
├── pyproject.toml           # no dependencies — plugin runtime is stdlib-only
└── README.md
```

## register(ctx) Does 4 Things
1. **Download the bundle tarball** from GitHub Releases (latest `v*` tag) → extract to `plugin_data_dir()/bin/linux-x86_64/` (loose binaries are not usable — they abort without assets)
2. **Write MCP server config** via `hermes config set` (never hand-edit): the tested shape above — command + args, `--base-url http://localhost:8387`
3. **Register one skill**: `dosbox-full-auto` (merged from dosbox-mcp-driving + decision-vllm-skill + vision/d3)
4. **Print**: tools appear as `mcp_dosbox_full_auto_*` once the gateway discovers the entry; restart only needed if a server binary was rebuilt

## User Flow
```bash
hermes plugins install <url>
# plugin downloads tarball, extracts bundle, writes config entry
# gateway registers mcp_dosbox_full_auto_* tools (restart only if a rebuild happened)
```

## Required Setup (Separate Repo) — DONE
Workflow shipped in `dosbox-automation-is-not-your-nanny` (commit 134626dc9), release **v1.0.1** published with the Linux bundle and verified end-to-end. Windows/macOS jobs remain `if: false` until enabled.

## Hard Rules
- No custom MCP tools wrappers — agent calls `mcp_dosbox_full_auto__*` directly
- No computer-use provider, no runtime MCP registration API, no subprocess management
- Config edits via `hermes config set` only (agent-side config.yaml edits are blocked)
- Credentials via Hermes secret scope (`agent.secret_scope.get_secret("VLLM_API_KEY")`)
- VLLM key rotation: lazy verification, 24h cooldown persisted in `plugin_db()`

## Capabilities (plugin.yaml)
```yaml
capabilities:
  skills: true
  requires_env:
    - VLLM_API_KEY
```

---

## Status
- **Workflow + release v1.0.1**: DONE, verified end-to-end (bundle boots through MCP; DOS prompt vision-confirmed)
- **Test teardown**: release test pair, test config entry, and all test files removed after verification
- **Plugin scaffold**: DONE (2026-11-09) — `hermes plugins validate` + `doctor` clean; register() probe-tested with a stub ctx; status()/release lookup verified live (v1.0.1, asset name exact). Deliberate deviations from the plan above (register stays light — install is explicit via `/dosbox-full-auto install`; `hermes mcp add` instead of `hermes config set`; no `capabilities:` block) are documented in README.md
- **Location**: moved into this repo at `plugins/dosbox-full-auto/` (2026-11-09) — a monorepo, so the catalog entry uses `subdir: plugins/dosbox-full-auto` and the owner is the submitter (rule 5). Re-validated clean at the new path (Hermes 0.22.7, `requires_hermes: ">=0.22.7"`, no python dependencies — stdlib only)
- **Next**: commit + push this repo; then `hermes plugins install ~/dosbox-automation-is-not-your-nanny/plugins/dosbox-full-auto`, run `/dosbox-full-auto install`, verify `mcp_dosbox_full_auto_*` appear; then catalog entry PR (repo + subdir + 40-char SHA) in hermes-agent repo; later enable Windows/macOS jobs in the workflow
