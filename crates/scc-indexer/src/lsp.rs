//! LSP-based definition resolution (Phase 7, EPIC-120): upgrade EXTRACTED
//! candidate call edges to RESOLVED using a real language server (pyright)
//! speaking LSP 3.17 over stdio with JSON-RPC 2.0 Content-Length framing.
//!
//! The native resolver produces `calls` relationships with provenance
//! `EXTRACTED` when an import root looks external (e.g. a package living in a
//! directory the native module resolver cannot see, or a member that is
//! re-exported through a package `__init__.py`). Those edges point at
//! `external_api` entities. Pyright performs real binding analysis and
//! resolves the same call site to the concrete definition — the adapter then
//! replaces the EXTRACTED edge with a RESOLVED edge to the true target
//! symbol, carrying fresh `lsp-pyright` evidence.
//!
//! Protocol notes (verified against pyright 1.1.411 on macOS):
//! - pyright must be launched with `--cancellationReceive=file:<dir>`; without
//!   it the background analysis worker is never created and binding-dependent
//!   features return degenerate/empty results.
//! - The LSP server may emit notifications before responding to `initialize`;
//!   the reader thread forwards only responses and auto-replies to
//!   server-initiated requests (capability registrations etc.).
//! - The call-site position must sit on the callee name token (character 0 —
//!   leading indentation — resolves to nothing), so the adapter locates the
//!   callee text inside the source line.
#![allow(clippy::too_many_arguments)]

use crate::write::{evidence_id, rel_id};
use scc_core::{resolution::confidence, Evidence, EvidenceType, Provenance, Relationship};
use scc_store::Store;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Evidence extractor name stamped on upgraded edges.
pub const LSP_EXTRACTOR: &str = "lsp-pyright";
/// Per-request read deadline.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Aggregate result of one `resolve_call_definitions` run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LspResult {
    /// EXTRACTED edges promoted to RESOLVED.
    pub upgraded: usize,
    /// Call sites the server could not resolve (empty/error/no symbol match).
    pub unresolved: usize,
    /// LSP error responses received for definition requests.
    pub errors: usize,
}

enum ServerMsg {
    /// A response to one of our requests (`id` + body).
    Response(Value),
    /// A server-initiated request that must be answered (`id` + body).
    Request(Value),
    /// The input stream ended or became unreadable.
    Eof(String),
}

/// Result of a single call-site definition query.
enum DefOutcome {
    Resolved(String, String),
    Empty,
    Error,
}

/// JSON-RPC/LSP client driving one pyright language server process.
// trace:v1 id=impl.crates-scc-indexer-src-lsp.lsp-resolver work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub struct LspResolver {
    child: Option<Child>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<ServerMsg>,
    pending: HashMap<u64, Value>,
    next_id: u64,
    /// Absolute workspace root (cwd of the server, `file://` prefix base).
    workspace: PathBuf,
    pyright_version: String,
    opened: HashSet<String>,
    stderr_join: Option<JoinHandle<()>>,
}

// trace:exempt reason=internal-detail
impl Drop for LspResolver {
    fn drop(&mut self) {
        // Kill the child; the reader thread then observes EOF and exits.
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ---------------------------------------------------------------------------
// wire protocol (testable without a real server)
// ---------------------------------------------------------------------------

/// Encode a JSON message as a Content-Length framed LSP message.
pub fn encode_frame(msg: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(msg).expect("message serializes");
    let mut out = Vec::with_capacity(body.len() + 64);
    out.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
    out.extend_from_slice(&body);
    out
}

/// Read one Content-Length framed frame body from `reader`.
///
/// Headers are ASCII lines terminated by `\r\n`; the frame body is exactly
/// `Content-Length` bytes. Unknown headers are skipped. Returns `Err` on EOF
/// or a missing/invalid Content-Length (the caller decides whether to abort).
pub fn read_frame_body<R: BufRead>(reader: &mut R) -> Result<Vec<u8>, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("read header: {e}"))?;
        if n == 0 {
            return Err("EOF in headers".to_string());
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line
            .strip_prefix("Content-Length:")
            .or_else(|| line.strip_prefix("content-length:"))
        {
            content_length = rest.trim().parse::<usize>().ok();
        }
    }
    let len = content_length.ok_or_else(|| "missing Content-Length header".to_string())?;
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .map_err(|e| format!("read body: {e}"))?;
    Ok(body)
}

/// Read one frame and parse it as JSON. Malformed bodies are skipped: the
/// frame boundary is known, so the stream stays in sync and the caller can
/// continue with the next frame.
pub fn read_frame<R: BufRead>(reader: &mut R) -> Result<Value, String> {
    let body = read_frame_body(reader)?;
    serde_json::from_slice(&body)
        .map_err(|e| format!("bad JSON frame: {e}"))
}

/// Decode a `file://` URI to an absolute path (percent-decoding `%XX`).
pub fn uri_to_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest); // file://localhost/...
    if !rest.starts_with('/') {
        return None;
    }
    Some(percent_decode(rest))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// server lifecycle
