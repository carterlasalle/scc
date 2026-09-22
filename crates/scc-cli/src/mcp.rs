//! MCP server (docs/API_AND_INTEGRATIONS.md §2, EPIC-080).
//!
//! Exposes exactly ten intent-level tools to agents — never analyzer-level
//! graph operations:
//!   system_overview, system_atlas, task_context, component_context,
//!   flow_context, impact_context, verify_context, system_context,
//!   surface_map, structural_source
//!
//! Transport: stdio, newline-delimited JSON-RPC 2.0 (MCP stdio framing).
//! Repository read-only by default (docs/SECURITY.md §10).

use std::io::{BufRead, Write};
use std::path::Path;

const PROTOCOL_VERSION: &str = "2025-06-18";
/// The newest MCP protocol revision this server can speak. The client
/// requests a version in `initialize.params.protocolVersion`; we negotiate
/// down to the newest revision we support that is <= the client's, so both
/// the 2025-06-18 and 2025-11-25 protocol generations work (fixwave Item
/// 14 — OMP offers 2025-11-25, other clients 2025-06-18).
const MAX_PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-11-25"];

/// Negotiate the protocol version: prefer the client's requested revision
/// when we support it; otherwise fall back to the newest supported revision
/// that is not newer than the client's request; as a last resort use our
/// oldest supported revision (a client that predates both gets the oldest
/// we speak).
// trace:v1 id=impl.crates-scc-cli-src-mcp.negotiate-protocol work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn negotiate_protocol(requested: Option<&str>) -> &'static str {
    let Some(req) = requested else {
        return PROTOCOL_VERSION;
    };
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&req) {
        return SUPPORTED_PROTOCOL_VERSIONS
            .iter()
            .find(|v| **v == req)
            .copied()
            .unwrap_or(PROTOCOL_VERSION);
    }
    // Unsupported request: if it is newer than everything we support,
    // answer with our newest; if it is older, answer with our oldest.
    if req > MAX_PROTOCOL_VERSION {
        return MAX_PROTOCOL_VERSION;
    }
    PROTOCOL_VERSION
}

// trace:v1 id=impl.crates-scc-cli-src-mcp.Tool work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
struct Tool {
    name: &'static str,
    description: &'static str,
    input_schema: serde_json::Value,
}

/// MCP tool annotations for a read-only deterministic context tool.
/// All ten tools are non-destructive and idempotent for identical model
/// state and input; only `task_context` takes open-world input.
// trace:v1 id=impl.crates-scc-cli-src-mcp.tool-annotations work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
fn tool_annotations(name: &str) -> serde_json::Value {
    serde_json::json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": name == "task_context",
    })
}

