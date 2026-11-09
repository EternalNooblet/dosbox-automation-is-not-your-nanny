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


def _status() -> str:
    from .src import install as installer

    return installer.status()


def _handle_slash(raw_args: str) -> str | None:
    """`/dosbox-full-auto [install|status|reinstall|help]` — never raises."""
    try:
        parts = raw_args.strip().split()
        cmd = parts[0] if parts else "status"
        if cmd == "install":
            return _install(reinstall=False)
        if cmd == "reinstall":
            return _install(reinstall=True)
        if cmd == "status":
            return _status()
        if cmd in ("help", "-h", "--help"):
            return ("dosbox-full-auto — DOSBox full automation\n"
                    "  /dosbox-full-auto status      bundle + MCP entry + VLLM key state\n"
                    "  /dosbox-full-auto install     download release bundle, register MCP server\n"
                    "  /dosbox-full-auto reinstall   force a fresh download\n"
                    "Skill: dosbox-full-auto (merged MCP-driving + Decision API rules)")
        return f"unknown subcommand: {cmd} (try help)"
    except Exception as exc:  # a slash handler error must never break the turn
        return f"dosbox-full-auto error: {exc}"


def _cli_setup(parser: argparse.ArgumentParser) -> None:
    subs = parser.add_subparsers(dest="dosbox_full_auto_command", required=False)
    subs.add_parser("status", help="bundle + MCP entry + VLLM key state")
    subs.add_parser("install", help="download release bundle, register MCP server")
    subs.add_parser("reinstall", help="force a fresh download")


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
        args_hint="[install|status|reinstall]",
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
