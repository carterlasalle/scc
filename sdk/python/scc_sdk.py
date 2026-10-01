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
        recordVisibility: bool | None = None,
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
        req: dict[str, Any] = {
            "goal": goal,
            "files": files or [],
            "symbols": symbols or [],
            "budget": tokenBudget,
            "hook": False,
        }
        # Omit when unset: the engine field is non-nullable bool with a
        # server-side default; an explicit null fails deserialization.
        if recordVisibility is not None:
            req["record_visibility"] = recordVisibility
        return self.invoke("context.task", req)

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

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.ranking-symbols work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def ranking(
        self,
        goal: str | None = None,
        limit: int = 50,
        explain: bool = False,
        profile: str | None = None,
    ) -> dict[str, Any]:
        """Rank symbols for a goal through the full blend (task/global PPR,
        lexical/semantic, confidence, criticality, change risk, novelty,
        plus any active plugin features). Returns the RankResult envelope
        verbatim: ``{"items": [{"id", "rank", "position", "features",
        "specificity", "reasons", "plugin_features"}], "omitted_ids",
        "warnings"}``."""
        return self.invoke("ranking.symbols", {
            "goal": goal,
            "limit": limit,
            "explain": explain,
            "profile": profile,
        })

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.ranking-explain work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def explainRanking(
        self,
        id: str,
        goal: str | None = None,
    ) -> dict[str, Any]:
        """Explain one ranked symbol (§19/§96): the RankItem audit —
        blended score, position, 8-feature decomposition, specificity,
        reasons, and plugin_features. Same item shape as ranking() items."""
        return self.invoke("ranking.explain", {"id": id, "goal": goal})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.ranking-seeds work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def rankingSeeds(
        self,
        goal: str | None = None,
    ) -> dict[str, Any]:
        """Task-seed introspection (§123.11): lexical seeds merged with
        plugin seed providers (weight sums by id) — the same merge
        ranking.symbols consumes. Read-only stage view."""
        return self.invoke("ranking.seeds", {"goal": goal})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.ranking-graph work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def rankGraph(self) -> dict[str, Any]:
        """Rank-universe stage views (§123.8/12): universe nodes, rank
        edges (pre-aggregation weights), and the reference graph — the
        structure the PPR vectors diffuse over. Three read-only invokes."""
        return {
            "universe": self.invoke("ranking.universe", {}),
            "edges": self.invoke("ranking.edges", {}),
            "reference_graph": self.invoke("ranking.reference_graph", {}),
        }

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.ranking-features work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def rankingFeatures(
        self,
        goal: str | None = None,
        limit: int = 50,
        explain: bool = False,
        profile: str | None = None,
    ) -> dict[str, Any]:
        """Feature-score introspection (§123.13): per-symbol core + plugin
        feature decomposition before the blend — the same computation as
        ranking.symbols, projected to feature rows."""
        return self.invoke("ranking.features", {
            "goal": goal,
            "limit": limit,
            "explain": explain,
            "profile": profile,
        })

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.ranking-trace work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def rankingTrace(
        self,
        goal: str | None = None,
        limit: int = 50,
        explain: bool = False,
        profile: str | None = None,
    ) -> dict[str, Any]:
        """Full ranking trace (§19): RankResult items plus the seed and
        required inputs the blend consumed. Same computation as
        ranking.symbols; the envelope is the audit path."""
        return self.invoke("ranking.trace", {
            "goal": goal,
            "limit": limit,
            "explain": explain,
            "profile": profile,
        })

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.pagerank-vectors work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def pagerankVectors(
        self,
        goal: str | None = None,
    ) -> dict[str, Any]:
        """Raw PPR stage vectors (§123.10): the global vector and the
        task vector for a goal. No plugin hooks by contract — the
        structure the symbol projection blends from."""
        return {
            "global": self.invoke("ranking.pagerank.global", {}),
            "task": self.invoke("ranking.pagerank.task", {"goal": goal}),
        }

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.ranking-candidates work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def rankingCandidates(
        self,
        goal: str | None = None,
        limit: int = 50,
    ) -> dict[str, Any]:
        """Candidate-stage introspection: scored entities (id/kind/name/
        score/reason) merged from the core Surface compiler plus plugin
        candidate providers — the same merge ranking.symbols consumes."""
        return self.invoke("ranking.candidates", {"goal": goal, "limit": limit})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.selection-preview work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def selectionPreview(
        self,
        ranked: list[dict[str, Any]],
        budget: int = 0,
        quotas: list[dict[str, Any]] | None = None,
        lam: float | None = None,
    ) -> dict[str, Any]:
        """Selection-effects introspection (§123.14): per-stage survivors
        (after_mmr, after_quotas, after_budget) over caller-supplied rows
        ``[{id, value, token_cost, kind, group}]``. Default math only."""
        req: dict[str, Any] = {"ranked": ranked, "budget": budget}
        if quotas is not None:
            req["quotas"] = quotas
        if lam is not None:
            req["lambda"] = lam
        return self.invoke("selection.preview", req)

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.selection-required work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def selectionRequired(
        self,
        goal: str | None = None,
    ) -> dict[str, Any]:
        """Required-coverage set (§123 never-omit): engine required_ids
        plus plugin coverage providers, unioned. Same inputs
        ranking.symbols blends criticality from."""
        return self.invoke("selection.required", {"goal": goal})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.project-symbols work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def projectSymbols(
        self,
        vector: list[dict[str, Any]] | None = None,
        source: str | None = None,
        goal: str | None = None,
    ) -> dict[str, Any]:
        """Projection introspection (§123 intermediate): map a universe
        vector to per-symbol scores. Pass explicit ``vector`` rows
        ``[{id, score}]``, or ``source="global"``, or a task ``goal``
        (task vector). Same projection ranking.symbols blends from."""
        return self.invoke("ranking.project_symbols", {
            "vector": vector,
            "source": source,
            "goal": goal,
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

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.traverse work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def traverse(self, kind=None, name=None, from_ids=None, steps=None, limit=100, trusted_only=True):
        """Multi-step graph traversal (§16). Steps: dicts with
        dir (out|in|both), optional predicate, optional where_kind."""
        return self.invoke("graph.traverse", {
            "kind": kind, "name": name, "from_ids": from_ids or [],
            "steps": steps or [], "limit": limit, "trusted_only": trusted_only,
        })

    def query(self, kind=None, name=None):
        """Start a fluent traversal: SCC(store).query(kind, name).out(...)."""
        return _Query(self, kind=kind, name=name)

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.explain work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def explain(self, subject, predicate, object):
        """Overlay diagnostics: every assertion behind one edge plus verdict."""
        return self.invoke("graph.explain", {"subject": subject, "predicate": predicate, "object": object})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.schema work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def operation_schema(self, op_id):
        """JSON Schema for one operation's input (from request types)."""
        return self.invoke("operations.schema", {"id": op_id})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.capabilities work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def capabilities(self):
        """Capability vocabulary: permissions, extension points, versions."""
        return self.invoke("operations.capabilities", {})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.plugin-graph work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def plugin_graph(self):
        """Deterministic extension order per type."""
        return self.invoke("plugins.graph", {})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.sidecar-put work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def sidecar_put(self, plugin, graph, key, value):
        """Write one raw sidecar fact (never authoritative; promote explicitly)."""
        return self.invoke("sidecar.put", {"plugin": plugin, "graph": graph, "key": key, "value": value})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.sidecar-get work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def sidecar_get(self, plugin, graph, key):
        """Read one raw sidecar fact."""
        return self.invoke("sidecar.get", {"plugin": plugin, "graph": graph, "key": key})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.sidecar-scan work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def sidecar_scan(self, plugin, graph, prefix="", limit=100):
        """Scan raw sidecar facts by prefix within one plugin graph."""
        return self.invoke("sidecar.scan", {"plugin": plugin, "graph": graph, "prefix": prefix, "limit": limit})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.promote work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def promote(self, plugin, assertions):
        """Promote selected sidecar findings to canonical relationships."""
        return self.invoke("plugins.promote", {"plugin": plugin, "assertions": assertions})

    # trace:v1 id=impl.sdk-python-scc-sdk-scc.viewer-panels work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    def viewer_panels(self):
        """Plugin viewer data panels (structured title/html + provenance)."""
        return self.invoke("viewer.panels", {})


# trace:v1 id=impl.sdk-python-scc-sdk.query-builder work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
class _Query:
    """Fluent builder for graph.traverse: collects steps, executes on demand."""

    def __init__(self, scc, kind=None, name=None):
        self._scc = scc
        self._kind = kind
        self._name = name
        self._steps = []

    def out(self, predicate=None, where_kind=None, limit=0):
        self._steps.append({"dir": "out", "predicate": predicate, "where_kind": where_kind, "limit": limit})
        return self

    def in_(self, predicate=None, where_kind=None, limit=0):
        self._steps.append({"dir": "in", "predicate": predicate, "where_kind": where_kind, "limit": limit})
        return self

    def both(self, predicate=None, where_kind=None, limit=0):
        self._steps.append({"dir": "both", "predicate": predicate, "where_kind": where_kind, "limit": limit})
        return self

    def execute(self, limit=100):
        return self._scc.traverse(kind=self._kind, name=self._name, steps=self._steps, limit=limit)