// trace:v1 id=impl.scc.mcp work=WORK-SCC-001 satisfies=REQ-SCC-API
// trace:v1 id=impl.crates-scc-cli-src-mcp.tools work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "system_overview",
            description: "Compact system overview: purpose, components, boundaries, stores, external systems, flows, invariants, freshness.",
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
        },
        Tool {
            name: "system_atlas",
            description: "Full System Atlas: complete architecture for session startup (purpose, components, flows, ownership, contracts, invariants, failure paths, deployment, trust boundaries). The primary agent startup tool.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "token_budget": {"type": "integer", "description": "Optional token budget (default context.atlas_tokens)"},
                    "scope": {"type": "string", "description": "production (default: fixture/test/benchmark evidence labels components but never feeds architecture sections) or full (every role feeds architecture)"}
                }
            }),
        },
        Tool {
            name: "task_context",
            description: "Task-specific system context pack for a coding goal. Primary agent operation.",
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["goal"],
                "properties": {
                    "goal": {"type": "string", "description": "The task/goal in natural language"},
                    "files": {"type": "array", "items": {"type": "string"}, "description": "Explicit file paths"},
                    "symbols": {"type": "array", "items": {"type": "string"}, "description": "Explicit symbol names"},
                    "token_budget": {"type": "integer", "minimum": 512, "description": "Hard token budget (default 8000)"}
                }
            }),
        },
        Tool {
            name: "component_context",
            description: "Component detail: responsibility, implementation, dependencies, ownership, flows, contracts, tests, evidence.",
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["component"],
                "properties": {
                    "component": {"type": "string", "description": "Component id or name"}
                }
            }),
        },
        Tool {
            name: "flow_context",
            description: "Flow detail: trigger, steps, branches, data, failures, retries, evidence.",
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["flow"],
                "properties": {
                    "flow": {"type": "string", "description": "Flow id or name"}
                }
            }),
        },
        Tool {
            name: "impact_context",
            description: "Impact of a change: affected components, flows, consumers, contracts, data, invariants, tests, risk.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "files": {"type": "array", "items": {"type": "string"}},
                    "symbols": {"type": "array", "items": {"type": "string"}},
                    "diff": {"type": "string", "description": "Git base revision for diff (e.g. origin/main)"}
                }
            }),
        },
        Tool {
            name: "verify_context",
            description: "Verification report: freshness, stale facts, conflicts, low-confidence dependencies, drift, missing evidence.",
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
        },
        Tool {
            name: "system_context",
            description: "Session-startup artifact: the System Atlas fused with the System Surface Map (the actual callable API layer), model coverage and honest omissions in one deterministic pack. The primary agent startup tool.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"token_budget": {"type": "integer", "description": "Optional token budget. Default is the production adaptive startup total; the atlas:surface split is chosen from repository complexity."}}
            }),
        },
        Tool {
            name: "surface_map",
            description: "The System Surface Map: the repository's actual callable API layer, ranked by global importance — or, with a goal, personalized to that task (task PPR re-ranking).",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "goal": {"type": "string", "description": "Task goal to personalize the map"},
                    "token_budget": {"type": "integer", "description": "Optional token budget (default context.surface_tokens, 7000)"}
                }
            }),
        },
        Tool {
            name: "structural_source",
            description: "Structural Source representation of files: exact declaration headers plus per-symbol call/write evidence (deep) or signatures and imports (fallback). Pass files or a goal (a goal selects the task-matched files via the PPR->Surface pipeline).",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "files": {"type": "array", "items": {"type": "string"}, "description": "Repository-relative file paths or scc:// content handles"},
                    "goal": {"type": "string", "description": "Task goal; resolves to the task-matched files (build_surface Task mode)"},
                    "token_budget": {"type": "integer", "description": "Optional token budget (default context.structural_source, 6000)"}
                }
            }),
        },
    ]
}


// trace:v1 id=impl.crates-scc-cli-src-mcp.send work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn send(msg: &serde_json::Value) {
    let mut line = serde_json::to_string(msg).unwrap_or_default();
    line.push('\n');
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(line.as_bytes());
    let _ = lock.flush();
}

// trace:v1 id=impl.crates-scc-cli-src-mcp.reply work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn reply(id: &serde_json::Value, result: serde_json::Value) {
    send(&serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}));
}

// trace:v1 id=impl.crates-scc-cli-src-mcp.error work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn error(id: &serde_json::Value, code: i64, message: &str) {
    send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message}
    }));
}

