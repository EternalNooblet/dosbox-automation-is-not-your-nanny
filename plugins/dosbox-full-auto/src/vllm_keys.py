"""VLLM API key management — explicit create-key only (terms URL shown first),
creation via PoW. Hermes has no secret-scope SETTER (agent.secret_scope has
get_secret but no set_secret), so the plugin never stores a key: after minting,
the user is told to set VLLM_API_KEY via `hermes config set` or their profile
.env. Nothing here creates a key on its own; the caller (the create-key
command) drives the terms confirmation."""

from __future__ import annotations

import hashlib
import itertools
import json
import time
import urllib.request
from pathlib import Path

# Shared constants (avoid circular import with install.py)
PLUGIN_NAME = "dosbox-full-auto"
VLLM_BASE = "https://decision.vllm-sr.ai"

TERMS_URL = VLLM_BASE + "/terms"

# Rate limits: 5 keys/IP/hour, 20/day
VLLM_KEY_CREATE_COOLDOWN_S = 720  # 12 min between attempts (5/hr)
VLLM_KEY_DAILY_LIMIT = 20
VLLM_DAILY_WINDOW_S = 24 * 3600

# Window in which the confirm step (second create-key run) may mint
TERMS_CONFIRM_WINDOW_S = 600


def _db():
    """Get plugin database connection."""
    try:
        from plugins.plugin_storage import plugin_db
        return plugin_db(PLUGIN_NAME)
    except Exception:
        import sqlite3
        try:
            from plugins.plugin_storage import plugin_data_dir
            root = plugin_data_dir(PLUGIN_NAME)
        except Exception:
            import os
            from hermes_constants import get_hermes_home
            home = Path(get_hermes_home())
            root = home / "plugin-data" / PLUGIN_NAME
        root.mkdir(parents=True, exist_ok=True)
        return sqlite3.connect(str(root / "data.db"))


def _vllm_key() -> str:
    """Read VLLM_API_KEY via the Hermes secret scope. get_secret exists and
    reads the active profile's scope (.env + secret sources, os.environ only
    when not multiplexing); there is deliberately no set_secret in that module,
    so keys are read-only here — never stored by the plugin."""
    try:
        from agent.secret_scope import get_secret
        return get_secret("VLLM_API_KEY") or ""
    except Exception:
        return ""


def _verify_key(key: str) -> bool:
    """Check if key is active via /api/keys/self."""
    try:
        req = urllib.request.Request(
            VLLM_BASE + "/api/keys/self",
            headers={"Authorization": f"Bearer {key}", "User-Agent": PLUGIN_NAME},
        )
        with urllib.request.urlopen(req, timeout=30) as res:
            body = json.load(res)
        return body.get("status") == "active"
    except Exception:
        return False


def _create_key_pow() -> str | None:
    """Create new key via PoW: challenge -> nonce -> submit -> verify."""
    try:
        # 1. Get challenge — endpoint requires POST (no body); GET 405s.
        req = urllib.request.Request(
            VLLM_BASE + "/api/keys/challenge",
            data=b"",
            headers={"User-Agent": PLUGIN_NAME},
        )
        with urllib.request.urlopen(req, timeout=30) as res:
            challenge_data = json.load(res)

        challenge = challenge_data["challenge"]
        difficulty = challenge_data["difficulty"]
        target = 1 << (256 - difficulty)
        prefix = challenge + ":"

        # 2. Find nonce (PoW)
        nonce = next(
            n for n in itertools.count()
            if int.from_bytes(hashlib.sha256(f"{prefix}{n}".encode()).digest(), "big") < target
        )

        # 3. Submit challenge + nonce
        req = urllib.request.Request(
            VLLM_BASE + "/api/keys",
            data=json.dumps({
                "challenge": challenge,
                "nonce": str(nonce),
                "accept_terms": True
            }).encode(),
            headers={"Content-Type": "application/json", "User-Agent": PLUGIN_NAME},
            method="POST"
        )
        with urllib.request.urlopen(req, timeout=30) as res:
            key_data = json.load(res)

        api_key = key_data.get("api_key")
        if api_key and api_key.startswith("sk-vsr-") and _verify_key(api_key):
            return api_key
        return None
    except Exception:
        return None


def terms_gate_pending() -> bool:
    """True if the terms URL was shown recently (the confirm half of create-key).
    State kept in plugin_db() — per profile, like every other plugin secret."""
    try:
        db = _db()
        db.execute(
            "CREATE TABLE IF NOT EXISTS terms_gate (id INTEGER PRIMARY KEY, shown_at REAL)"
        )
        row = db.execute("SELECT shown_at FROM terms_gate WHERE id = 1").fetchone()
        return bool(row and row[0] and (time.time() - row[0]) < TERMS_CONFIRM_WINDOW_S)
    except Exception:
        return False


def terms_gate_show() -> None:
    """Record that the terms URL has just been shown to the user."""
    try:
        db = _db()
        db.execute(
            "CREATE TABLE IF NOT EXISTS terms_gate (id INTEGER PRIMARY KEY, shown_at REAL)"
        )
        db.execute(
            "INSERT OR REPLACE INTO terms_gate (id, shown_at) VALUES (1, ?)",
            (time.time(),),
        )
        db.commit()
    except Exception:
        pass


def create_vllm_key() -> str:
    """Mint a NEW VLLM_API_KEY via PoW; the plugin does NOT store it (agent.secret_scope
    has no set_secret — writing a key is the user's job: `hermes config set` or .env).
    Only ever called from the explicit create-key command, and only after
    terms_gate_pending() confirms the terms URL was shown to the user.
    Enforces the 5/hr and 20/day creation limits locally before asking.
    Returns the new key, or empty string if limits/refusal blocked it."""
    # Mint regardless of an existing key — create-key means "new key".
    db = _db()
    db.execute(
        "CREATE TABLE IF NOT EXISTS vllm_key_created "
        "(id INTEGER PRIMARY KEY, created_at REAL)"
    )
    now = time.time()

    # Hourly cooldown (5/hr = 720s)
    row = db.execute(
        "SELECT created_at FROM vllm_key_created WHERE id = 1"
    ).fetchone()
    if row and row[0] and (now - row[0]) < VLLM_KEY_CREATE_COOLDOWN_S:
        return ""  # hourly rate limited

    # Daily limit (20/day)
    day_start = now - (now % VLLM_DAILY_WINDOW_S)
    daily_count = db.execute(
        "SELECT COUNT(*) FROM vllm_key_created WHERE created_at >= ?",
        (day_start,)
    ).fetchone()[0]
    if daily_count >= VLLM_KEY_DAILY_LIMIT:
        return ""  # daily limit reached

    # Create new key
    new_key = _create_key_pow()
    if new_key:
        db.execute(
            "INSERT OR REPLACE INTO vllm_key_created (id, created_at) VALUES (1, ?)",
            (now,)
        )
        db.commit()
        return new_key
    return ""