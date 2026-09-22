"""Structured Python SDK for the SCC engine (``scc rpc --stdio``).

One persistent ``scc rpc`` child per :class:`SCC` instance; every method
speaks the operation registry (no CLI-text scraping). The binary is resolved
from the ``bin`` constructor argument, then the ``SCC_BIN`` environment
variable, then ``scc`` on PATH. Transport errors raise :class:`SCCError`.

``invoke(operation, input)`` reaches every registered operation — including
plugin operations — without a typed wrapper.
"""

from __future__ import annotations

import itertools
import json
import os
import subprocess
import threading
from typing import Any

# trace:v1 id=impl.scc.sdk.python work=WORK-SCC-014 satisfies=REQ-SCC-IR


class SCCError(Exception):
    """Raised when the ``scc`` CLI exits with a non-zero status."""


# trace:v1 id=impl.sdk-python-scc-sdk.scc work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
class SCC:
    """Client for the ``scc`` CLI (thin subprocess wrapper)."""

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.init work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def __init__(self, bin: str | None = None, cwd: str | None = None) -> None:
        self._bin = bin or os.environ.get("SCC_BIN") or "scc"
        self._cwd = cwd or os.getcwd()
        self._ids = itertools.count(1)
        self._lock = threading.Lock()
        self._proc: subprocess.Popen | None = None

    # trace:exempt reason=internal-detail
    def _rpc(self) -> subprocess.Popen:
        """Lazily spawn the persistent ``scc rpc --stdio`` child."""
        if self._proc is None:
            try:
                self._proc = subprocess.Popen(
                    [self._bin, "rpc", "--stdio"],
                    cwd=self._cwd,
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                    bufsize=1,
                )
            except OSError as e:
                raise SCCError(f"failed to spawn {self._bin}: {e}")
        assert self._proc is not None
        return self._proc

    # trace:exempt reason=internal-detail
    def close(self) -> None:
        """Terminate the RPC child (also runs via :meth:`__del__`)."""
        proc, self._proc = self._proc, None
        if proc is not None:
            try:
                proc.terminate()
            except OSError:
                pass

    # trace:exempt reason=internal-detail
    def __del__(self) -> None:  # pragma: no cover - GC timing
        try:
            self.close()
        except Exception:
            pass

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.invoke work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def invoke(self, operation: str, input: dict[str, Any] | None = None) -> Any:
        """Call any registered engine operation (including plugin ops).

        Returns the operation's structured ``output`` verbatim — real token
        counts, real ids, never synthesized placeholders.
        """
        proc = self._rpc()
        assert proc.stdin is not None and proc.stdout is not None
        rid = next(self._ids)
        frame = json.dumps({"id": rid, "operation": operation, "input": input or {}})
        with self._lock:
            try:
                proc.stdin.write(frame + "\n")
                proc.stdin.flush()
            except (OSError, ValueError) as e:
                raise SCCError(f"{self._bin} rpc write failed: {e}")
            line = proc.stdout.readline()
        if not line:
            err = (proc.stderr.read() if proc.stderr else "") or f"{self._bin} rpc exited"
            raise SCCError(err.strip())
        try:
            msg = json.loads(line)
        except json.JSONDecodeError as e:
            raise SCCError(f"{self._bin} rpc invalid JSON: {e}")
        if msg.get("id") != rid:
            raise SCCError(f"{self._bin} rpc id mismatch: {line.strip()[:200]}")
        if "error" in msg:
            raise SCCError(str(msg["error"]))
        return msg.get("output")

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.system-overview work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def systemOverview(self) -> dict[str, Any]:
        """Compile the system overview capsule."""
        return self.invoke("context.overview", {})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.task-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def taskContext(
        self,
        goal: str,
        files: list[str] | None = None,
        symbols: list[str] | None = None,
        tokenBudget: int | None = None,
    ) -> dict[str, Any]:
        """Compile the complete task context artifact for a goal: the enriched
        task pack plus its task-personalized Surface delta.

        Returns the CLI's `scc context task --json` output verbatim:
        ``{"pack": {...}, "delta": "...", "delta_ids": [...],
        "token_count": N}`` — ``pack`` is the flat task pack (keys
        ``kind``, ``content``, ``entity_ids``, ...); ``delta`` is the
        task-personalized Surface delta; ``delta_ids`` are the delta's
        rendered entry ids. Never flattened: consumers read
        ``result["pack"]["content"]``, not ``result["content"]``.
        """
        return self.invoke("context.task", {
            "goal": goal,
            "files": files or [],
            "symbols": symbols or [],
            "budget": tokenBudget,
            "hook": False,
        })

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.component-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def componentContext(self, id: str) -> dict[str, Any]:
        """Compile the context pack for one component (by id or name)."""
        return self.invoke("context.component", {"id": id})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.flow-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def flowContext(self, id: str) -> dict[str, Any]:
        """Compile the context pack for one flow (by id or name)."""
        return self.invoke("context.flow", {"id": id})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.impact-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def impactContext(
        self, files: list[str] | None = None, symbols: list[str] | None = None
    ) -> dict[str, Any]:
        """Compile an impact analysis pack for a set of files/symbols."""
        return self.invoke("context.impact", {
            "files": files or [],
            "symbols": symbols or [],
        })

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.verify-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def verifyContext(self) -> dict[str, Any]:
        """Run the freshness/evidence verification (structured pack)."""
        return self.invoke("context.verify", {})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.context-startup work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def contextStartup(self, budget: int | None = None) -> dict[str, Any]:
        """Compile the fused session-startup artifact (text + budget + artifact)."""
        out = self.invoke("context.startup", {"budget": budget})
        return {"kind": "startup", "content": out.get("text", ""), "budget": out.get("budget", budget or 0), "artifact": out.get("artifact")}

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.surface-map work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def surfaceMap(
        self, goal: str | None = None, budget: int | None = None
    ) -> dict[str, Any]:
        """Compile the System Surface Map (structured result + text, verbatim)."""
        return self.invoke("surface.build", {
            "task": goal,
            "budget": budget,
            "explain": False,
        })

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.structural-source work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def structuralSource(
        self,
        files: list[str] | None = None,
        goal: str | None = None,
        budget: int | None = None,
    ) -> str:
        """Compile the Structural Source representation (rendered text, verbatim)."""
        out = self.invoke("context.structural", {
            "files": files or [],
            "task": goal,
            "budget": budget,
        })
        # Engine returns {"text": ...}; unwrap to the rendered string.
        if isinstance(out, dict):
            return out.get("text", "")
        return out

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.index work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def index(self) -> dict[str, bool]:
        """Index the repository (idempotent; incremental after the first run)."""
        self.invoke("index.full", {})
        return {"ok": True}

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.operations work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
    def operations(self) -> Any:
        """List registered engine operations (introspection, via RPC)."""
        return self.invoke("operations.list", {})