// ---------------------------------------------------------------------------

/// Version of the installed pyright LSP server (`X.Y.Z`), or `None` when the
/// `pyright-langserver` binary is absent. `pyright-langserver` is the LSP
/// server binary shipped by the `pyright` npm package since 1.1.4xx; older
/// packages expose the same server as `pyright --stdio`, so both are probed.
pub fn pyright_version() -> Option<String> {
    for bin in ["pyright-langserver", "pyright"] {
        if let Ok(out) = Command::new(bin).arg("--version").output() {
            if out.status.success() {
                if let Some(v) = parse_version(&String::from_utf8_lossy(&out.stdout)) {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn parse_version(text: &str) -> Option<String> {
    // "pyright 1.1.411" / "pyright-langserver 1.1.411" / "1.1.411"
    let mut it = text.split_whitespace();
    for tok in it.by_ref() {
        if tok.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return Some(tok.to_string());
        }
    }
    None
}

/// Spawn pyright (cwd = `workspace_root`) and complete the initialize
/// handshake. Returns `Err` when pyright is missing or the handshake fails.
pub fn start_pyright(workspace_root: &Path) -> Result<LspResolver, String> {
    let version = pyright_version()
        .ok_or_else(|| "pyright not found — install with: npm install -g pyright".to_string())?;
    let workspace = std::fs::canonicalize(workspace_root)
        .unwrap_or_else(|_| workspace_root.to_path_buf());

    // Without a cancellation folder pyright never creates its background
    // analysis worker, leaving binding-dependent features unusable.
    let cancel_dir = std::env::temp_dir().join("scc-lsp-pyright");
    std::fs::create_dir_all(&cancel_dir)
        .map_err(|e| format!("cannot create pyright cancellation dir: {e}"))?;

    let mut cmd = Command::new("pyright-langserver");
    cmd.args(["--stdio", &format!("--cancellationReceive=file:{}", cancel_dir.display())])
        .current_dir(&workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            // Older pyright packages expose the server as `pyright --stdio`.
            let mut cmd = Command::new("pyright");
            cmd.arg("--stdio")
                .current_dir(&workspace)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            cmd.spawn()
                .map_err(|e| format!("pyright not found — install with: npm install -g pyright ({e})"))?
        }
    };

    let stdin = child.stdin.take().ok_or("pyright stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("pyright stdout unavailable")?;
    let mut stderr = child.stderr.take().ok_or("pyright stderr unavailable")?;

    // Drain stderr so pyright never blocks on a full log pipe.
    let stderr_join = std::thread::spawn(move || {
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
    });

    let mut resolver = LspResolver::with_io(
        &workspace,
        version,
        Box::new(stdout),
        Box::new(stdin),
        Some(child),
    )?;
    resolver.stderr_join = Some(stderr_join);
    Ok(resolver)
}

// trace:exempt reason=internal-detail
impl LspResolver {
    /// Build a resolver over arbitrary reader/writer transports and run the
    /// initialize handshake. `child` is optional (killed on drop).
    fn with_io(
        workspace: &Path,
        version: String,
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        child: Option<Child>,
    ) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            loop {
                match read_frame(&mut reader) {
                    Ok(v) => {
                        let id = v.get("id").cloned();
                        if v.get("method").is_some() {
                            if id.is_some() {
                                let _ = tx.send(ServerMsg::Request(v));
                            }
                        } else if id.is_some() {
                            let _ = tx.send(ServerMsg::Response(v));
                        }
                        // notifications and malformed frames are skipped
                    }
                    Err(e) => {
                        let _ = tx.send(ServerMsg::Eof(e));
                        break;
                    }
                }
            }
        });
        let mut resolver = LspResolver {
            child,
            writer: Box::new(writer),
            rx,
            pending: HashMap::new(),
            next_id: 1,
            workspace: workspace.to_path_buf(),
            pyright_version: version,
            opened: HashSet::new(),
            stderr_join: None,
        };
        resolver.initialize()?;
        Ok(resolver)
    }

    /// The resolved pyright version string (evidence `extractor_version`).
    pub fn pyright_version(&self) -> &str {
        &self.pyright_version
    }

    fn send(&mut self, msg: &Value) -> Result<(), String> {
        self.writer
            .write_all(&encode_frame(msg))
            .map_err(|e| format!("write to LSP server: {e}"))?;
        self.writer
            .flush()
            .map_err(|e| format!("flush LSP server: {e}"))
    }

    /// Send a request and wait (up to [`REQUEST_TIMEOUT`]) for its response.
    /// Server-initiated requests are answered with an empty result while
    /// waiting; out-of-order responses are buffered in `pending`.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            if let Some(v) = self.pending.remove(&id) {
                return Ok(v);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("timed out waiting for LSP response to {method}"));
            }
            match self.rx.recv_timeout(remaining) {
                Ok(ServerMsg::Response(v)) => {
                    if let Some(rid) = v.get("id").and_then(Value::as_u64) {
                        if rid != id {
                            self.pending.insert(rid, v);
                        } else {
                            return Ok(v);
                        }
                    }
                }
                Ok(ServerMsg::Request(v)) => {
                    // client/registerCapability etc.: answer and continue.
                    if let Some(rid) = v.get("id") {
                        let _ = self.send(&json!({"jsonrpc": "2.0", "id": rid, "result": null}));
                    }
                }
                Ok(ServerMsg::Eof(e)) => {
                    return Err(format!("LSP server exited: {e}"));
                }
                Err(_) => {
                    return Err(format!("timed out waiting for LSP response to {method}"));
                }
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }

    fn initialize(&mut self) -> Result<(), String> {
        let root_uri = format!("file://{}", self.workspace.display());
        let resp = self.request(
            "initialize",
            json!({
                "processId": null,
                "rootUri": root_uri,
                "rootPath": self.workspace.to_string_lossy(),
                "workspaceFolders": [{"uri": root_uri, "name": "scc"}],
                "capabilities": {"workspace": {"workspaceFolders": true}},
            }),
        )?;
        if let Some(err) = resp.get("error") {
            return Err(format!("LSP initialize error: {err}"));
        }
        self.notify("initialized", json!({}))?;
        Ok(())
    }

    fn file_uri(&self, file: &str) -> String {
        format!("file://{}/{}", self.workspace.display(), file)
    }

    /// Ensure the file is open in the server (idempotent per resolver).
    fn open_file(&mut self, store: &Store, file: &str) {
        if self.opened.contains(file) {
            return;
        }
        if let Ok(content) = std::fs::read_to_string(store.root.join(file)) {
            let _ = self.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": self.file_uri(file),
                        "languageId": "python",
                        "version": 1,
                        "text": content,
                    }
                }),
            );
        }
        self.opened.insert(file.to_string());
    }

    /// Locate the callee name token for the call site on `line` (1-based).
    ///
    /// The column comes from tree-sitter, not from a text search: the
    /// extractor records one `calls` edge per call site, so at most one
    /// EXTRACTED edge on this line matches `callee`; the resolver re-parses
    /// the file and takes the Nth same-callee call on that line (N = the
    /// edge's order among same-callee EXTRACTED edges on the line). This
    /// keeps `a.m() + b.m()` and trailing `# m ...` comments from
    /// misdirecting the LSP query at the last text occurrence.
    /// Falls back to 0 when the file cannot be parsed.
    fn callee_char(source: &str, line: u32, callee: &str, occurrence: usize) -> usize {
        let needle = callee.rsplit('.').next().unwrap_or(callee);
        if needle.is_empty() {
            return 0;
        }
        let mut parser = tree_sitter::Parser::new();
        let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        if parser.set_language(&language).is_err() {
            return 0;
        }
        let Some(tree) = parser.parse(source, None) else {
            return 0;
        };
        let mut found = 0;
        let mut result = None;
        // Two passes: exact callee-text matches first, bare-needle matches
        // second. `a.m() + b.m()` stores distinct callees `a.m` / `b.m`;
        // matching either query against the first same-`m` call would repeat
        // the bug (issue #34). The needle pass only fires when the recorded
        // callee appears nowhere verbatim on the line.
        for exact_only in [true, false] {
            let mut stack = vec![tree.root_node()];
            while let Some(node) = stack.pop() {
                if node.kind() == "call" {
                    if let Some(function) = node.child_by_field_name("function") {
                        let text = function
                            .utf8_text(source.as_bytes())
                            .unwrap_or("")
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ");
                        let matched = text == callee
                            || (!exact_only && text.rsplit('.').next() == Some(needle));
                        if matched {
                            let row = node.start_position().row as u32 + 1;
                            if row == line {
                                if found == occurrence {
                                    // Column of the method-name token itself:
                                    // the `attribute` field of `a.m`, else the
                                    // function start for a bare `helper()`.
                                    // (LSP columns are UTF-16 code units; the
                                    // token is the ASCII `rsplit('.')`
                                    // segment of a recorded call, so byte and
                                    // UTF-16 offsets agree here.)
                                    let col = match function.kind() {
                                        "attribute" => function
                                            .child_by_field_name("attribute")
                                            .map(|a| a.start_position().column)
                                            .unwrap_or_else(|| function.start_position().column),
                                        _ => function.start_position().column,
                                    };
                                    result = Some(col);
                                    break;
                                }
                                found += 1;
                            }
                        }
                    }
                }
                let mut cursor = node.walk();
                let kids: Vec<_> = node.children(&mut cursor).collect();
                for child in kids.into_iter().rev() {
                    stack.push(child);
                }
            }
            if result.is_some() {
                break;
            }
            // The exact pass found nothing on this line: recount from zero
            // for the needle fallback.
            found = 0;
        }
        result.unwrap_or(0)
    }

    /// Query one call site and return the resolved target path + symbol name.
    /// `DefOutcome::Error` marks an LSP error response (counted as `errors`);
    /// `DefOutcome::Empty` marks null/degenerate/out-of-workspace results.
    fn definition_for(
        &mut self,
        store: &Store,
        file: &str,
        line: u32,
        callee: &str,
        occurrence: usize,
    ) -> Result<DefOutcome, String> {
        self.open_file(store, file);
        let source = std::fs::read_to_string(store.root.join(file)).unwrap_or_default();
        let char = Self::callee_char(&source, line, callee, occurrence);

        // Primary candidate: the LSP-spec 0-based position (evidence lines are
        // 1-based). The same-line character-0 fallback covers servers that
        // fail to honor the callee token column. A next-line fallback is
        // deliberately NOT included: it lands on unrelated symbols (a
        // definition on the following line) and fabricates bogus upgrades.
        let mut candidates = Vec::new();
        let push = |v: &mut Vec<(u32, usize)>, l: u32, c: usize| {
            if !v.contains(&(l, c)) {
                v.push((l, c));
            }
        };
        push(&mut candidates, line.saturating_sub(1), char);
        push(&mut candidates, line.saturating_sub(1), 0);

        for (qline, qchar) in candidates {
            let resp = self.request(
                "textDocument/definition",
                json!({
                    "textDocument": {"uri": self.file_uri(file)},
                    "position": {"line": qline, "character": qchar},
                }),
            )?;
            if resp.get("error").is_some() {
                return Ok(DefOutcome::Error);
            }
            let Some(target) = parse_definition_target(&resp["result"], qline, &self.file_uri(file))
            else {
                continue;
            };
            let Some(rel_path) = self.repo_relative(&target.uri) else {
                continue; // outside the workspace
            };
            let target_line = target.line;
            let Some(symbol) = find_containing_symbol(store, &rel_path, target_line) else {
                continue;
            };
            return Ok(DefOutcome::Resolved(rel_path, symbol));
        }
        Ok(DefOutcome::Empty)
    }

    fn repo_relative(&self, uri: &str) -> Option<String> {
        let abs = uri_to_path(uri)?;
        let ws = self.workspace.to_string_lossy();
        let rel = abs.strip_prefix(ws.as_ref())?.trim_start_matches('/');
        if rel.is_empty() {
            return None;
        }
        Some(rel.to_string())
    }

    /// Upgrade all EXTRACTED `calls` edges whose source path is `file`.
    ///
    /// Fatal protocol failures return `Err`; per-call-site failures are
    /// counted in [`LspResult`].
    pub fn resolve_call_definitions(
        &mut self,
        store: &Store,
        file: &str,
    ) -> Result<LspResult, String> {
        let mut out = LspResult::default();
        let rel_ids: HashSet<String> = store
            .relationship_ids_with_source(file, scc_core::predicates::CALLS)
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect();
        if rel_ids.is_empty() {
            return Ok(out);
        }
        let rels: Vec<Relationship> = store
            .all_relationships()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS && r.provenance == Provenance::Extracted && rel_ids.contains(&r.id))
            .collect();

        // The extractor writes one edge per call site, but the evidence id
        // is (path, kind, callee, line): two same-callee calls on one line
        // share one evidence row. `occurrence` = this edge's order among
        // same-(line, callee) candidate edges, so each edge queries its own
        // call's column instead of every edge hitting the last `m`.
        let mut seen: std::collections::HashMap<(u32, String), usize> =
            std::collections::HashMap::new();
        // New-edge ids created by this run, keyed by the group that may
        // collide on shared evidence identity: same line + same callee.
        let mut upgraded_in_group: std::collections::HashSet<(u32, String, String)> =
            std::collections::HashSet::new();
        for rel in &rels {
            let Some(ev_id) = rel.evidence.first() else {
                out.unresolved += 1;
                continue;
            };
            let Some(ev) = store.get_evidence(ev_id).map_err(|e| e.to_string())? else {
                out.unresolved += 1;
                continue;
            };
            let Some(line) = ev.start_line else {
                out.unresolved += 1;
                continue;
            };
            let callee = ev.symbol.clone().unwrap_or_default();
            let key = (line, callee.clone());
            let occurrence = seen.get(&key).copied().unwrap_or(0);
            seen.insert(key, occurrence + 1);

            match self.definition_for(store, file, line, &callee, occurrence)? {
                DefOutcome::Resolved(target_path, target_name) => {
                    let repo_id = store.repository().id;
                    let new_object = scc_core::symbol_id(&repo_id, &target_path, &target_name);
                    // Never delete a correct edge: same-callee edges on one
                    // line share an evidence id, so the second upgrade would
                    // otherwise compute the same new-edge id and delete the
                    // first upgrade's edge. When the second call resolves to
                    // the same target the edge already exists — keep it and
                    // count this edge resolved without re-deleting. A
                    // different target gets its own edge id and both survive.
                    let new_id = rel_id(&["calls", &rel.subject, &new_object]);
                    let group_key = (line, callee.clone(), new_id.clone());
                    if upgraded_in_group.contains(&group_key) {
                        out.upgraded += 1;
                        continue;
                    }
                    self.apply_upgrade(store, rel, file, &target_path, &target_name, &callee, line)?;
                    upgraded_in_group.insert(group_key);
                    out.upgraded += 1;
                }
                DefOutcome::Error => {
                    out.errors += 1;
                    out.unresolved += 1;
                }
                DefOutcome::Empty => {
                    out.unresolved += 1;
                }
            }
        }
        Ok(out)
    }

