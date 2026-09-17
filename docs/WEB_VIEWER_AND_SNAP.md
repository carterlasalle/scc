# Web Viewer, Architecture Diagram, and Snap Bitmap Export
<!-- trace:v1 id=SPEC-SCC-VIEWER type=document work=WORK-SCC-VIEWER title="Web viewer, SCC-native diagram, and Snapcompact-style bitmap export" -->

## Goal

Give SCC a human-readable web surface and an LLM-efficient bitmap export,
both derived from the same System IR that already feeds agents — no new
model, no paid service, no network calls.

## Non-goals

- No hosted SaaS, no private-repo exfiltration, no GitHub API fetching
  (contrast with gitdiagram, which is cloud-LLM over public repos only).
- No new MCP tool, no new ranking model, no new extraction language.
- No replacing the text packs: bitmap export is opt-in and default-off.

<!-- trace:v1 id=SPEC-SCC-VIEWER-acceptance type=document work=WORK-SCC-VIEWER documents=SPEC-SCC-VIEWER -->
## Acceptance criteria

1. `scc diagram [--format mermaid|svg] [--out FILE]` prints an
   architecture diagram derived from components + relationships + flows.
2. `scc view [--port N] [--no-open]` serves a local web viewer (overview,
   components, flows, diagram, search) backed by the index; daemon gains
   the same routes.
3. `scc snap [--out FILE.png] [--max-chars N]` renders the repo map as a
   dense bitmap for vision-capable models; off by default, enabled by
   `context.snap_enabled` or the flag.
4. `cargo test -p scc-cli` passes; clippy clean; `trace verify --changed`
   passes.

<!-- trace:v1 id=SPEC-SCC-VIEWER-protected type=document work=WORK-SCC-VIEWER documents=SPEC-SCC-VIEWER -->
## Protected surface

Existing CLI output bytes, HTTP routes, config defaults, and the System IR
schema stay unchanged. New config keys default to current behavior.

<!-- trace:v1 id=SPEC-SCC-VIEWER-design type=document work=WORK-SCC-VIEWER documents=SPEC-SCC-VIEWER -->
## Design

### 1. `scc diagram` — SCC-native gitdiagram

gitdiagram's shape (paste URL → Mermaid diagram → click nodes) is right;
its substance (file tree + README + cloud LLM labels) is exactly what SCC
already beats locally with evidence. Our version derives the diagram from
the compiled model instead of guessing from filenames:

- Nodes: COMPONENT/SERVICE/DEPLOYMENT_UNIT/DATA_STORE entities (L1
  architectural layer, same filter as `export_ccg`).
- Edges: DEPENDS_ON / CALLS / DATA_FLOW relationships between those nodes,
  capped per node so the diagram stays readable.
- Flows: each named flow becomes a subgraph listing its participant
  symbols in order.
- Output: Mermaid `flowchart LR` (default, printable to stdout, embeddable
  like gitdiagram's) and a dependency-free SVG (nodes as boxes, edges as
  lines, no JS needed).

Determinism: entities sorted by name, edges sorted by (subject,
predicate, object), caps applied after sorting. Same index → same bytes.

### 2. `scc view` — local web viewer

`scc view` starts a loopback server (reuse `httpd.rs`, same
`SCC_ALLOW_REMOTE_LISTEN` gate) and opens the browser. Pages are
server-rendered HTML from live store reads — no framework, no build step:

- `/` — overview: repo id, revision, freshness, stats, component/flow
  counts, staleness warnings.
- `/components` — component list → per-component detail (responsibility,
  implementation, evidence, member symbols).
- `/flows` — flow list → per-flow steps with participant links.
- `/diagram` — the SVG diagram inline + the Mermaid source in a
  copyable block.
- `/search?q=` — lexical entity/symbol search (same fallback as
  `cmd_query`).

JSON stays on `/v1/*`; the viewer serves `text/html` routes alongside it.
`scc view --no-open` prints the URL without launching a browser (CI/SSH).

### 3. `scc snap` — Snapcompact-style bitmap export

