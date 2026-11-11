# This file is part of the dosbox-automation-is-not-your-nanny fork
# License: GPL-2.0-or-later. This file is original work of the fork. Contact: EternalNooblet <67196335+EternalNooblet@users.noreply.github.com>.

"""dosbox-full-auto plugin — DOSBox full automation in one directory.

register() stays light by design: the loader's capability probe runs it in
isolation, so it must not do network I/O or config writes. It only registers
the merged skill plus two entry points; the actual download + MCP registration
happens on demand via /dosbox-full-auto install or `hermes dosbox-full-auto
install` (src.install). Tools appear as mcp_dosbox_full_auto__* once the MCP
server entry is written — the gateway picks up a NEW mcp_servers entry
mid-session, no restart needed (a rebuilt server binary does need one).
"""

from __future__ import annotations

import argparse
import logging
from pathlib import Path

logger = logging.getLogger(__name__)

_SKILL_DIR = Path(__file__).parent / "skills" / "dosbox-full-auto"
SKILL_MD = _SKILL_DIR / "SKILL.md"


def _install(*, tag: str | None = None, reinstall: bool = False) -> str:
    from .src import install as installer

    return installer.install(tag=tag, reinstall=reinstall)


def _create_key() -> str:
    """Explicit key minting ONLY — never implicit (status must not do this).
    Two-step: the first run shows the terms URL and creates nothing; a second,
    explicit create-key within TERMS_CONFIRM_WINDOW_S accepts and mints."""
    from .src.vllm_keys import TERMS_URL, terms_gate_pending, terms_gate_show, create_vllm_key

    if not terms_gate_pending():
        terms_gate_show()
        return (f"terms of use: {TERMS_URL}\n"
                "No key was created yet — the terms have not been accepted on your "
                "behalf without you seeing them. If you accept them, run create-key "
                "again to mint the key.")
    key = create_vllm_key()
    if key:
        return (
            "terms accepted; a new key was created, but it is NOT stored by the plugin "
            "(Hermes has no secret-scope setter) — set it yourself: `hermes config set "
            "VLLM_API_KEY <key>` or add VLLM_API_KEY=<key> to your profile .env. "
            "Until it is set, status reports it unset and every create-key run mints a fresh one "
            "(burns quota)."
        )
    return "key creation failed (rate limits 5/hr, 20/day, or PoW submission refused)"


def _status() -> str:
    from .src import install as installer

    return installer.status()


def _handle_slash(raw_args: str) -> str | None:
    """/dosbox-full-auto [install|status|reinstall|create-key|key-status|help] — never raises."""
    try:
        parts = raw_args.strip().split()
        cmd = parts[0] if parts else "status"
        if cmd == "install":
            return _install(reinstall=False)
        if cmd == "reinstall":
            return _install(reinstall=True)
        if cmd == "status":
            return _status()
        if cmd == "create-key":
            return _create_key()
        if cmd == "key-status":
            from .src.vllm_keys import _vllm_key, _verify_key
            key = _vllm_key()
            if not key:
                return (
                    "VLLM_API_KEY not set — set it via `hermes config set VLLM_API_KEY <key>` "
                    "or your profile .env (or mint one with create-key; it shows the terms first)"
                )
            return "VLLM_API_KEY: active" if _verify_key(key) else "VLLM_API_KEY: invalid/dead"
        if cmd in ("help", "-h", "--help"):
            return ("dosbox-full-auto — DOSBox full automation\n"
                    "  /dosbox-full-auto status      bundle + MCP entry + VLLM key state\n"
                    "  /dosbox-full-auto install     download release bundle, register MCP server\n"
                    "  /dosbox-full-auto reinstall   force a fresh download\n"
                    "  /dosbox-full-auto create-key  mint new VLLM key: 1st run shows terms, re-run accepts\n"
                    "  /dosbox-full-auto key-status  show VLLM key status\n"
                    "Skill: dosbox-full-auto (merged MCP-driving + Decision API rules)")
        return f"unknown subcommand: {cmd} (try help)"
    except Exception as exc:  # a slash handler error must never break the turn
        return f"dosbox-full-auto error: {exc}"


def _cli_setup(parser: argparse.ArgumentParser) -> None:
    subs = parser.add_subparsers(dest="dosbox_full_auto_command", required=False)
    subs.add_parser("status", help="bundle + MCP entry + VLLM key state")
    subs.add_parser("install", help="download release bundle, register MCP server")
    subs.add_parser("reinstall", help="force a fresh download")
    subs.add_parser("create-key", help="mint a new VLLM API key — shows the terms of use first; re-run within 10 min to accept (respects 5/hr limit)")
    subs.add_parser("key-status", help="show VLLM key status (verifies with API)")


def _cli_dispatch(args: argparse.Namespace) -> int:
    sub = getattr(args, "dosbox_full_auto_command", None)
    if sub is None or sub == "status":
        print(_status())
        return 0
    if sub == "install":
        print(_install(reinstall=False))
        return 0
    if sub == "reinstall":
        print(_install(reinstall=True))
        return 0
    if sub == "create-key":
        print(_create_key())
        return 0
    if sub == "key-status":
        from .src.vllm_keys import _vllm_key, _verify_key
        key = _vllm_key()
        if not key:
            print(
                "VLLM_API_KEY not set — set it via `hermes config set VLLM_API_KEY <key>` "
                "or your profile .env (or mint one with create-key; it shows the terms first)"
            )
            return 0
        if _verify_key(key):
            print("VLLM_API_KEY: active")
        else:
            print("VLLM_API_KEY: invalid/dead")
        return 0
    print(f"unknown subcommand: {sub}", flush=True)
    return 2


def register(ctx) -> None:
    """Register the merged skill and the install/status entry points."""
    ctx.register_skill(
        "dosbox-full-auto",
        SKILL_MD,
        "Drive DOSBox via mcp_dosbox_full_auto__* tools (MCP only, never the "
        "webserver directly) plus Decision API visual verification.",
        frontmatter={"name": "dosbox-full-auto"},
    )
    ctx.register_command(
        "dosbox-full-auto",
        handler=_handle_slash,
        description="Install/status the DOSBox full-auto release bundle and MCP server.",
        args_hint="[install|status|reinstall|create-key|key-status]",
    )
    ctx.register_cli_command(
        name="dosbox-full-auto",
        help="DOSBox full-auto bundle install/status",
        setup_fn=_cli_setup,
        handler_fn=_cli_dispatch,
        description=("Download the self-contained dosbox-full-auto release bundle "
                     "(linux-x86_64) and register it as the dosbox-full-auto MCP "
                     "server on port 8387."),
    )