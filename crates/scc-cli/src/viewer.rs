//! Web viewer, architecture diagram, and snap-bitmap recipe
//! (SPEC-SCC-VIEWER): human-readable HTML over the live store, a
//! deterministic Mermaid/SVG diagram from the L1 architecture layer, and
//! the repo-map text plus pinned Pillow recipe behind `scc snap`.
//!
//! The viewer reuses `httpd.rs` routing and the loopback gate; the diagram
//! reuses the `export_ccg` L1 filter; the snap recipe carries the same map
//! text the CLI prints. One IR, three surfaces.

use scc_core::{kinds, predicates};
use std::collections::{BTreeMap, BTreeSet};

/// L1 architecture nodes plus capped edges, sorted for byte-determinism.
// trace:v1 id=impl.crates-scc-cli-src-viewer.diagram-model work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub struct DiagramModel {
    pub nodes: Vec<DiagramNode>,
    pub edges: Vec<DiagramEdge>,
    /// Flow name plus participant labels in step order.
    pub flows: Vec<(String, Vec<String>)>,
}

/// One architecture-layer entity in the diagram.
#[derive(Debug, Clone)]
// trace:v1 id=impl.crates-scc-cli-src-viewer.diagram-node work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub struct DiagramNode {
    pub id: String,
    pub label: String,
    pub kind: String,
}

/// One architectural relationship between diagram nodes.
#[derive(Debug, Clone)]
// trace:v1 id=impl.crates-scc-cli-src-viewer.diagram-edge work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub struct DiagramEdge {
    pub from: String,
    pub to: String,
    pub label: String,
}

// Readability caps (spec: naive grid degrades past ~100 L1 nodes).
pub const MAX_DIAGRAM_NODES: usize = 100;
// Max edges kept per source node (same limit group as above).
pub const MAX_EDGES_PER_NODE: usize = 6;

/// Build the diagram model from the live store. Same L1 filter as
/// `export_ccg`; edges limited to architectural predicates between known
/// nodes, capped per node after sorting so output is deterministic.
// trace:v1 id=impl.crates-scc-cli-src-viewer.build-diagram-model work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn build_diagram_model(store: &scc_store::Store) -> crate::Result<DiagramModel> {
    let mut nodes: Vec<DiagramNode> = store
        .all_entities()?
        .into_iter()
        .filter(|e| {
            matches!(
                e.kind.as_str(),
                kinds::COMPONENT
                    | kinds::SERVICE
                    | kinds::DATA_STORE
                    | kinds::DEPLOYMENT_UNIT
                    | kinds::EXTERNAL_API
            )
        })
        .map(|e| DiagramNode {
            id: e.id.clone(),
            label: e.name.clone(),
            kind: e.kind.clone(),
        })
        .collect();
    nodes.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
    nodes.truncate(MAX_DIAGRAM_NODES);
    let known: BTreeSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();

    let mut edges: Vec<DiagramEdge> = store
        .all_relationships()?
        .into_iter()
        .filter(|r| {
            matches!(
                r.predicate.as_str(),
                predicates::DEPENDS_ON
                    | predicates::CALLS
                    | predicates::HANDLES
                    | predicates::ROUTES_TO
                    | predicates::CONTAINS
                    | predicates::PUBLISHES
                    | predicates::CONSUMES
                    | predicates::READS
                    | predicates::WRITES
            ) && known.contains(r.subject.as_str())
                && known.contains(r.object.as_str())
        })
        .map(|r| DiagramEdge {
            from: r.subject.clone(),
            to: r.object.clone(),
            label: r.predicate.clone(),
        })
        .collect();
    edges.sort_by(|a, b| {
        (a.from.clone(), a.to.clone(), a.label.clone())
            .cmp(&(b.from.clone(), b.to.clone(), b.label.clone()))
    });
    edges.dedup_by(|a, b| a.from == b.from && a.to == b.to && a.label == b.label);
    // `edges` is sorted, so the first N per source win deterministically.
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = Vec::new();
    for e in edges {
        let n = seen.entry(e.from.clone()).or_insert(0);
        if *n < MAX_EDGES_PER_NODE {
            *n += 1;
            out.push(e);
        }
    }

    let mut flows: Vec<(String, Vec<String>)> = store
        .flows()?
        .into_iter()
        .map(|f| {
            let mut steps = f.steps;
            steps.sort_by_key(|s| s.order);
            let parts: Vec<String> = steps
                .into_iter()
                .map(|s| format!("{}:{}", s.actor, s.operation))
                .collect();
            (f.name, parts)
        })
        .collect();
    flows.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(DiagramModel {
        nodes,
        edges: out,
        flows,
    })
}

