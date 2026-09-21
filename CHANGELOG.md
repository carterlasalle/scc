# Changelog

## [0.2.6] — 2026-09-21

Samply/flamegraph + temporary spans, all semantics preserved.

- **`SurfaceRenderResult.rendered_entries`.** The render pipeline already
  held the rendered entries; callers recompiled the whole surface map to
  resolve them (the startup path already avoided this). Both recompile
  sites now use the attached entries. `ledger_record` 65ms → 1ms.
  Wire-compatible (`default` + `skip_serializing_if empty`).
- **Gated derived recompute on no-change index.** Mentions, RPC bridges,
  and BM25 stats are pure functions of hash-identical inputs — skipped
  when nothing was processed and nothing removed (removals included: a
  purge changes what other files' mentions resolve to).
  `indexer.index` 1070ms → ~220ms; no-change index ~1.1s → ~0.2s warm.
- **Raw-column incremental content hash.** `graph_content_hash` parsed
  every row's JSON and re-serialized 26k structs just to hash bytes; now
  feeds raw columns into incremental FNV in query order. Same dedup
  contract (opaque equality vs head) — one digest discontinuity, one
  extra revision on first record after upgrade, then dedup resumes.

Receipts (release, M1 Pro, self repo, 5-run medians): index 0.22s,
surface --task 0.52s, context task 0.54s, startup 0.48s, surface 0.44s,
important 0.27s, status 0.06s, atlas 0.17s. Full suite 980/980.
Stop: remainder is SQLite row fetch + filesystem walk/hash (real work).
Parked: `stale_paths` daemon dirty-set; cold-query page warmth.
## [0.2.5] — 2026-09-21

Speed release: every read path profiled with Samply/flamegraph, all
features and output preserved (byte-identical modulo artifact hashes).

Measured on system_ir (649 files, 8k entities, 18k rels), release binary:

- **No-change `scc index`: 6.5s → 1.1s.** Skips the derived recompile
  when zero files changed and the extractor is current (the revision
  record still runs, preserving the epoch/ledger contract). Glob
  matchers compile once per scan instead of per file+dir; doc-mention
  matching precomputes span suffixes and entity keys once per run
  instead of per (entity x span) pair.
- **`scc context startup`: 7.0s → 0.4s.** Flow matching replaced a
  ~10M-substring-search scan per map with a per-map actor→flows index
  plus a per-file fallback cache (parity-proven over 4236 symbols).
- **`scc surface --task`: 5.3s → 0.5s.** Same flow index; the surface
  pipeline was the dominant cost.
- **`scc context task`: 4.0s → 0.5s.** Same flow index via the
  task-delta path.
- **`scc important`: 1.7s → 0.2s.** Deleted a full `build_surface_staged`
  whose result was discarded (a second SystemRanker build).
- **`scc query`: unchanged at 0.8s cold.** Measured as cold page-cache
  fetch on the 70MB FTS index (30ms warm), not a code hotspot — no code
  change beats that physics for a one-shot CLI; the daemon never pays
  it after warmup.

Deliberately not taken: skipping the snapshot+record on no-change
index (would keep epoch caches warm but breaks the reindex-resets-
ledger contract the task cache pins — a product semantic change, not
a perf fix).

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
