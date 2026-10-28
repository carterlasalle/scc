//! Structured operation RPC over stdio: newline-delimited requests, same registry.
//!
//! Request:  {"id": 1, "operation": "context.task", "input": {...}}
//! Response: {"id": 1, "output": {...}} | {"id": 1, "error": "..."}
//! Plus `operations.list` / `operations.describe` meta-operations.
//! SDK subprocess mode speaks this — never CLI text. This is not
//! JSON-RPC 2.0 (`scc mcp` is): there is no `jsonrpc`/`method`/`params`.

// trace:exempt reason=internal-detail
pub fn serve_stdio(root: &std::path::Path) -> crate::Result<()> {
    use std::io::BufRead;
    let stdin = std::io::stdin();
    let mut line = String::new();
    let mut lock = stdin.lock();
    loop {
        line.clear();
        let n = lock.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let msg: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                println!("{}", serde_json::json!({"id": null, "error": format!("invalid JSON: {e}")}));
                continue;
            }
        };
        let id = msg.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let Some(op) = msg.get("operation").and_then(|o| o.as_str()) else {
            let mut keys: Vec<&str> = msg
                .as_object()
                .map(|m| m.keys().map(String::as_str).collect())
                .unwrap_or_default();
            keys.sort_unstable();
            println!(
                "{}",
                serde_json::json!({"id": id, "error": format!("request has no \"operation\" key (got keys: {})", keys.join(", "))})
            );
            continue;
        };
        let input = msg.get("input").cloned().unwrap_or(serde_json::json!({}));
        if op == "operations.list" {
            let ids: Vec<&str> = crate::ops::ids();
            println!("{}", serde_json::json!({"id": id, "output": {"operations": ids, "api_version": scc_api::API_VERSION}}));
            continue;
        }
        if op == "operations.describe" {
            let target = input.get("id").and_then(|v| v.as_str()).unwrap_or(op);
            match crate::ops::describe(target) {
                Some(d) => println!("{}", serde_json::json!({"id": id, "output": d})),
                None => println!("{}", serde_json::json!({"id": id, "error": format!("unknown operation '{target}'")})),
            }
            continue;
        }
        match crate::invoke(root, op, input) {
            Ok(output) => println!("{}", serde_json::json!({"id": id, "output": output})),
            Err(e) => println!("{}", serde_json::json!({"id": id, "error": e.to_string()})),
        }
    }
    Ok(())
}
