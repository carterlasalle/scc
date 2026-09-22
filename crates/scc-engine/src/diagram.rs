//! Architecture diagram model + deterministic Mermaid/SVG rendering.
//!
//! Pure Store -> model -> text. Lives in the engine so every transport
//! (CLI, RPC, HTTP, SDKs, FFI) renders the same diagram from the same code;
//! the CLI only parses args and prints. Map/snap/viewer HTML pages stay
//! CLI-local (terminal/HTTP-viewer UX, not model behavior).

use scc_core::{kinds, predicates};
use std::collections::{BTreeMap, BTreeSet};

/// L1 architecture nodes plus capped edges, sorted for byte-determinism.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// trace:v1 id=impl.crates-scc-cli-src-viewer.diagram-model work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub struct DiagramModel {
    pub nodes: Vec<DiagramNode>,
    pub edges: Vec<DiagramEdge>,
    /// Flow name plus participant labels in step order.
    pub flows: Vec<(String, Vec<String>)>,
}

/// One architecture-layer entity in the diagram.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
// trace:v1 id=impl.crates-scc-cli-src-viewer.diagram-node work=WORK-SCC-VIEWER satisfies=SPEC-SCC-VIEWER
pub struct DiagramNode {
    pub id: String,
    pub label: String,
    pub kind: String,
}

/// One architectural relationship between diagram nodes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
pub fn truncate_label(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}...", &s[..max.saturating_sub(3)])
}

#[cfg(test)]
mod tests {
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
}