/// Sanitize an entity id into a Mermaid-safe node id.
// trace:v1 id=impl.crates-scc-cli-src-viewer.mermaid-id work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
fn mermaid_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for c in id.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, 'n');
    }
    out
}

/// Mermaid flowchart: gitdiagram-shaped output, SCC-derived content.
// trace:v1 id=impl.crates-scc-cli-src-viewer.render-mermaid work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn render_mermaid(model: &DiagramModel) -> String {
    let mut s = String::from("flowchart LR\n");
    for n in &model.nodes {
        let label = n.label.replace('"', "");
        s.push_str(&format!(
            "    {}[\"{} ({})\"]\n",
            mermaid_id(&n.id),
            label,
            n.kind
        ));
    }
    for e in &model.edges {
        s.push_str(&format!(
            "    {} -->|{}| {}\n",
            mermaid_id(&e.from),
            e.label,
            mermaid_id(&e.to)
        ));
    }
    for (i, (name, parts)) in model.flows.iter().enumerate() {
        let fname = name.replace('"', "");
        s.push_str(&format!("    subgraph flow{i}[\"flow: {fname}\"]\n"));
        s.push_str("        direction TB\n");
        for (j, p) in parts.iter().enumerate() {
            let p = p.replace('"', "");
            s.push_str(&format!("        f{i}s{j}[\"{p}\"]\n"));
        }
        s.push_str("    end\n");
    }
    s
}

/// Dependency-free SVG: naive grid layout (spec-acknowledged limit),
/// boxes plus lines plus labels, no JS.
// trace:v1 id=impl.crates-scc-cli-src-viewer.render-svg work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn render_svg(model: &DiagramModel) -> String {
    const COLS: usize = 4;
    const BOX_W: usize = 300;
    const BOX_H: usize = 64;
    const GAP_X: usize = 60;
    const GAP_Y: usize = 48;
    const PAD: usize = 40;
    let rows = model.nodes.len().div_ceil(COLS).max(1);
    let w = PAD * 2 + COLS * BOX_W + (COLS - 1) * GAP_X;
    let flow_h = model.flows.len() * 56;
    let h = PAD * 2 + rows * BOX_H + rows.saturating_sub(1) * GAP_Y + flow_h + 40;
    let pos = |i: usize| {
        let c = i % COLS;
        let r = i / COLS;
        (PAD + c * (BOX_W + GAP_X), PAD + r * (BOX_H + GAP_Y))
    };
    let idx: BTreeMap<&str, usize> = model
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" font-family=\"sans-serif\">\n"
    );
    for e in &model.edges {
        if let (Some(&a), Some(&b)) = (idx.get(e.from.as_str()), idx.get(e.to.as_str())) {
            let (x1, y1) = pos(a);
            let (x2, y2) = pos(b);
            s.push_str(&format!(
                "  <line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#8b949e\" stroke-width=\"1.5\"><title>{}</title></line>\n",
                x1 + BOX_W / 2,
                y1 + BOX_H / 2,
                x2 + BOX_W / 2,
                y2 + BOX_H / 2,
                esc(&e.label)
            ));
        }
    }
    for (i, n) in model.nodes.iter().enumerate() {
        let (x, y) = pos(i);
        s.push_str(&format!(
            "  <rect x=\"{x}\" y=\"{y}\" width=\"{BOX_W}\" height=\"{BOX_H}\" rx=\"8\" fill=\"#131a24\" stroke=\"#2a3442\"/>\n"
        ));
        s.push_str(&format!(
            "  <text x=\"{}\" y=\"{}\" fill=\"#f0f3f6\" font-size=\"15\">{}</text>\n",
            x + 12,
            y + 26,
            esc(&truncate_label(&n.label, 34))
        ));
        s.push_str(&format!(
            "  <text x=\"{}\" y=\"{}\" fill=\"#8b949e\" font-size=\"12\">{}</text>\n",
            x + 12,
            y + 46,
            esc(&n.kind)
        ));
    }
    let mut fy = PAD + rows * BOX_H + rows.saturating_sub(1) * GAP_Y + 28;
    for (name, parts) in &model.flows {
        s.push_str(&format!(
            "  <text x=\"{PAD}\" y=\"{fy}\" fill=\"#DEA584\" font-size=\"14\">flow: {}</text>\n",
            esc(&truncate_label(name, 60))
        ));
        fy += 20;
        let chain = parts
            .iter()
            .take(8)
            .map(|p| truncate_label(p, 30))
            .collect::<Vec<_>>()
            .join(" -> ");
        let more = if parts.len() > 8 {
            format!(" (+{} more)", parts.len() - 8)
        } else {
            String::new()
        };
        s.push_str(&format!(
            "  <text x=\"{PAD}\" y=\"{fy}\" fill=\"#8b949e\" font-size=\"12\">{}{}</text>\n",
            esc(&chain),
            esc(&more)
        ));
        fy += 36;
    }
    s.push_str("</svg>\n");
    s
}

