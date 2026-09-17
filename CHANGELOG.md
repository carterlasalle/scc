# Changelog

## [0.2.4] — 2026-09-17

Web viewer, SCC-native diagram, and Snapcompact bitmap export.

- **scc diagram.** Architecture diagram from the L1 layer (components,
  services, stores, deployment units) with capped architectural edges
  and flow subgraphs — as Mermaid (default) or dependency-free SVG.
  Deterministic: same index, same bytes.
- **scc view.** Local web viewer over the live index: overview,
  components, flows, diagram, search. Loopback only; the daemon serves
  the same pages as text/html alongside untouched /v1/* JSON.
- **scc snap.** Snapcompact-style bitmap export of the repo map
  (1568px canvas, 8x16 cells, 28px patch alignment, pinned Pillow
  recipe). Default OFF via context.snap_enabled — text packs remain
  the verified ceiling; the bitmap is an experiment hatch with a
  token-honesty line, not a migration.

## [0.2.3] — 2026-09-16

Root fixes for the bake-off gaps — no bandaids, no weakened contracts.

- **Impact answers file truth first.** Per-file importer BFS (depth-graded,
  provenance-tagged) over stored `imports` edges is the primary signal,
  rendered as AFFECTED FILES; components/flows interpret the closure.
- **Flow matching by exact step identity.** The quadratic substring matrix
  (and its 20s time budget) is gone — linear, terminates by construction.
- **One commit per index phase under FULL fsync.** Speed from batching
  commits, not from weakening durability; per-file atomicity via savepoints.
- **Generic mega-component cap.** Undeclared clusters over 64 files split
  recursively by directory; intent/service/package evidence always protects.
- **Lexical fallback on empty FTS search.** Same tables, no new index.

## [0.2.2] — 2026-09-16

Bake-off gap closure: index throughput, impact trust, and self-hosting hygiene.

- **Index writes 20x faster.** One transaction per file (savepoint-nested
  store internals) plus `synchronous=NORMAL` under WAL; scanner prunes
  config-ignored descents. Cold full-repo index: 609 files in ~12s.
- **Mid-flight corruption self-heals.** Quarantine-and-rebuild now triggers
  on any write failure, not just open; mid-flight retry added.
- **Ghost impact targets refuse.** `scc impact` on unknown files errors
  instead of fabricating RISK:MEDIUM; cochange-backed queries on unindexed
  repos still answer forgotten partners.
- **Impact budget.** 20s hard deadline with scored partial answers and
  truncation notes; flow dedup fixed (quadratic `contains`).
- **Self-ignore keeps intent committable.** `.scc/*` plus
  `!.scc/intent.yaml`, migrating bare `.scc/` lines that silently hid
  declared components and flows from the walker.

## [0.1.0] — 2026-08-31

First public release of the System Context Compiler (SCC).

### What it does

SCC compiles a repository into a structured System Atlas — a layered knowledge
graph of components, flows, contracts, and entrypoints — so coding agents start
with real architectural context instead of guessing.

### Highlights

- **Full extraction pipeline.** Tree-sitter parsing for Rust, Go, Java,
  TypeScript, Python, and Kotlin; Rust and Go AST extractors produce entities,
  relationships, imports, and symbols in a single pass.

- **Canonical causal flow graphs.** `FlowGraph` preserves branches, retries,
  joins, and fan-out — the behavioral truth from which surface projections are
  derived.

- **Four-level context stack.** Startup context (architectural overview),
  task context (relevant slices), surface context (API signatures and
  contracts), and runtime context (latency/error aggregates) — each with
  its own budget and ranker.

- **Semantic resolution.** Heuristics nominate candidate symbols, then LSP-backed
  engines resolve cross-file references, conflicts, and ambiguous callsites.

- **External benchmark suite.** 20-repo corpus with independently authored ground
  truth, pinned aider/repomix adapters, A–H harness variants, and automated
  recall/precision measurement against a 0.9 recall gate.

- **Context benchmark gate.** `scc bench context --min-recall 0.9` runs in CI —
  context packs must surface ≥ 90% of ground-truth keys to merge.

- **Model epoch invalidation.** Any change to source, semantic, evidence, intent,
  or runtime generations recomputes the cache epoch, so stale context packs are
  never served.

### SDKs

- **Python SDK** (`scc_sdk.py`). Token-optimized CLI proxy that strips up to 90%
  of bash noise. Supports `pip install` and direct script inclusion.

- **TypeScript SDK** (`scc-sdk`). Pack builder for `scc context --format json`
  output, with typed interfaces and unit tests.

### Tooling

- **CLI.** `scc` binary with subcommands: `context`, `atlas`, `bench` (atlas +
  context + agent), `index`, `info`, `doctor`.

- **MCP server.** Model Context Protocol adapter exposing SCC context as a
  tool for Claude, Codex, and other agents.

- **Hermes plugin.** Native tool registration for the Hermes agent framework,
  with auto-installer and contract test.

### Infrastructure

- **TraceLayer integration.** Mandatory `trace:v1` markers on all behavioral
  boundaries; `trace verify --changed` gates every merge.

- **CI.** GitHub Actions workflow with Build, Clippy (deny warnings), Python SDK
  tests, TypeScript build, benchmark harness, cargo tests, trace policy gate,
  context benchmark gate, Hermes contract test, and SBOM generation.

- **Docker support.** `SCC_STATE_DIR` for writable state outside read-only
  repos; SBOM artifact uploaded per run.

- **OMP integration.** Native extension registers SCC lifecycle with Oh My Pi
  harness; idle YAML hook marked inert.

### Fixes in this release

- `chunks_exact` → `as_chunks` for Rust 1.98 clippy compliance (`lib.rs`,
  `benchatlas.rs`).
- Python SDK: `from __future__ import annotations` for py3.9-safe PEP 604
  unions; typing modernization (`Dict` → `dict`, `Optional[X]` → `X | None`).
- Fixture repairs: `large-ts` test stub (private `db` prop), `go-facts-service`
  `go.sum` regeneration, `java-service` local `@Retryable`/`@Backoff`
  annotations.
- Benchmark scripts: shellcheck warnings resolved (`verify_gt.sh` SC2034/SC2155,
  `run_agent_bench.sh` scoped SC2016 disable).
- Trace obligations: all 102 `lib.rs` symbols and 106 `benchatlas.rs` symbols
  accounted; `get_embedding` exempt marker added.
- CI: `tracelayer` pip install step added to workflow (was missing from runner).