/// Run the MCP server over stdin/stdout for `root`.
// trace:v1 id=impl.crates-scc-cli-src-mcp.serve-stdio work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
pub fn serve_stdio(root: &Path) -> crate::Result<()> {
    let stdin = std::io::stdin();
    let mut line = String::new();
    let mut lock = stdin.lock();
    loop {
        line.clear();
        let n = lock.read_line(&mut line)?;
        if n == 0 {
            break; // EOF
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(method) = msg.get("method").and_then(|m| m.as_str()) else {
            continue; // response or notification
        };
        let id = msg.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let params = msg.get("params").cloned().unwrap_or(serde_json::json!({}));

        match method {
            "initialize" => {
                let requested = params
                    .get("protocolVersion")
                    .and_then(|v| v.as_str());
                reply(
                    &id,
                    serde_json::json!({
                        "protocolVersion": negotiate_protocol(requested),
                        "capabilities": {"tools": {"listChanged": false}},
                        "serverInfo": {"name": "scc", "version": env!("CARGO_PKG_VERSION")}
                    }),
                );
            }
            "notifications/initialized" | "notifications/cancelled" => {}
            "ping" => reply(&id, serde_json::json!({})),
            "tools/list" => {
                let list: Vec<serde_json::Value> = tools()
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "name": t.name,
                            "description": t.description,
                            "inputSchema": t.input_schema,
                            "annotations": tool_annotations(t.name),
                        })
                    })
                    .collect();
                reply(&id, serde_json::json!({"tools": list}));
            }
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));
                match call_tool(root, name, &args) {
                    Ok(text) => reply(
                        &id,
                        serde_json::json!({"content": [{"type": "text", "text": text}]}),
                    ),
                    Err(e) => reply(
                        &id,
                        serde_json::json!({
                            "content": [{"type": "text", "text": format!("error: {e}")}],
                            "isError": true
                        }),
                    ),
                }
            }
            other => error(&id, -32601, &format!("method not found: {other}")),
        }
    }
    Ok(())
}

