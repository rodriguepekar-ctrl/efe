"""Persistent history for the EFE REPL.

Convention resolution order (XDG Base Directory Specification):

    1. explicit ``--history PATH`` at boot (stored in SessionState indirectly)
    2. ``$EFE_HISTORY_PATH`` env var
    3. ``$XDG_DATA_HOME/efe/history``
    4. ``~/.local/share/efe/history``
    5. ``~/.efe/history``

The file format is one input line per row, UTF-8 strict, no header,
no timestamps (to remain ``fc``-compatible). Rotation is FIFO at
:data:`HISTORY_CAP` lines.

The class reads on construction and writes on ``flush()``. It does
*not* tie into ``readline.read_history_file`` directly — the caller
wires ``readline`` explicitly because the input loop may be in batch
mode (non-TTY) where readline is irrelevant.
"""

from __future__ import annotations

import os
import stat
import sys
from pathlib import Path
from typing import Iterable, Optional


HISTORY_CAP: int = 10_000


# ---------------------------------------------------------------------------
#  Path resolution
# ---------------------------------------------------------------------------


def resolve_history_path(
    explicit: Optional[str] = None,
    *,
    env: Optional[dict[str, str]] = None,
    home: Optional[Path] = None,
) -> Path:
    """Pick the canonical history file path.

    Args:
        explicit: value of ``--history`` (takes precedence). Empty
                  string / None means "no explicit path".
        env:  environment mapping; defaults to :data:`os.environ`.
        home: override for tests.
    """
    envm = env if env is not None else os.environ
    if explicit:
        return Path(explicit).expanduser()
    if envm.get("EFE_HISTORY_PATH"):
        return Path(envm["EFE_HISTORY_PATH"]).expanduser()
    xdg_data = envm.get("XDG_DATA_HOME")
    if xdg_data:
        return Path(xdg_data).expanduser() / "efe" / "history"
    h = home or Path.home()
    default_xdg = h / ".local" / "share" / "efe" / "history"
    legacy = h / ".efe" / "history"
    # Prefer XDG layout if it already exists; otherwise if the legacy
    # layout exists, keep using it (don't migrate silently); otherwise
    # create XDG.
    if default_xdg.exists():
        return default_xdg
    if legacy.exists():
        return legacy
    return default_xdg


# ---------------------------------------------------------------------------
#  History class
# ---------------------------------------------------------------------------


class History:
    """In-memory list of lines with persistence.

    Parameters
    ----------
    path:
        File path. When ``None`` or when ``enabled=False``, the instance
        acts as a null sink (nothing read, nothing written).
    enabled:
        ``False`` disables all persistence (maps to ``--no-history``).
    cap:
        Maximum number of entries kept in memory and on disk.
    """

    def __init__(
        self,
        path: Optional[Path],
        *,
        enabled: bool = True,
        cap: int = HISTORY_CAP,
    ) -> None:
        self.path = Path(path) if path else None
        self.enabled = enabled and (self.path is not None)
        self.cap = cap
        self._lines: list[str] = []

    # ------------------------------------------------------------------
    #  Load / flush
    # ------------------------------------------------------------------

    def load(self) -> None:
        """Read the file into memory. Tolerates a missing or unreadable file."""
        if not self.enabled or self.path is None or not self.path.exists():
            return
        try:
            with self.path.open("r", encoding="utf-8", errors="replace") as f:
                self._lines = [
                    ln.rstrip("\n\r") for ln in f if ln.strip()
                ]
        except OSError:
            # Corrupt or unreadable file: start fresh rather than crash.
            self._lines = []
        self._cap()

    def flush(self) -> bool:
        """Write the in-memory list atomically to disk.

        Returns ``True`` on success, ``False`` on any I/O failure
        (caller may log).
        """
        if not self.enabled or self.path is None:
            return False
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            tmp = self.path.with_suffix(self.path.suffix + ".tmp")
            with tmp.open("w", encoding="utf-8") as f:
                f.write("\n".join(self._lines[-self.cap:]))
                if self._lines:
                    f.write("\n")
            tmp.replace(self.path)
            # Best-effort chmod 0600.
            try:
                self.path.chmod(stat.S_IRUSR | stat.S_IWUSR)
            except OSError:  # pragma: no cover
                pass
            return True
        except OSError:
            return False

    # ------------------------------------------------------------------
    #  Public API
    # ------------------------------------------------------------------

    def append(self, line: str) -> None:
        """Record a line (if enabled)."""
        if not self.enabled:
            return
        line = line.rstrip("\n\r")
        if not line:
            return
        # Skip duplicate of the most recent line (same convention as bash).
        if self._lines and self._lines[-1] == line:
            return
        self._lines.append(line)
        self._cap()

    def recent(self, n: int = 20) -> list[str]:
        return list(self._lines[-n:])

    def clear(self) -> None:
        self._lines.clear()
        if self.enabled and self.path is not None and self.path.exists():
            try:
                self.path.unlink()
            except OSError:  # pragma: no cover
                pass

    def __len__(self) -> int:
        return len(self._lines)

    def __iter__(self) -> Iterable[str]:
        return iter(self._lines)

    # ------------------------------------------------------------------
    #  Readline integration
    # ------------------------------------------------------------------

    def populate_readline(self) -> None:
        """Push each stored line into the current readline history.

        This lets the user navigate with up/down arrows through previous
        sessions' inputs. It is a no-op on platforms where ``readline``
        is not available (Windows without pyreadline3).
        """
        if not self.enabled:
            return
        try:
            import readline  # type: ignore
        except ImportError:  # pragma: no cover
            return
        try:
            readline.set_history_length(self.cap)
            readline.clear_history()
            for line in self._lines:
                readline.add_history(line)
        except Exception:  # pragma: no cover
            # Any readline glitch is non-fatal.
            pass

    def sync_from_readline(self) -> None:
        """Copy the current ``readline`` history into this object's list.

        Called right before :meth:`flush` so lines edited in the TTY are
        persisted. When ``readline`` holds nothing (typically because
        we ran in batch mode where the loop used ``stream.readline()``
        directly), we keep the in-memory list intact.
        """
        if not self.enabled:
            return
        try:
            import readline  # type: ignore
        except ImportError:  # pragma: no cover
            return
        try:
            n = readline.get_current_history_length()
            if n <= 0:
                return  # nothing to sync; preserve our in-memory list
            lines = [readline.get_history_item(i) for i in range(1, n + 1)]
            synced = [ln for ln in lines if ln]
            if synced:
                self._lines = synced
                self._cap()
        except Exception:  # pragma: no cover
            pass

    # ------------------------------------------------------------------
    #  Internals
    # ------------------------------------------------------------------

    def _cap(self) -> None:
        over = len(self._lines) - self.cap
        if over > 0:
            del self._lines[:over]


__all__ = ["History", "HISTORY_CAP", "resolve_history_path"]