The Stencil article's finding: dense pixel-font text bitmaps decode in
vision models at ~1/3 the input-token price with near-verbatim recall —
provided glyphs stay above ~35–40 px²/char and rows align to the vision
patch grid. Our application is narrower than theirs (repo maps, not full
session logs), so the honest version is:

- Input: the same repo-map text we already emit (skeleton + component
  list + surface names), not a new summarizer.
- Render: monospace bitmap, 8×16 cell (matches their doc-8on16 winner),
  black on white, 1568px wide, sentence-cycle ink color optional
  (`--color` flag; default monochrome).
- Encoder: Pillow (already on this machine) renders a monospace TTF
  (Menlo/Courier) onto the bitmap; the CLI shells out only when Pillow
  is present, else it emits the exact `.png.py` recipe. No new Rust
  dependency, no system libs beyond what Pillow already uses.
- Output: `scc snap --out map.png` + a `--tokens` estimate line
  (chars ÷ 4 ≈ text tokens vs PNG pixel-formula image tokens).
- Default OFF: `context.snap_enabled = false`; the flag or config enables
  it. Text packs remain the default carrier — the bitmap is an experiment
  hatch, not a migration.

Why default-off: the article's own data shows a decode tax (extra thinking
tokens) and model-dependent recall; SCC's text packs are the verified
ceiling. Ship the renderer, measure later, never silently switch carriers.

---

## Alternatives considered

- **Mermaid-only, no SVG**: rejected — SVG renders with zero JS and embeds
  in the viewer directly; Mermaid needs a client library.
- **`image` crate for PNG**: rejected — pulls codecs we don't need; the
  `png` crate writes our RGB buffer directly.
- **Bitmap as default carrier**: rejected — decode tax + model variance
  make it an experiment, not the product.
- **React/Svelte viewer**: rejected — server-rendered HTML has no build
  step and matches the daemon's existing dependency footprint.

<!-- trace:v1 id=SPEC-SCC-VIEWER-renderer type=document work=WORK-SCC-VIEWER documents=SPEC-SCC-VIEWER -->
## Bitmap renderer decision (measured 2026-09-17)

No `font`/`glyph`/`embedded-graphics` crate exists in the offline cargo
cache, and macOS ships no legible sub-20px bitmap font file — but Pillow
12.3.0 is installed with TTF support. So the Rust CLI emits the *map text*
plus a pinned Pillow recipe; rendering happens in Python where fonts
already work. Zero new Rust deps, zero vendored font blobs.
Font chain: matplotlib DejaVuSansMono → Menlo/Courier New → PIL bitmap
fallback. Grid: 8px cols / 16px rows on a 1568px canvas (196 chars/row),
height padded to a multiple of 28px so rows align to the 28px vision patch
grid. Token estimate uses the Anthropic pixel formula (w*h/750) against
chars/4 text tokens.

"Spider" is not a word in this codebase; nothing was renamed to it.

<!-- trace:v1 id=SPEC-SCC-VIEWER-risks type=document work=WORK-SCC-VIEWER documents=SPEC-SCC-VIEWER -->
## Risks

- SVG layout is naive grid, not force-directed — fine for ≤100 L1 nodes,
  degrades past that (cap + note, same as Mermaid readability limits).
- Bitmap recall is model-dependent (their table: 0.60–0.88 F1 at 6×10) —
  hence default-off and the `--tokens` honesty line.
- Viewer pages read the live store per request — acceptable on loopback;
  never exposed without the existing remote opt-in gate.

<!-- trace:v1 id=SPEC-SCC-VIEWER-tests type=document work=WORK-SCC-VIEWER documents=SPEC-SCC-VIEWER -->
## Test plan

- `diagram` golden test on a fixture repo: Mermaid output contains the
  known component names and edge predicates; byte-identical on re-run.
- `snap` test: PNG parses (magic bytes + IHDR width), pixelchecksum stable
  across runs, char budget honored.
- Viewer test: route functions return 200 + `text/html` for `/`,
  `/components`, `/flows`, `/diagram`, `/search` against a fixture store.
- Existing suite unchanged: no modified golden bytes.