// trace:exempt reason=internal-detail
// trace:v1 id=impl.crates-scc-cli-src-mcp.call-tool work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
fn call_tool(root: &Path, name: &str, args: &serde_json::Value) -> crate::Result<String> {
    // Curated MCP surface stays (10 semantic tools); DERIVATION routes into
    // the operation registry — same engine, same semantics as every other
    // transport. Text extraction below (content/text fields) is rendering.
    let store = crate::open_store(root)?;
    if !store.snapshot_status()?.is_some() {
        return Ok("# NOT INDEXED\nRun `scc index` before asking for system context.".to_string());
    }

    let str_arg = |k: &str| -> String {
        args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
    };
    let arr_arg = |k: &str| -> Vec<String> {
        args.get(k)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let invoke = |op: &str, input: serde_json::Value| -> crate::Result<serde_json::Value> {
        scc_engine::invoke(root, op, input).map_err(|e| crate::CliError::Other(e.to_string()))
    };
    let pack_text = |v: &serde_json::Value| -> String {
        v.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string()
    };


    match name {
        "system_overview" => Ok(pack_text(&invoke("context.overview", serde_json::json!({}))?)),
        "system_atlas" => {
            let budget = args.get("token_budget").and_then(|b| b.as_u64()).map(|b| b as usize);
            let full = args.get("scope").and_then(|s| s.as_str()).map(|s| s == "full").unwrap_or(false);
            Ok(pack_text(&invoke("context.atlas", serde_json::json!({"budget": budget, "full": full}))?))
        }
        "task_context" => {
            let goal = str_arg("goal");
            if goal.is_empty() {
                return Ok("task_context requires a `goal` string.".to_string());
            }
            let budget = args
                .get("token_budget")
                .and_then(|v| v.as_u64())
                .map(|b| b as usize);
            // Transport parity: THE one complete task artifact via invoke.
            let out = invoke("context.task", serde_json::json!({
                "goal": goal, "files": arr_arg("files"),
                "symbols": arr_arg("symbols"), "budget": budget, "hook": false,
            }))?;
            let pack = out.get("pack").cloned().unwrap_or(serde_json::json!({}));
            let delta = out.get("delta").and_then(|d| d.as_str()).unwrap_or("");
            let content = pack_text(&pack);
            if delta.is_empty() {
                Ok(content)
            } else {
                Ok(format!("{content}\n{delta}"))
            }
        }
        "component_context" => {
            let id = str_arg("component");
            if id.is_empty() {
                return Ok("component_context requires a `component` id or name.".to_string());
            }
            Ok(pack_text(&invoke("context.component", serde_json::json!({"id": id}))?))
        }
        "flow_context" => {
            let id = str_arg("flow");
            if id.is_empty() {
                return Ok("flow_context requires a `flow` id or name.".to_string());
            }
            Ok(pack_text(&invoke("context.flow", serde_json::json!({"id": id}))?))
        }
        "impact_context" => {
            let diff = str_arg("diff");
            Ok(pack_text(&invoke("context.impact", serde_json::json!({
                "files": arr_arg("files"), "symbols": arr_arg("symbols"),
                "diff": if diff.is_empty() { serde_json::Value::Null } else { serde_json::json!(diff) },
            }))?))
        }
        "verify_context" => Ok(pack_text(&invoke("context.verify", serde_json::json!({}))?)),
        "system_context" => {
            let budget_tokens = args
                .get("token_budget")
                .and_then(|b| b.as_u64())
                .map(|b| b as usize);
            // Transport parity via the registry: engine startup derives +
            // records the ledger (same as CLI `context startup`).
            let out = invoke("context.startup", serde_json::json!({"budget": budget_tokens}))?;
            Ok(out.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string())
        }
        "surface_map" => {
            let goal = str_arg("goal");
            let tokens = args
                .get("token_budget")
                .and_then(|v| v.as_u64())
                .map(|b| b as usize);
            // Registry derivation (lexical scorer; same as inference-disabled
            // CLI — remote-model wiring stays transport-side via engine API).
            let out = invoke("surface.build", serde_json::json!({
                "task": if goal.is_empty() { serde_json::Value::Null } else { serde_json::json!(goal) },
                "budget": tokens, "explain": false,
            }))?;
            Ok(out.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string())
        }
        "structural_source" => {
            let goal = str_arg("goal");
            let budget = args
                .get("token_budget")
                .and_then(|v| v.as_u64())
                .map(|b| b as usize);
            let files = arr_arg("files");
            let task = if goal.is_empty() { None } else { Some(goal.as_str()) };
            // Registry derivation: the engine owns the structural
            // build; this transport renders its `text` field.
            let task_s: Option<String> = task.map(|s: &str| s.to_string());
            let out = invoke("context.structural", serde_json::json!({
                "files": files, "task": task_s, "budget": budget,
            }))?;
            Ok(out.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string())
        }
        other => Err(crate::CliError::Other(format!("unknown tool: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
// trace:exempt reason=internal-detail
// trace:v1 id=impl.crates-scc-cli-src-mcp.tool-schemas-are-valid-json-schema work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
    fn tool_schemas_are_valid_json_schema() {
        for t in tools() {
            assert_eq!(t.input_schema["type"], "object");
            assert!(t.input_schema.get("properties").is_some());
        }
        assert_eq!(tools().len(), 10, "the ten semantic tools only");
        // truthful annotations: read-only, non-destructive, idempotent;
        // only task_context takes open-world input.
        for t in tools() {
            let a = tool_annotations(t.name);
            assert_eq!(a["readOnlyHint"], true, "{}", t.name);
            assert_eq!(a["destructiveHint"], false, "{}", t.name);
            assert_eq!(a["idempotentHint"], true, "{}", t.name);
            assert_eq!(
                a["openWorldHint"],
                t.name == "task_context",
                "{}",
                t.name
            );
        }
    }

    #[test]
// trace:v1 id=impl.crates-scc-cli-src-mcp.negotiate-protocol-echoes-supported work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
    fn negotiate_protocol_echoes_supported() {
        // A client requesting a version we support gets it back verbatim.
        assert_eq!(negotiate_protocol(Some("2025-06-18")), "2025-06-18");
        assert_eq!(negotiate_protocol(Some("2025-11-25")), "2025-11-25");
    }

    #[test]
// trace:v1 id=impl.crates-scc-cli-src-mcp.negotiate-protocol-newer-falls-back work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
    fn negotiate_protocol_newer_falls_back_to_max() {
        // A client requesting a NEWER protocol than we support gets our
        // newest supported revision (2025-11-25).
        assert_eq!(negotiate_protocol(Some("2026-01-01")), "2025-11-25");
    }

    #[test]
// trace:v1 id=impl.crates-scc-cli-src-mcp.negotiate-protocol-absent-defaults work=WORK-wave-15-2-heterogeneous-hierarchy-edges-semantic-scoring-explain-rank-caching
    fn negotiate_protocol_absent_defaults() {
        // No requested version -> our baseline.
        assert_eq!(negotiate_protocol(None), "2025-06-18");
    }
}