// trace:v1 id=impl.crates-scc-cli-src-viewer.html-esc work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// trace:v1 id=impl.crates-scc-cli-src-viewer.truncate-label work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
fn truncate_label(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}...", &s[..max.saturating_sub(3)])
}

/// The repo-map text behind `scc snap`: overview line plus component
/// lines plus surface symbol names. Same text the CLI prints; the PNG
/// recipe renders exactly these bytes.
// trace:v1 id=impl.crates-scc-cli-src-viewer.map-text work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn map_text(store: &scc_store::Store, max_chars: usize) -> crate::Result<String> {
    let repo = store.repository();
    let mut lines = vec![format!("SCC REPO MAP: {} ({})", repo.name, repo.id)];
    let mut comps = store.components()?;
    comps.sort_by(|a, b| a.name.cmp(&b.name));
    for c in &comps {
        lines.push(format!("[{}] {}", c.kind, c.name));
    }
    let mut syms = store.all_entities()?;
    syms.sort_by(|a, b| a.name.cmp(&b.name));
    for e in syms.iter().filter(|e| e.kind == "symbol").take(400) {
        lines.push(format!("  {} {}", e.kind, e.name));
    }
    let mut out = lines.join("\n");
    out.truncate(max_chars);
    Ok(out)
}

/// Pinned Pillow recipe rendering `map_text` bytes to PNG. Grid: 8px
/// cols and 16px rows on a 1568px canvas (196 chars per row), height
/// padded to a multiple of 28px (vision patch alignment).
// trace:v1 id=impl.crates-scc-cli-src-viewer.snap-recipe work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn snap_recipe() -> &'static str {
    concat!(
        "from PIL import Image, ImageDraw, ImageFont\n",
        "import sys\n",
        "COLS, ROW_H, WIDTH = 196, 16, 1568\n",
        "FONTS = (\"DejaVuSansMono.ttf\", \"/System/Library/Fonts/Supplemental/Courier New.ttf\", \"/System/Library/Fonts/Supplemental/Menlo.ttf\")\n",
        "def load_font(size=16):\n",
        "    for spec in FONTS:\n",
        "        try:\n",
        "            return ImageFont.truetype(spec, size)\n",
        "        except Exception:\n",
        "            continue\n",
        "    return ImageFont.load_default(size=size)\n",
        "def snap(text, out):\n",
        "    font = load_font()\n",
        "    rows = text.splitlines()\n",
        "    h = ((len(rows) * ROW_H + 27) // 28) * 28\n",
        "    img = Image.new(\"RGB\", (WIDTH, h), \"white\")\n",
        "    d = ImageDraw.Draw(img)\n",
        "    for i, line in enumerate(rows):\n",
        "        d.text((4, i * ROW_H), line[:COLS], font=font, fill=\"black\")\n",
        "    img.save(out)\n",
        "if __name__ == \"__main__\":\n",
        "    snap(open(sys.argv[1]).read(), sys.argv[2])\n",
    )
}

/// Honesty line: text tokens (chars divided by 4) versus PNG image
/// tokens (Anthropic pixel formula at the snapped canvas size).
// trace:v1 id=impl.crates-scc-cli-src-viewer.token-estimate work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn token_estimate(map_chars: usize, rows: usize) -> (usize, usize) {
    let text_tokens = map_chars / 4;
    let h = (rows * 16).div_ceil(28) * 28;
    let image_tokens = 1568 * h / 750;
    (text_tokens, image_tokens)
}

/// Minimal page shell: no framework, no build step, loopback only.
// trace:v1 id=impl.crates-scc-cli-src-viewer.page-shell work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title><style>body{{font-family:sans-serif;max-width:1100px;margin:2em auto;padding:0 1em}}nav a{{margin-right:1em}}pre{{background:#f4f4f4;padding:1em;overflow:auto}}table{{border-collapse:collapse}}td,th{{border:1px solid #ccc;padding:.3em .6em;text-align:left}}</style></head><body><nav><a href=\"/\">overview</a><a href=\"/components\">components</a><a href=\"/flows\">flows</a><a href=\"/diagram\">diagram</a><form style=\"display:inline\" action=\"/search\"><input name=\"q\" placeholder=\"search\"></form></nav><h1>{}</h1>{}<hr><p><small>served by scc view (loopback only)</small></p></body></html>",
        esc(title),
        esc(title),
        body
    )
}

