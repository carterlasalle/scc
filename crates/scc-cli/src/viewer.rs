//! Web viewer, architecture diagram, and snap-bitmap recipe
//! (SPEC-SCC-VIEWER): human-readable HTML over the live store, a
//! deterministic Mermaid/SVG diagram from the L1 architecture layer, and
//! the repo-map text plus pinned Pillow recipe behind `scc snap`.
//!
//! The viewer reuses `httpd.rs` routing and the loopback gate; the diagram
//! reuses the `export_ccg` L1 filter; the snap recipe carries the same map
//! text the CLI prints. One IR, three surfaces.


/// Diagram model + Mermaid/SVG rendering live in the engine
/// (`scc_engine::diagram`) so every transport renders the same bytes.
/// This module re-exports the engine implementation; the CLI only parses
/// args and prints.
pub use scc_engine::diagram::{DiagramEdge, DiagramModel, DiagramNode};
pub use scc_engine::diagram::{
    MAX_DIAGRAM_NODES, MAX_EDGES_PER_NODE, build_diagram_model, render_mermaid, render_svg,
};

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

// trace:v1 id=impl.crates-scc-cli-src-viewer.page-html-esc work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
    let model = build_diagram_model(store).map_err(|e| crate::CliError::Other(e.to_string()))?;
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
    fn token_estimate_matches_pixel_formula() {
        let (text, img) = token_estimate(40000, 250);
        assert_eq!(text, 10000);
        assert_eq!(img, 1568 * (250_usize * 16).div_ceil(28) * 28 / 750);
    }

    #[test]
    // trace:exempt reason=unit-test
    fn labels_truncate_without_panic() {
        assert_eq!(scc_engine::diagram::truncate_label("abc", 10), "abc");
        assert!(scc_engine::diagram::truncate_label("abcdefghij", 5).ends_with("..."));
        assert_eq!(esc("<a>&"), "&lt;a&gt;&amp;");
    }
}
