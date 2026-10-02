<!-- trace:v1 id=doc.adr-0004 type=document work=WORK-SI-MMMJA4G6 -->
# ADR 0004: Owned engine handle, handle-based C ABI, and plugin precision/resolver evidence extension points

<!-- trace:exempt reason=document-structure -->
## Context

The universal-engine migration (spec §§5/11) needed three things the
borrowed `Engine<'a>` plus path-based FFI could not give: (1) an owned
in-process handle so Rust programs embed SCC without spawning `scc`
(spec §123 item 1); (2) a genuine handle-based C ABI
(`scc_engine_open`/`invoke`/`close`) rather than path-per-call functions;
(3) third-party precision and evidence contributions (spec §31
SemanticResolver/EvidenceProvider) without forking the resolver or the
importer table.

<!-- trace:exempt reason=document-structure -->
## Decision

- `scc_engine::facade::SccEngine`: owns store + config, exposes
  `open`/`invoke`/`session`/`invoke_session`/`refresh` plus the spec §5
  usage namespaces (`context_atlas`, `surface_build`, `ranking_symbols`,
  `context_task`). Borrowed typed namespaces stay reachable via
  `with_engine`; no math moved — every method delegates to the same
  namespace modules `invoke()` dispatches to.
- `scc_engine_open`/`scc_engine_invoke`/`scc_engine_close`: opaque
  handle over the facade; JSON is the ABI boundary; NULL inputs yield
  error JSON, never UB. Path-based `scc_invoke_json` stays for one-shot
  callers. `scc_engine_invoke` is `unsafe` (raw handle deref, clippy
  `not_unsafe_ptr_arg_deref`).
- `resolver`-extension plugins answering `resolution.resolve`: run after
  pyright/tsserver in `resolve_repository`, same EXTRACTED→RESOLVED
  contract (confidence 0.99, semantic epoch bump), validate-then-commit
  per file. Invented endpoints fail the file without touching the model.
- `evidence-provider` plugins answering `evidence.import`: reachable as
  `import.<plugin-id>` through the same `import_evidence` entry and
  `ImportReport` shape as built-in formats, committed through
  validate+commit with evidence epoch bump and recompile.

<!-- trace:exempt reason=document-structure -->
## Alternatives considered

- Returning borrowed namespaces (`context()`/`ranking()`) from the owned
  handle: rejected — borrows outlive the temporary `open_engine` view
  (E0515). `with_engine` closure scoping keeps the borrow sound.
- `dylib` Rust plugins as the default: rejected per spec §23 (no stable
  Rust ABI). Process plugins stay the portable format; WASM stays a
  declared-but-unhosted runtime (loud `UnsupportedRuntime`).
- Copying Joern/CPG graphs into the Reality Graph: rejected per spec §3
  anti-goals. Sidecar storage (`sidecar.put/get/scan`) plus promotion
  through the normal contribution path is the only ingress.

<!-- trace:exempt reason=document-structure -->
## Consequences

- Rust embedding needs no subprocess; any FFI language opens one handle
  and invokes all 157+ registered operations (plugin ops included).
- Resolver/evidence plugins participate without source changes, under the
  same trust model (provenance-stamped, epoch-bumped, conflict-honest).
- `scc plugin new/test/pack` scaffolding stays DEFERRED per the
  capability ledger (no second plugin author yet); WASM host stays
  DEFERRED; Joern stays a future sidecar+overlay consumer, not a port.