/// Overview page from live store reads.
// trace:v1 id=impl.crates-scc-cli-src-viewer.overview-page work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn overview_page(store: &scc_store::Store) -> crate::Result<String> {
    let repo = store.repository();
    let stats = store.stats().unwrap_or_default();
    let mut keys: Vec<&String> = stats.keys().collect();
    keys.sort();
    let mut rows = String::new();
    for k in keys {
        rows.push_str(&format!("<tr><td>{}</td><td>{}</td></tr>", esc(k), stats[k]));
    }
    let stale = crate::stale_paths(store).unwrap_or_default();
    let fresh = if stale.is_empty() {
        "<p>freshness: CURRENT</p>".to_string()
    } else {
        format!("<p>freshness: STALE - {} file(s) changed</p>", stale.len())
    };
    let body = format!(
        "<p>repo {} ({})</p>{fresh}<table>{rows}</table>",
        esc(&repo.name),
        esc(&repo.id)
    );
    Ok(page(&format!("SCC: {}", repo.name), &body))
}

/// Component list page.
// trace:v1 id=impl.crates-scc-cli-src-viewer.components-page work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn components_page(store: &scc_store::Store) -> crate::Result<String> {
    let mut comps = store.components()?;
    comps.sort_by(|a, b| a.name.cmp(&b.name));
    let mut body = String::from("<ul>");
    for c in comps {
        body.push_str(&format!(
            "<li><a href=\"/components/{}\">{} [{}]</a></li>",
            esc(&c.id),
            esc(&c.name),
            esc(&c.kind)
        ));
    }
    body.push_str("</ul>");
    Ok(page("Components", &body))
}

/// Component detail page; None when the id is unknown.
// trace:v1 id=impl.crates-scc-cli-src-viewer.component-detail work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn component_detail_page(store: &scc_store::Store, id: &str) -> crate::Result<Option<String>> {
    for c in store.components()? {
        if c.id == id {
            let body = format!(
                "<p>[{}] {}</p><pre>{}</pre><h2>evidence</h2><pre>{}</pre>",
                esc(&c.kind),
                esc(&c.name),
                esc(&serde_json::to_string_pretty(&c.attributes).unwrap_or_default()),
                esc(&serde_json::to_string_pretty(&c.evidence).unwrap_or_default())
            );
            return Ok(Some(page(&c.name, &body)));
        }
    }
    Ok(None)
}

/// Flow list page.
// trace:v1 id=impl.crates-scc-cli-src-viewer.flow-pages work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn flows_page(store: &scc_store::Store) -> crate::Result<String> {
    let mut flows = store.flows()?;
    flows.sort_by(|a, b| a.name.cmp(&b.name));
    let mut body = String::from("<ul>");
    for f in flows {
        body.push_str(&format!(
            "<li><a href=\"/flows/{}\">{} [{}]</a></li>",
            esc(&f.id),
            esc(&f.name),
            esc(crate::flow_kind_str(&f.kind))
        ));
    }
    body.push_str("</ul>");
    Ok(page("Flows", &body))
}

/// Flow detail page; None when the id is unknown.
// trace:v1 id=impl.crates-scc-cli-src-viewer.flow-detail work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn flow_detail_page(store: &scc_store::Store, id: &str) -> crate::Result<Option<String>> {
    for f in store.flows()? {
        if f.id == id {
            let mut steps = f.steps.clone();
            steps.sort_by_key(|s| s.order);
            let mut body = format!(
                "<p>[{}] {}</p><ol>",
                esc(crate::flow_kind_str(&f.kind)),
                esc(&f.name)
            );
            for s in steps {
                body.push_str(&format!("<li>{}: {}</li>", esc(&s.actor), esc(&s.operation)));
            }
            body.push_str("</ol>");
            return Ok(Some(page(&f.name, &body)));
        }
    }
    Ok(None)
}

/// Lexical search page reusing the store FTS path.
// trace:v1 id=impl.crates-scc-cli-src-viewer.search-page work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn search_page(store: &scc_store::Store, q: &str) -> crate::Result<String> {
    let mut body = format!("<p>query: {}</p>", esc(q));
    if !q.is_empty() {
        let entities = store.search_entities(q, 20).unwrap_or_default();
        body.push_str("<h2>entities</h2><ul>");
        for e in entities {
            body.push_str(&format!("<li>{} [{}]</li>", esc(&e.name), esc(&e.kind)));
        }
        body.push_str("</ul>");
    }
    Ok(page("Search", &body))
}