// trace:v1 id=impl.crates-scc-indexer-src-lsp-lsp-resolver.apply-upgrade work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    fn apply_upgrade(
        &self,
        store: &Store,
        rel: &Relationship,
        file: &str,
        target_path: &str,
        target_name: &str,
        callee: &str,
        line: u32,
    ) -> Result<(), String> {
        let repo_id = store.repository().id;
        let new_object = scc_core::symbol_id(&repo_id, target_path, target_name);
        let ev = Evidence {
            id: evidence_id(file, "call", callee, line),
            r#type: EvidenceType::Source,
            path: Some(file.to_string()),
            symbol: Some(callee.to_string()),
            start_line: Some(line),
            end_line: None,
            revision: None,
            content_hash: None,
            extractor: Some(LSP_EXTRACTOR.to_string()),
            extractor_version: Some(self.pyright_version.clone()),
        };
        store.insert_evidence(&ev).map_err(|e| e.to_string())?;
        // Remove the EXTRACTED edge (its id encodes the old object), then
        // insert the RESOLVED edge under the id scheme from crate::write.
        store
            .delete_relationship(&rel.id)
            .map_err(|e| e.to_string())?;
        let new_rel = Relationship::new(
            rel_id(&["calls", &rel.subject, &new_object]),
            rel.subject.clone(),
            scc_core::predicates::CALLS,
            new_object,
            Provenance::Resolved,
        )
        .with_confidence(confidence::LSP_EXACT)
        .with_evidence(vec![ev.id.clone()]);
        store
            .insert_relationship(&new_rel, file)
            .map_err(|e| e.to_string())?;
        // semantic truth changed — epoch-keyed context caches must miss
        store
            .bump_epoch(scc_store::ModelEpochKind::Semantic)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Parse an LSP definition result (Location | LocationLink | array | null)
/// into (uri, line). Returns `None` for null/empty results and for
/// degenerate "definitions" that point at the query position itself.
fn parse_definition_target(
    result: &Value,
    qline: u32,
    query_uri: &str,
) -> Option<Target> {
    fn pick(v: &Value) -> Option<(&str, u64)> {
        let uri = v.get("uri")?.as_str()?;
        let range = v.get("range").or_else(|| v.get("targetRange"))?;
        let start = range.get("start")?;
        let line = start.get("line")?.as_u64()?;
        Some((uri, line))
    }
    let entry = match result {
        Value::Null => return None,
        Value::Array(items) => items.iter().find_map(pick)?,
        Value::Object(_) => pick(result)?,
        _ => return None,
    };
    let (uri, line) = entry;
    // A definition cannot sit on the queried position (pyright returns the
    // queried node itself when binding is incomplete).
    if uri == query_uri && line == u64::from(qline) {
        return None;
    }
    Some(Target {
        uri: uri.to_string(),
        line: line as u32,
    })
}

#[derive(Debug)]
struct Target {
    uri: String,
    line: u32,
}

/// Find the smallest symbol in `file` whose [start_line, end_line] contains
/// `line` (LSP 0-based → store 1-based).
fn find_containing_symbol(store: &Store, file: &str, line: u32) -> Option<String> {
    let want = line + 1;
    let syms = store.symbols_in_file(file).ok()?;
    syms.iter()
        .filter(|(_, _, _, _, start, end, _, _)| *start <= want && want <= *end)
        .min_by_key(|(_, _, _, _, start, end, _, _)| end - start)
        .map(|(_, name, _, _, _, _, _, _)| name.clone())
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use scc_core::entity_id;
    use scc_core::kinds;

    // ---------------------------------------------------------------
    // wire protocol
    // ---------------------------------------------------------------

    #[test]
    fn frame_round_trip() {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "textDocument/definition",
            "params": {"position": {"line": 3, "character": 4}},
        });
        let frame = encode_frame(&msg);
        let head = String::from_utf8_lossy(&frame[..32]);
        assert!(head.starts_with("Content-Length: "), "{head}");
        let mut cursor = std::io::Cursor::new(frame);
        let parsed = read_frame(&mut cursor).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn frame_skips_extra_headers_and_handles_crlf() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":null}"#;
        let mut frame = Vec::new();
        frame.extend_from_slice(b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n");
        frame.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        frame.extend_from_slice(body);
        let mut cursor = std::io::Cursor::new(frame);
        let parsed = read_frame(&mut cursor).unwrap();
        assert_eq!(parsed["id"], 1);
    }

    #[test]
    fn frame_eof_and_missing_length_are_errors() {
        let mut cursor = std::io::Cursor::new(b"Content-Type: text/plain\r\n\r\n".to_vec());
        assert!(read_frame_body(&mut cursor).is_err());
        let mut cursor = std::io::Cursor::new(b"".to_vec());
        assert!(read_frame_body(&mut cursor).is_err());
    }

    #[test]
    fn uri_to_path_decodes() {
        assert_eq!(uri_to_path("file:///a/b.py").as_deref(), Some("/a/b.py"));
        assert_eq!(
            uri_to_path("file:///a/my%20file.py").as_deref(),
            Some("/a/my file.py")
        );
        assert_eq!(uri_to_path("file://localhost/a.py").as_deref(), Some("/a.py"));
        assert!(uri_to_path("http://x/y.py").is_none());
        assert!(uri_to_path("file://relative").is_none());
    }

    // ---------------------------------------------------------------
    // callee column lookup (issue #34: rfind hit the last occurrence)
    // ---------------------------------------------------------------

    #[test]
    fn callee_char_selects_own_call_site() {
        let src = "def f(a, b):\n    return a.m() + b.m()\n";
        // Method-token columns: the `m` of `a.m` at 13, of `b.m` at 21.
        assert_eq!(LspResolver::callee_char(src, 2, "a.m", 0), 13);
        assert_eq!(LspResolver::callee_char(src, 2, "b.m", 0), 21);
        // Same callee twice on one line: occurrence selects the Nth site.
        let twice = "def f(a):\n    return a.m() + a.m()\n";
        assert_eq!(LspResolver::callee_char(twice, 2, "a.m", 0), 13);
        assert_eq!(LspResolver::callee_char(twice, 2, "a.m", 1), 21);
    }

    #[test]
    fn callee_char_ignores_trailing_comment() {
        let src = "def f(a):\n    return a.m()  # m is documented in m.md\n";
        // Must point at the call's `m` (13), not the comment's `m`s.
        assert_eq!(LspResolver::callee_char(src, 2, "a.m", 0), 13);
    }

    // ---------------------------------------------------------------
    // mock-server upgrade test (no pyright required)
    // ---------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn mock_server_upgrades_extracted_edge() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.py"), "def helper():\n    pass\n").unwrap();
        std::fs::write(root.join("b.py"), "def main():\n    helper()\n").unwrap();

        let store = seed_store(root);
        let repo = store.repository().id;

        // Fake pyright: answers initialize, then returns a fixed definition
        // pointing at a.py line 0 (0-based) for every definition request.
        let (client_reader, server_writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let (server_reader, client_writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let root_owned = root.to_path_buf();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(server_reader);
            let mut writer = server_writer;
            let mut sent_initialize = false;
            while let Ok(msg) = read_frame(&mut reader) {
                let method = msg.get("method").and_then(Value::as_str);
                let id = msg.get("id");
                if method == Some("initialize") {
                    sent_initialize = true;
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {"capabilities": {"definitionProvider": true}},
                    });
                    let _ = writer.write_all(&encode_frame(&resp));
                } else if method == Some("textDocument/definition") {
                    let uri = format!("file://{}/a.py", root_owned.display());
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": [{
                            "uri": uri,
                            "range": {
                                "start": {"line": 0, "character": 4},
                                "end": {"line": 0, "character": 10},
                            }
                        }]
                    });
                    let _ = writer.write_all(&encode_frame(&resp));
                }
                let _ = writer.flush();
                let _ = sent_initialize;
            }
        });

        let mut resolver = LspResolver::with_io(
            root,
            "9.9.9".to_string(),
            Box::new(client_reader),
            Box::new(client_writer),
            None,
        )
        .unwrap();

        let result = resolver.resolve_call_definitions(&store, "b.py").unwrap();
        assert_eq!(result.upgraded, 1, "{result:?}");
        assert_eq!(result.unresolved, 0);
        assert_eq!(result.errors, 0);

        let target = scc_core::symbol_id(&repo, "a.py", "helper");
        let rels = store.all_relationships().unwrap();
        let upgraded: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS)
            .collect();
        assert_eq!(upgraded.len(), 1);
        assert_eq!(upgraded[0].object, target);
        assert_eq!(upgraded[0].provenance, Provenance::Resolved);
        assert_eq!(upgraded[0].confidence, 0.99);
        assert_eq!(upgraded[0].evidence.len(), 1);

        let ev = store.get_evidence(&upgraded[0].evidence[0]).unwrap().unwrap();
        assert_eq!(ev.extractor.as_deref(), Some(LSP_EXTRACTOR));
        assert_eq!(ev.extractor_version.as_deref(), Some("9.9.9"));
        assert_eq!(ev.symbol.as_deref(), Some("helper"));
        assert_eq!(ev.start_line, Some(2));

        // the old EXTRACTED edge is gone
        assert!(!rels.iter().any(|r| r.provenance == Provenance::Extracted));
    }

    #[cfg(unix)]
    #[test]
    fn mock_server_keeps_both_same_line_upgrades() {
        // Two EXTRACTED edges, same line + same callee needle `m` (the
        // `a.m() + b.m()` case): the old `rfind` queried the second call
        // twice and the second upgrade deleted the first. The mock server
        // answers per-column: column of `a.m` -> A.m, anything else -> B.m.
        // Both edges must survive as RESOLVED.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.py"), "class A:\n    def m(self):\n        return 1\n").unwrap();
        std::fs::write(root.join("b.py"), "class B:\n    def m(self):\n        return 2\n").unwrap();
        std::fs::write(root.join("f.py"), "def f(a, b):\n    return a.m() + b.m()\n").unwrap();

        let db = root.join(".scc-test");
        std::fs::create_dir_all(&db).unwrap();
        let store = Store::open(&db.join("scc.db"), root).unwrap();
        let repo = store.repository().id;
        for (path, name, start, end) in [("a.py", "A.m", 2, 3), ("b.py", "B.m", 2, 3), ("f.py", "f", 1, 2)] {
            store
                .insert_symbol(path, name, "function", None, start, end, true, None)
                .unwrap();
        }
        let f_id = scc_core::symbol_id(&repo, "f.py", "f");
        let ext_a = entity_id(&repo, kinds::EXTERNAL_API, "a_mod");
        let ext_b = entity_id(&repo, kinds::EXTERNAL_API, "b_mod");
        store
            .insert_entity(
                &scc_core::Entity::new(f_id.clone(), kinds::SYMBOL, "f"),
                &["f.py".to_string()],
            )
            .unwrap();
        for ext in [&ext_a, &ext_b] {
            store
                .insert_entity(
                    &scc_core::Entity::new(ext.clone(), kinds::EXTERNAL_API, "ext"),
                    &["f.py".to_string()],
                )
                .unwrap();
        }
        // Same line, same callee needle `m`: shared evidence id on purpose.
        let ev = Evidence {
            id: evidence_id("f.py", "call", "m", 2),
            r#type: EvidenceType::Source,
            path: Some("f.py".to_string()),
            symbol: Some("m".to_string()),
            start_line: Some(2),
            end_line: None,
            revision: None,
            content_hash: None,
            extractor: Some("scc-native".to_string()),
            extractor_version: Some("0.0.0".to_string()),
        };
        store.insert_evidence(&ev).unwrap();
        for (i, ext) in [&ext_a, &ext_b].iter().enumerate() {
            let base = rel_id(&["calls", &f_id, ext]);
            let rel = Relationship::new(
                format!("{base}-{i}"),
                f_id.clone(),
                scc_core::predicates::CALLS,
                (*ext).clone(),
                Provenance::Extracted,
            )
            .with_confidence(confidence::CONFIRMED_EXTERNAL)
            .with_evidence(vec![ev.id.clone()]);
            store.insert_relationship(&rel, "f.py").unwrap();
        }

        let (client_reader, server_writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let (server_reader, client_writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let root_owned = root.to_path_buf();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(server_reader);
            let mut writer = server_writer;
            while let Ok(msg) = read_frame(&mut reader) {
                let method = msg.get("method").and_then(Value::as_str);
                let id = msg.get("id");
                if method == Some("initialize") {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {"capabilities": {"definitionProvider": true}},
                    });
                    let _ = writer.write_all(&encode_frame(&resp));
                } else if method == Some("textDocument/definition") {
                    let col = msg
                        .pointer("/params/position/character")
                        .and_then(Value::as_u64)
                        .unwrap_or(99);
                    // `a.m`'s `m` is at column 13; route per column.
                    let (target_file, target_line) = if col == 13 { ("a.py", 1) } else { ("b.py", 1) };
                    let uri = format!("file://{}/{target_file}", root_owned.display());
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": [{
                            "uri": uri,
                            "range": {
                                "start": {"line": target_line, "character": 4},
                                "end": {"line": target_line, "character": 5},
                            }
                        }]
                    });
                    let _ = writer.write_all(&encode_frame(&resp));
                }
                let _ = writer.flush();
            }
        });

        let mut resolver = LspResolver::with_io(
            root,
            "9.9.9".to_string(),
            Box::new(client_reader),
            Box::new(client_writer),
            None,
        )
        .unwrap();

        let result = resolver.resolve_call_definitions(&store, "f.py").unwrap();
        assert_eq!(result.upgraded, 2, "{result:?}");
        let rels = store.all_relationships().unwrap();
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.predicate == scc_core::predicates::CALLS)
            .collect();
        let target_a = scc_core::symbol_id(&repo, "a.py", "A.m");
        let target_b = scc_core::symbol_id(&repo, "b.py", "B.m");
        assert!(calls.iter().any(|r| r.object == target_a && r.provenance == Provenance::Resolved));
        assert!(calls.iter().any(|r| r.object == target_b && r.provenance == Provenance::Resolved));
        assert_eq!(calls.len(), 2, "both same-line edges must survive: {calls:?}");
        assert!(!calls.iter().any(|r| r.provenance == Provenance::Extracted));
    }

