"""Keep a shipped sidecar's application bundle stable for its entire lifetime."""

import atexit
import json
import os
from pathlib import Path
import tempfile


def _lock(file):
    if os.name == "nt":
        import msvcrt
        file.seek(0)
        msvcrt.locking(file.fileno(), msvcrt.LK_NBLCK, 1)
    else:
        import fcntl
        fcntl.flock(file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)


def register(script):
    """Return a lifetime lock for portable bundles; source/MSI runs are unaffected."""
    root = Path(script).resolve().parent.parent
    manifest = root / "teshi-bundle.json"
    if not manifest.is_file():
        return None
    metadata = json.loads(manifest.read_text(encoding="utf-8"))
    if metadata.get("kind") != "portable":
        return None
    state = root / ".teshi-update"
    if state.is_symlink():
        raise RuntimeError("Linked update state is not allowed")
    state.mkdir(mode=0o700, exist_ok=True)
    with (state / "gate.lock").open("a+b") as gate:
        _lock(gate)
        if (state / "pending.json").exists():
            raise RuntimeError("An update needs recovery; start the Teshi CLI first")
        participants = state / "participants"
        participants.mkdir(mode=0o700, exist_ok=True)
        descriptor, name = tempfile.mkstemp(prefix="sidecar-", dir=participants)
        lock = os.fdopen(descriptor, "w+b")
        _lock(lock)

    def release():
        lock.close()
        Path(name).unlink(missing_ok=True)

    atexit.register(release)
    return lock