/// Diagram page: inline SVG plus copyable Mermaid source.
// trace:v1 id=impl.crates-scc-cli-src-viewer.diagram-page work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn diagram_page(store: &scc_store::Store) -> crate::Result<String> {
    let model = build_diagram_model(store)?;
    let svg = render_svg(&model);
    let mermaid = render_mermaid(&model);
    let body = format!(
        "<p>{} nodes, {} edges, {} flows (caps: {}/{})</p>{}<h2>mermaid</h2><pre>{}</pre>",
        model.nodes.len(),
        model.edges.len(),
        model.flows.len(),
        MAX_DIAGRAM_NODES,
        MAX_EDGES_PER_NODE,
        svg,
        esc(&mermaid)
    );
    Ok(page("Diagram", &body))
}

/// Viewer route predicate: exact pages plus `/components/<id>`,
/// `/flows/<id>`, and `/search` with query string.
// trace:v1 id=impl.crates-scc-cli-src-viewer.is-viewer-path work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn is_viewer_path(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or(url);
    path == "/"
        || path == "/components"
        || path == "/flows"
        || path == "/diagram"
        || path == "/search"
        || path.starts_with("/components/")
        || path.starts_with("/flows/")
}

/// Serve one viewer route: (status, HTML body). Query strings honored on
/// `/search?q=`; unknown ids yield 404.
// trace:v1 id=impl.crates-scc-cli-src-viewer.serve-viewer work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub fn serve_viewer(store: &scc_store::Store, url: &str) -> (u16, String) {
    let (path, query) = match url.split_once('?') {
        Some((p, q)) => (p, q),
        None => (url, ""),
    };
    let q = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("q="))
        .map(percent_decode)
        .unwrap_or_default();
    match path {
        "/" => (200, overview_page(store).unwrap_or_else(|e| format!("error: {e}"))),
        "/components" => (200, components_page(store).unwrap_or_else(|e| format!("error: {e}"))),
        "/flows" => (200, flows_page(store).unwrap_or_else(|e| format!("error: {e}"))),
        "/diagram" => (200, diagram_page(store).unwrap_or_else(|e| format!("error: {e}"))),
        "/search" => (200, search_page(store, &q).unwrap_or_else(|e| format!("error: {e}"))),
        p if p.starts_with("/components/") => {
            let id = percent_decode(p.trim_start_matches("/components/"));
            match component_detail_page(store, &id) {
                Ok(Some(html)) => (200, html),
                _ => (404, page("Not found", "<p>unknown component</p>")),
            }
        }
        p if p.starts_with("/flows/") => {
            let id = percent_decode(p.trim_start_matches("/flows/"));
            match flow_detail_page(store, &id) {
                Ok(Some(html)) => (200, html),
                _ => (404, page("Not found", "<p>unknown flow</p>")),
            }
        }
        _ => (404, page("Not found", "<p>no such viewer route</p>")),
    }
}

/// Minimal percent-decoding for route ids and `q=` values.
// trace:v1 id=impl.crates-scc-cli-src-viewer.percent-decode work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut bytes = s.as_bytes().iter();
    while let Some(&b) = bytes.next() {
        if b == b'%' {
            let hi = bytes.next().copied().unwrap_or(b'0');
            let lo = bytes.next().copied().unwrap_or(b'0');
            let hex = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
            out.push((hex(hi) * 16 + hex(lo)) as char);
        } else if b == b'+' {
            out.push(' ');
        } else {
            out.push(b as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
// trace:exempt reason=unit-test
    use super::*;

    #[test]
    // trace:exempt reason=unit-test
    fn mermaid_ids_are_safe_and_deterministic() {
        assert_eq!(mermaid_id("repo://a/b c"), "repo___a_b_c");
        assert_eq!(mermaid_id("repo://a/b c"), mermaid_id("repo://a/b c"));
        assert!(
            render_mermaid(&DiagramModel {
                nodes: vec![],
                edges: vec![],
                flows: vec![]
            })
            .starts_with("flowchart LR")
        );
    }

    #[test]
    // trace:exempt reason=unit-test
    fn token_estimate_matches_pixel_formula() {
        let (text, img) = token_estimate(40000, 250);
        assert_eq!(text, 10000);
        assert_eq!(img, 1568 * (250_usize * 16).div_ceil(28) * 28 / 750);
    }

    #[test]
    // trace:exempt reason=unit-test
    fn labels_truncate_without_panic() {
        assert_eq!(truncate_label("abc", 10), "abc");
        assert!(truncate_label("abcdefghij", 5).ends_with("..."));
        assert_eq!(esc("<a>&"), "&lt;a&gt;&amp;");
    }
}