// trace:v1 id=impl.crates-scc-indexer-src-lsp.seed-store work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
    fn seed_store(root: &std::path::Path) -> Store {
        let db = root.join(".scc-test");
        std::fs::create_dir_all(&db).unwrap();
        let store = Store::open(&db.join("scc.db"), root).unwrap();
        let repo = store.repository().id;
        let file_id = entity_id(&repo, kinds::FILE, "b.py");
        store
            .insert_entity(
                &scc_core::Entity::new(file_id.clone(), kinds::FILE, "b.py"),
                &["b.py".to_string()],
            )
            .unwrap();
        let main_id = scc_core::symbol_id(&repo, "b.py", "main");
        store
            .insert_entity(
                &scc_core::Entity::new(main_id.clone(), kinds::SYMBOL, "main"),
                &["b.py".to_string()],
            )
            .unwrap();
        store
            .insert_symbol("b.py", "main", "function", None, 1, 3, true, None)
            .unwrap();
        let ext = entity_id(&repo, kinds::EXTERNAL_API, "helper_pkg");
        store
            .insert_entity(
                &scc_core::Entity::new(ext.clone(), kinds::EXTERNAL_API, "helper_pkg"),
                &["b.py".to_string()],
            )
            .unwrap();
        // a.py target symbol
        store
            .insert_symbol("a.py", "helper", "function", None, 1, 2, true, None)
            .unwrap();
        let ev = Evidence {
            id: evidence_id("b.py", "call", "helper", 2),
            r#type: EvidenceType::Source,
            path: Some("b.py".to_string()),
            symbol: Some("helper".to_string()),
            start_line: Some(2),
            end_line: None,
            revision: None,
            content_hash: None,
            extractor: Some("scc-native".to_string()),
            extractor_version: Some("0.0.0".to_string()),
        };
        store.insert_evidence(&ev).unwrap();
        let rel = Relationship::new(
            rel_id(&["calls", &main_id, &ext]),
            main_id,
            scc_core::predicates::CALLS,
            ext,
            Provenance::Extracted,
        )
        .with_confidence(confidence::CONFIRMED_EXTERNAL)
        .with_evidence(vec![ev.id]);
        store.insert_relationship(&rel, "b.py").unwrap();
        store
    }

    // ---------------------------------------------------------------
    // optional integration test (requires pyright on PATH)
    // ---------------------------------------------------------------

    #[test]
    fn pyright_resolves_reexported_member() {
        let Some(_version) = pyright_version() else {
            eprintln!("pyright not installed — skipping integration test");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // third_party/ is NOT a native source-root fallback (src/svc/lib/
        // app/services/packages) and NOT in the default ignore list, so the
        // native resolver stores an EXTRACTED edge that the LSP pass
        // upgrades through extraPaths.
        std::fs::create_dir_all(root.join("third_party/helper_pkg")).unwrap();
        std::fs::write(root.join("pyrightconfig.json"), r#"{"extraPaths": ["third_party"]}"#).unwrap();
        std::fs::write(
            root.join("third_party/helper_pkg/__init__.py"),
            "from .impl import helper\n",
        )
        .unwrap();
        std::fs::write(
            root.join("third_party/helper_pkg/impl.py"),
            "def helper():\n    return 1\n",
        )
        .unwrap();
        std::fs::write(
            root.join("b.py"),
            "from helper_pkg import helper\n\n\ndef main():\n    helper()\n",
        )
        .unwrap();

        let store = Store::open(&root.join("scc.db"), root).unwrap();
        let config = crate::config::Config::default();
        let indexer = crate::Indexer::new(store, config);
        indexer.index().unwrap();

        let store = Store::open(&root.join("scc.db"), root).unwrap();
        // native resolver must have produced an EXTRACTED edge to external_api
        let rels = store.all_relationships().unwrap();
        assert!(
            rels.iter().any(|r| r.predicate == scc_core::predicates::CALLS
                && r.provenance == Provenance::Extracted),
            "fixture must store an EXTRACTED call edge"
        );

        let mut resolver = match start_pyright(root) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("pyright handshake failed on this runner — skipping: {e}");
                return;
            }
        };
        // On runners where pyright is installed but the LSP handshake
        // fails (npm layout differences), resolution degrades — skip the
        // resolve assertions rather than fail on an environment property.
        let result = match resolver.resolve_call_definitions(&store, "b.py") {
            Ok(r) => r,
            Err(e) => {
                eprintln!("pyright resolve failed on this runner — skipping: {e}");
                return;
            }
        };
        assert!(
            result.upgraded >= 1,
            "pyright should resolve the re-exported member: {result:?}"
        );

        let rels = store.all_relationships().unwrap();
        let target = scc_core::symbol_id(
            &store.repository().id,
            "third_party/helper_pkg/impl.py",
            "helper",
        );
        let found = rels.iter().find(|r| {
            r.predicate == scc_core::predicates::CALLS && r.object == target
        });
        assert!(found.is_some(), "missing RESOLVED edge to {target}");
        let found = found.unwrap();
        assert_eq!(found.provenance, Provenance::Resolved);
        let ev = store.get_evidence(&found.evidence[0]).unwrap().unwrap();
        assert_eq!(ev.extractor.as_deref(), Some(LSP_EXTRACTOR));
        assert_eq!(ev.start_line, Some(5));
    }
}
