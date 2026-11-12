# This file is part of the dosbox-full-auto fork
# License: GPL-2.0-or-later. This file is original work of the fork. Contact: EternalNooblet <67196335+EternalNooblet@users.noreply.github.com>.

"""dosbox-full-auto — install + status logic (stdlib only).

The release ships ONE self-contained tarball (binaries alone abort without
assets: libiir.so.1 missing, then RENDER fallback-shader abort — v1.0.0).
All paths resolve under the plugin data dir, never the plugin source dir
(which is git-pulled by ``plugins update`` and deleted by ``plugins remove``).

Hard rules:
- No custom MCP tool wrappers — the agent calls mcp_dosbox_full_auto__* directly.
- No subprocess management of the server/emulator (Hermes spawns the stdio
  MCP server on demand; it spawns the emulator as its child).
- Config writes only via the Hermes CLI (``hermes mcp add``), never a
  hand-edited config.yaml.
- VLLM API key never created automatically: minted ONLY by an explicit
  ``create-key`` run, after the terms URL was shown first (two-step confirm).
  Not stored by the plugin (Hermes has no secret-scope setter) — the user sets
  VLLM_API_KEY via ``hermes config set`` or their profile .env.
  Rate limits: 5 keys/IP/hour, 20/day.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tarfile
import time
import urllib.request
from pathlib import Path

from .vllm_keys import _db, PLUGIN_NAME, VLLM_BASE

GITHUB_REPO = "EternalNooblet/dosbox-full-auto"
BASE_URL = "http://localhost:8387"
CPU_CYCLES = 30000
VLLM_COOLDOWN_S = 24 * 3600
VERSION_FILE = "VERSION"

# Platform-specific asset names and bundle top-level dirs
def _asset_name() -> str:
    import sys
    if sys.platform.startswith("win"):
        return "dosbox-full-auto-windows-x86_64.tar.gz"
    return "dosbox-full-auto-linux-x86_64.tar.gz"

def _bundle_topdir() -> str:
    import sys
    if sys.platform.startswith("win"):
        return "dosbox-full-auto-windows-x86_64"
    return "dosbox-full-auto-linux-x86_64"

# Hermes catalog rule 17: a binary fetched at setup time must come from the
# upstream tool's official source, pinned by checksum, and disclosed (rule 13).
# These digests are GitHub's published asset digests (sha256) for each release
# tag. install() only installs a pinned tag and verifies the download against
# this table BEFORE extracting — an unpinned tag is refused, never fetched.
DEFAULT_TAG = "v1.0.7"
PINNED_DIGESTS = {
    "v1.0.7": {
        "linux": "5754c6b72d8b27c65638ce50c5b7c7a77e22a9f3465908e1707cd6ffeb4b4c3a",
        "windows": "138c596a91282ac1bac17fdbd803ddc5c5b8289c5755b1ce5236502516d6555f",
    },
    "v1.0.6": {
        "linux": "8af4b45b61e8e99f56e074731634a186fb1fd714f433319f347d5319ed2ffa96",
        "windows": "a67db774dc1e4685955b919e0c7ffd0078ed031ad5c73cb28b0fbc1f7383b51c",
    },
    "v1.0.4": {
        "linux": "05791369e3297c15245be128258b4a541d96588cbb5f552b499fd8fb38b5d1c7",
        "windows": "b801c710422f80748b62acffa75c3712ce72736522165fa18d79fa47f4729f4f",
    },
    "v1.0.3": {
        "linux": "61ea3df52057628873c1ded6d3a65677a1fe00b6f27e9765e804dcc2aa38e486",
        "windows": "d481df74fb3ef09ced47c5168849bd25b1fea2a14c4664d6d42a8993f0c855a5",
    },
    "v1.0.2": {
        "linux": "3d7ade5d836a1e12e4c399a8b7d5065fead0620a4104bdf688a8c580004f8d47",
        "windows": "d8de307043ab4a10044d1deecb0a70eca8f515f848869f4ad4509900f24a7f86",
    },
    "v1.0.1": {
        "linux": "0cd20f89dc8c20354589c5cc339a35eb6d0de6efb685983173ece131808b32a2",
    },
}


# -- paths -------------------------------------------------------------------

def data_root() -> Path:
    """<hermes home>/plugin-data/dosbox-full-auto/ (profile-safe, per call)."""
    try:
        from plugins.plugin_storage import plugin_data_dir

        return plugin_data_dir(PLUGIN_NAME)
    except Exception:
        pass
    try:
        from hermes_constants import get_hermes_home

        home = Path(get_hermes_home())
    except Exception:
        home = Path(os.environ.get("HERMES_HOME", str(Path.home() / ".hermes")))
    root = home / "plugin-data" / PLUGIN_NAME
    root.mkdir(parents=True, exist_ok=True)
    return root


def drives_root() -> Path:
    """<hermes home>/plugin-data/dosbox-full-auto/drives/ — the mount root."""
    root = data_root() / "drives"
    root.mkdir(parents=True, exist_ok=True)
    return root


def bundle_dir() -> Path:
    import sys
    arch = "windows-x86_64" if sys.platform.startswith("win") else "linux-x86_64"
    return data_root() / "bin" / arch / _bundle_topdir()


# -- GitHub release fetch ----------------------------------------------------

def _api_get(url: str, timeout: float = 30.0):
    req = urllib.request.Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": PLUGIN_NAME,
        },
    )
    with urllib.request.urlopen(req, timeout=timeout) as res:
        return json.load(res)


def latest_release(tag: str | None = None) -> dict:
    """Newest public release (or a specific tag). Never raises to the caller
    of install(); the returned dict carries ok/error on failure."""
    try:
        if tag:
            return {"ok": True, "release": _api_get(
                f"https://api.github.com/repos/{GITHUB_REPO}/releases/tags/{tag}")}
        return {"ok": True, "release": _api_get(
            f"https://api.github.com/repos/{GITHUB_REPO}/releases/latest")}
    except Exception as exc:  # network, 404, JSON — all fail the install, loudly
        return {"ok": False, "error": f"release lookup failed: {exc}"}


def _download(url: str, dest: Path, timeout: float = 300.0) -> None:
    req = urllib.request.Request(url, headers={"User-Agent": PLUGIN_NAME})
    with urllib.request.urlopen(req, timeout=timeout) as res, dest.open("wb") as out:
        shutil.copyfileobj(res, out)


def _find_asset(release: dict) -> dict | None:
    target = _asset_name()
    for asset in release.get("assets", []) or []:
        if asset.get("name") == target:
            return asset
    return None


def _file_sha256(path: Path) -> str:
    import hashlib

    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


# -- install -----------------------------------------------------------------

def _verify_bundle(root: Path) -> list[str]:
    """Missing-piece list; empty = usable bundle. Loose binaries are not usable."""
    import sys
    missing = []
    for name in ("dosbox-full-auto-mcp", "dosbox-automation"):
        p = root / name
        if sys.platform.startswith("win"):
            p = p.with_suffix(".exe")
        if not p.is_file():
            missing.append(f"binaries: {name} missing")
        elif not os.access(p, os.X_OK):
            missing.append(f"binaries: {name} not executable")
    if not (root / "resources").is_dir():
        missing.append("resources/ missing (shaders/mapping/fonts/webserver)")
    iir = root / "_deps" / "iir1-build"
    if sys.platform.startswith("win"):
        # Windows: look for iir DLLs
        if not iir.is_dir() or not list(iir.glob("*.dll")):
            missing.append("_deps/iir1-build/iir DLLs missing")
    else:
        # Linux: look for libiir.so*
        if not iir.is_dir() or not list(iir.glob("libiir.so*")):
            missing.append("_deps/iir1-build/libiir.so missing")
    return missing


def install(tag: str | None = None, reinstall: bool = False) -> str:
    """Download + extract the pinned release bundle, then register the MCP
    server. Only tags in PINNED_DIGESTS install (checksum pin, rule 17);
    anything else is refused. Returns a human-readable result; never raises."""
    tag = tag or DEFAULT_TAG
    import sys
    platform_key = "windows" if sys.platform.startswith("win") else "linux"
    expected = PINNED_DIGESTS.get(tag, {}).get(platform_key)
    if not expected:
        pinned = ", ".join(f"{t} ({', '.join(PINNED_DIGESTS[t].keys())})" for t in sorted(PINNED_DIGESTS))
        return (f"install refused — release {tag} is not checksum-pinned for {platform_key} "
                f"(rule 17). Pinned: {pinned}. "
                "Add its published SHA256 to PINNED_DIGESTS in src/install.py first.")
    rel = latest_release(tag)
    if not rel["ok"]:
        return f"install failed — {rel['error']}"
    release = rel["release"]
    asset = _find_asset(release)
    if asset is None:
        return f"install failed — release {release.get('tag_name')} has no asset {_asset_name()}"
    version = release.get("tag_name", "unknown")
    root = bundle_dir()
    if root.is_dir() and not _verify_bundle(root):
        if not reinstall:
            return (f"already installed: bundle {version} present at {root}\n"
                    "Use reinstall to force a fresh download.")
    if root.exists():
        shutil.rmtree(root)
    root.parent.mkdir(parents=True, exist_ok=True)
    tar_path = root.parent / _asset_name()
    try:
        _download(asset["browser_download_url"], tar_path)
        actual = _file_sha256(tar_path)
        if actual != expected:
            tar_path.unlink(missing_ok=True)
            return (f"install failed — checksum mismatch for {tag} "
                    f"(expected {expected[:12]}…, got {actual[:12]}…). Refusing to extract.")
        published = (asset.get("digest") or "").removeprefix("sha256:")
        if published and published != expected:
            return ("install failed — upstream published digest does not match the "
                    "pinned checksum. Refusing to extract.")
        with tarfile.open(tar_path, "r:gz") as tar:
            tar.extractall(root.parent, filter="data")
        tar_path.unlink(missing_ok=True)
    except Exception as exc:
        return f"install failed — download/extract failed: {exc}"
    for name in ("dosbox-full-auto-mcp", "dosbox-automation"):
        try:
            os.chmod(root / name, 0o755)
        except OSError:
            pass
    missing = _verify_bundle(root)
    if missing:
        return "install failed — bundle incomplete:\n" + "\n".join(missing)
    (root / VERSION_FILE).write_text(version + "\n", encoding="utf-8")
    reg = register_mcp_server(root)
    return (f"installed bundle {version} at {root}\n"
            f"mcp server 'dosbox-full-auto' registered on {BASE_URL} "
            f"(tools appear as mcp_dosbox_full_auto_*; no restart needed for a new\n"
            f"config entry — restart only if the server binary itself was rebuilt)\n{reg}")


def register_mcp_server(root: Path) -> str:
    """Register via `hermes mcp add` (the supported config writer). The prompt
    is piped — `hermes mcp add` asks interactively even on non-TTY stdin and a
    piped run without the answer is cancelled and saves nothing. `--args`
    must be last: it consumes everything after it."""
    import sys
    mcp_bin = root / "dosbox-full-auto-mcp"
    emu_bin = root / "dosbox-automation"
    if sys.platform.startswith("win"):
        mcp_bin = mcp_bin.with_suffix(".exe")
        emu_bin = emu_bin.with_suffix(".exe")
    cmd = [
        "hermes", "mcp", "add", PLUGIN_NAME,
        "--command", str(mcp_bin),
        "--args",
        "--dosbox-bin", str(emu_bin),
        "--working-dir", str(root),
        "--base-url", BASE_URL,
        "--cpu-cycles", str(CPU_CYCLES),
        # Confine mount_drive host paths to the plugin's drives/ subdir; the server
        # resolves .. and symlinks before the containment check.
        "--mount-root", str(drives_root()),
    ]
    try:
        proc = subprocess.run(
            cmd, input="Y\n", capture_output=True, text=True, timeout=180
        )
    except Exception as exc:
        return f"mcp registration failed — could not run `hermes mcp add`: {exc}"
    if proc.returncode != 0:
        return ("mcp registration failed:\n"
                f"stdout: {proc.stdout.strip()[-1200:]}\n"
                f"stderr: {proc.stderr.strip()[-1200:]}")
    check = subprocess.run(
        ["hermes", "config", "get", f"mcp_servers.{PLUGIN_NAME}"],
        capture_output=True, text=True, timeout=30,
    )
    if check.returncode != 0 or not check.stdout.strip():
        return ("warning: `hermes mcp add` exited 0 but the config entry could not "
                "be read back — verify with `hermes mcp list`.")
    return ("mcp entry verified by readback: mcp_servers.dosbox-full-auto "
            f"(command={mcp_bin}, cwd={root}, {BASE_URL})")


# -- status ------------------------------------------------------------------

def _mcp_entry_present() -> bool:
    try:
        proc = subprocess.run(
            ["hermes", "config", "get", f"mcp_servers.{PLUGIN_NAME}"],
            capture_output=True, text=True, timeout=30,
        )
        return proc.returncode == 0 and bool(proc.stdout.strip())
    except Exception:
        return False


def _vllm_key() -> str:
    # Rule 11: secrets only via the Hermes secret scope. No bare os.environ
    # fallback — it can pick up another profile's key under multiplexing.
    try:
        from agent.secret_scope import get_secret  # Hermes secret scope

        return get_secret("VLLM_API_KEY") or ""
    except Exception:
        return ""


def _vllm_status() -> str:
    """Lazy verify with a 24 h cooldown in plugin_db(). Never prints the key
    (Hermes masks key-like strings in tool output anyway)."""
    try:
        db = _db()
        db.execute(
            "CREATE TABLE IF NOT EXISTS vllm_state "
            "(id INTEGER PRIMARY KEY, last_verify REAL, note TEXT)"
        )
        row = db.execute(
            "SELECT last_verify, note FROM vllm_state WHERE id = 1"
        ).fetchone()
        now = time.time()
        if row and row[0] and (now - row[0]) < VLLM_COOLDOWN_S:
            return f"VLLM key: cached status ({row[1]}), cooldown active"
        key = _vllm_key()
        if not key:
            return (
                "VLLM key: VLLM_API_KEY not set — set it via `hermes config set VLLM_API_KEY <key>` "
                "or your profile .env (or mint one with /dosbox-full-auto create-key — it shows "
                "the terms of use before minting)"
            )
        req = urllib.request.Request(
            VLLM_BASE + "/api/keys/self",
            headers={"Authorization": f"Bearer {key}", "User-Agent": PLUGIN_NAME},
        )
        try:
            with urllib.request.urlopen(req, timeout=30) as res:
                body = json.load(res)
            note = f"active (status={body.get('status')})"
        except Exception as exc:
            note = f"verify failed: {exc}"
        db.execute(
            "INSERT OR REPLACE INTO vllm_state (id, last_verify, note) "
            "VALUES (1, ?, ?)",
            (now, note),
        )
        db.commit()
        return f"VLLM key: {note}"
    except Exception as exc:
        return f"VLLM key: status unavailable ({exc})"


def status() -> str:
    # Reports key state only — NEVER mints a key. Creation is explicit via
    # create-key (which shows the terms URL first); status must not accept
    # terms on the user's behalf.
    root = bundle_dir()
    lines = []
    if root.is_dir():
        missing = _verify_bundle(root)
        version = ""
        try:
            version = (root / VERSION_FILE).read_text(encoding="utf-8").strip()
        except OSError:
            pass
        lines.append(f"bundle: present at {root}" + (f" ({version})" if version else ""))
        lines.append("bundle check: " + ("complete" if not missing else "; ".join(missing)))
    else:
        lines.append(f"bundle: NOT installed (expected at {root})")
    lines.append(f"mcp server 'dosbox-full-auto': { 'registered' if _mcp_entry_present() else 'not registered' }")
    lines.append(_vllm_status())
    if not root.is_dir() or not _mcp_entry_present():
        lines.append("next: run /dosbox-full-auto install (downloads the release bundle and registers the MCP server)")
    return "\n".join(lines)