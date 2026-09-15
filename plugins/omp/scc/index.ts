// SCC × Oh My Pi — native lifecycle integration.
//
// This file is the extension entry declared by package.json in this
// directory. `scc setup omp` installs the whole package at
// `.omp/extensions/scc/` (project scope). It registers ONE module that
// owns ALL ordering-dependent SCC lifecycle behavior via native OMP
// events — there is no cross-module or filename-ordering dependence.
//
// Lifecycle events wired here (verified against the current OMP API):
//   session_start / session_switch / session_branch / session_tree
//                          -> reset the startup-injection marker
//   before_agent_start     -> the ONLY model-visible injection point for
//                             normal turns: startup (once per branch)
//                             + task context/prompt
//   tool_call              -> snapshot dirty files before opaque tools
//   tool_result            -> post-edit `scc index --paths` (edit/write
//                             AND opaque bash/patch/generator mutations);
//                             index failure is retried then reported
//   session_before_compact  -> `scc checkpoint save`
//   session.compacting      -> inject startup + checkpoint into the
//                             compaction result (`{ context: [...] }`)
//   session_compact        -> fallback reset if compacting did not run
//
// The canonical public type is `ExtensionAPI`. The runtime resolves the
// `@oh-my-pi/pi-coding-agent` package (verified in the bundled binary's
// virtual-module map — the `@oh-my-pi` scope is canonical).
import type {
  BeforeAgentStartEvent,
  ExtensionAPI,
  ExtensionContext,
  SessionCompactEvent,
  SessionStartEvent,
  ToolResultEvent,
} from "@oh-my-pi/pi-coding-agent";
import { checkCachedUpdate, logEvent } from "./update-check";

// Honor SCC_BIN (claimed by `scc setup omp`) — never hard-code "scc" as
// the only lookup. A static path may also be written into .omp/mcp.json
// at setup time when SCC_BIN is set in the installer environment.
// trace:exempt reason=const-data
const SCC_BIN = process.env.SCC_BIN || "scc";

// Subprocess budgets: every pi.exec call site passes one of these. The
// harness kills handlers at 30s WITHOUT killing their children, so an
// unbounded spawn is a pileup + corruption risk, not just slowness.
// trace:exempt reason=const-data
const FAST_MS = 5000;
// trace:exempt reason=const-data
const SNAPSHOT_MS = 5000;
// trace:exempt reason=const-data
const INDEX_MS = 20000;
// trace:exempt reason=const-data
const CONTEXT_MS = 20000;
// Hard ceiling per handler invocation: the harness kills handlers at 30s,
// so every handler aborts its own spawns at 25s and returns. Individual
// call budgets above keep single spawns small; this keeps their SUM small.
// trace:exempt reason=const-data
const HANDLER_MS = 25000;

// trace:exempt reason=internal-helper
const withDeadline = (ms = HANDLER_MS): { signal: AbortSignal; done: () => void } => {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), ms);
  return { signal: ctl.signal, done: () => clearTimeout(t) };
};

// Installed CLI version, memoized per process: `scc --version` costs a
// subprocess spawn, so it runs at most once no matter how many sessions
// start. Empty string (memoized failure) means "unknown, stay quiet".
let cachedInstalled: string | undefined;
// trace:v1 id=ops.scc.update-version work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
const installedVersion = async (
  pi: ExtensionAPI,
  cwd: string,
  signal?: AbortSignal,
): Promise<string | undefined> => {
  if (cachedInstalled === undefined) {
    const r = await scc(pi, ["--version"], cwd, FAST_MS, signal);
    const m = /scc\s+(\S+)/.exec(r.code === 0 ? r.out : "");
    cachedInstalled = m ? m[1] : "";
  }
  return cachedInstalled || undefined;
};

// Session-startup update reminder. Cache read + semver compare only — the
// only async work is the memoized version lookup. Notification goes to the
// human via ctx.ui.notify, never into model context.
// trace:v1 id=ops.scc.update-notify work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
const maybeNotifyUpdate = async (
  pi: ExtensionAPI,
  ctx: ExtensionContext,
  signal?: AbortSignal,
): Promise<void> => {
  const v = await installedVersion(pi, ctx.cwd, signal);
  if (!v) return;
  const msg = checkCachedUpdate(v);
  if (msg) {
    logEvent(ctx.cwd, "update-notify", { msg: msg.slice(0, 200) });
    ctx.ui.notify(msg, "warning");
  }
};

// Sessions/branches that already received the startup capsule this process.
// Keyed by session id PLUS the active leaf/file when available: a Set of
// session IDs is not enough when the session id stays the same but the
// active branch no longer contains the injected message (switch/branch/tree
// /resume/compaction).
const startupInjected = new Set<string>();

type DirtySnap = { head: string; files: Map<string, string> };

// toolCallId -> fingerprint map captured on tool_call for opaque tools.
// Only stored when event.toolCallId is present (no timestamp fallback).
const dirtySnapshots = new Map<string, DirtySnap>();

// True when session.compacting already rehydrated the compacted context
// so session_compact (post-notification) must not wipe the marker.
let compactingRehydrated = false;

// trace:exempt reason=internal-helper
const onEvent = (
  pi: ExtensionAPI,
  event: string,
  handler: (event: unknown, ctx: ExtensionContext) => unknown,
): void => {
  (pi.on as unknown as (type: string, handler: (event: unknown, ctx: ExtensionContext) => unknown) => void)(
    event,
    handler,
  );
};

// trace:exempt reason=internal-helper
const scc = async (
  pi: ExtensionAPI,
  args: string[],
  cwd: string,
  timeoutMs = FAST_MS,
  signal?: AbortSignal,
): Promise<{ code: number; out: string; err: string }> => {
  const t0 = Date.now();
  const cmd = [SCC_BIN, ...args].join(" ");
  try {
    const res = await pi.exec(SCC_BIN, args, { cwd, timeout: timeoutMs, signal });
    if (res.killed) {
      logEvent(cwd, "spawn", { cmd: cmd.slice(0, 500), ms: Date.now() - t0, ok: false, err: "killed on timeout" });
      return { code: -1, out: "", err: "scc killed on timeout" };
    }
    logEvent(cwd, "spawn", {
      cmd: cmd.slice(0, 500),
      ms: Date.now() - t0,
      ok: res.code === 0,
      code: res.code,
      err: res.code === 0 ? undefined : String(res.stderr ?? "").slice(0, 500),
    });
    return {
      code: res.code,
      out: String(res.stdout ?? ""),
      err: String(res.stderr ?? ""),
    };
  } catch (e) {
    logEvent(cwd, "spawn", { cmd: cmd.slice(0, 500), ms: Date.now() - t0, ok: false, err: String(e instanceof Error ? e.message : e).slice(0, 500) });
    return { code: -1, out: "", err: e instanceof Error ? e.message : String(e) };
  }
};

// trace:exempt reason=internal-helper
const execBin = async (
  pi: ExtensionAPI,
  bin: string,
  args: string[],
  cwd: string,
  timeoutMs = FAST_MS,
  signal?: AbortSignal,
): Promise<{ code: number; out: string }> => {
  const t0 = Date.now();
  try {
    const res = await pi.exec(bin, args, { cwd, timeout: timeoutMs, signal });
    const ok = !res.killed && res.code === 0;
    logEvent(cwd, "spawn", { cmd: [bin, ...args].join(" ").slice(0, 500), ms: Date.now() - t0, ok });
    if (res.killed) return { code: -1, out: "" };
    return { code: res.code, out: String(res.stdout ?? "") };
  } catch {
    return { code: -1, out: "" };
  }
};

// trace:exempt reason=internal-helper
const isConversational = (prompt: string): boolean => {
  const p = prompt.trim().toLowerCase();
  if (p === "") return true;
  return /^(hi|hey|hello|yo|thanks|thank you|ok|okay|yes|no|bye|cool|nice|good|great|sure)[.!?\s]*$/i.test(p);
};

// Read-only tools never mutate the working tree. Everything else is treated
// as potentially opaque (bash, patch, generators, formatters, scripts).
// trace:exempt reason=internal-helper
const READ_ONLY_TOOLS = new Set([
  "read",
  "grep",
  "glob",
  "find",
  "ls",
  "search",
  "semsearch",
  "websearch",
  "webfetch",
]);

// trace:exempt reason=internal-helper
const isFileMutation = (toolName: string): boolean =>
  toolName === "edit" || toolName === "write";

// trace:exempt reason=internal-helper
const isOpaqueMutation = (toolName: string): boolean =>
  !READ_ONLY_TOOLS.has(toolName) && !isFileMutation(toolName);

// trace:exempt reason=internal-helper
const editedPath = (input: Record<string, unknown>): string => {
  if (typeof input.path === "string") return input.path;
  // Current OMP edit tool: the model passes a structured edit-command
  // DSL string (`{"input": "[path#tag]\nPUT ...", "i": "..."}`). Without
  // this branch the post-edit reindex NEVER fires (verified against a
  // real 18.0.11 session log: STALE after every edit).
  const dsl = input.input;
  if (typeof dsl === "string") {
    const m = /^\[([^\]#]+)(#[^\]]+)?\]/.exec(dsl);
    if (m) return m[1];
  }
  if (typeof input.file_path === "string") return input.file_path;
  return "";
};

// Does the RESUMED conversation already carry an SCC startup capsule?
// Scans session entries for a prior scc-context custom message with
// details.hasStartup. Persisted entries carry customType/details at the
// TOP level (verified against 18.0.11 session files), not nested.
// trace:exempt reason=internal-helper
const sessionHasStartup = (ctx: ExtensionContext): boolean => {
  try {
    const entries = ctx.sessionManager.getEntries();
    if (!Array.isArray(entries)) return false;
    for (let i = entries.length - 1; i >= 0; i--) {
      const e = entries[i] as {
        type?: unknown;
        customType?: unknown;
        details?: { hasStartup?: unknown };
      };
      if (e?.type !== "custom_message" || e.customType !== "scc-context") continue;
      if (e.details?.hasStartup === true) return true;
    }
    return false;
  } catch {
    return false;
  }
};

// Composite injection key: session id stays stable across branch/tree
// navigation, so the leaf/file identity is required to know whether the
// injected startup message is still on the active branch.
// trace:exempt reason=internal-helper
const injectionKey = (ctx: ExtensionContext): string => {
  const sid = ctx.sessionManager.getSessionId();
  const sm = ctx.sessionManager as unknown as {
    getLeafId?: () => unknown;
    getSessionFile?: () => unknown;
    sessionFile?: unknown;
  };
  let leaf = "";
  try {
    const id = sm.getLeafId?.();
    if (typeof id === "string" && id) leaf = id;
  } catch {
    // fall through
  }
  if (!leaf) {
    try {
      const file = sm.getSessionFile?.() ?? sm.sessionFile;
      if (typeof file === "string" && file) leaf = file;
    } catch {
      // fall through
    }
  }
  return leaf ? `${sid}::${leaf}` : sid;
};

// trace:exempt reason=internal-helper
const resetInjection = (ctx: ExtensionContext): void => {
  const sid = ctx.sessionManager.getSessionId();
  for (const k of [...startupInjected]) {
    if (k === sid || k.startsWith(`${sid}::`)) startupInjected.delete(k);
  }
};

// NUL-delimited porcelain v1: `XY PATH\0`, and for rename/copy
// `XY DEST\0ORIG\0`. Do not split on ` -> ` — that sequence is legal inside
// a quoted pathname.
// trace:exempt reason=internal-helper
const parsePorcelainZ = (out: string): string[] => {
  const parts = out.split("\0");
  const paths: string[] = [];
  let i = 0;
  while (i < parts.length) {
    const rec = parts[i];
    if (!rec) {
      i += 1;
      continue;
    }
    if (rec.length < 3) {
      i += 1;
      continue;
    }
    const x = rec[0];
    const y = rec[1];
    const path = rec.slice(3);
    const rename = x === "R" || y === "R" || x === "C" || y === "C";
    if (path) paths.push(path);
    if (rename) {
      const orig = parts[i + 1] || "";
      if (orig) paths.push(orig);
      i += 2;
    } else {
      i += 1;
    }
  }
  return paths;
};

// Porcelain + untracked + vs-HEAD names, fingerprinted with git hash-object
// so an already-dirty file that bash mutates further is still detected.
// HEAD is recorded so a clean commit/checkout (empty dirty maps) still
// refreshes SCC.
// trace:exempt reason=internal-helper
const snapshotDirty = async (pi: ExtensionAPI, cwd: string, signal?: AbortSignal): Promise<DirtySnap> => {
  const map = new Map<string, string>();
  const headRes = await execBin(pi, "git", ["rev-parse", "HEAD"], cwd, SNAPSHOT_MS, signal);
  const head = headRes.code === 0 ? headRes.out.trim() : "";
  const status = await execBin(pi, "git", ["status", "--porcelain=v1", "-z", "-uall"], cwd, SNAPSHOT_MS, signal);
  const names = new Set<string>(parsePorcelainZ(status.out));
  const diff = await execBin(pi, "git", ["diff", "--name-only", "-z", "HEAD"], cwd, SNAPSHOT_MS, signal);
  if (diff.code === 0) {
    for (const p of diff.out.split("\0")) {
      if (p) names.add(p);
    }
  }
  for (const p of names) {
    const h = await execBin(pi, "git", ["hash-object", "--", p], cwd, SNAPSHOT_MS, signal);
    map.set(p, h.code === 0 && h.out.trim() ? h.out.trim() : "missing");
  }
  return { head, files: map };
};

// trace:exempt reason=internal-helper
const fileFingerprintDiff = (
  before: Map<string, string>,
  after: Map<string, string>,
): string[] => {
  const out: string[] = [];
  for (const [p, hash] of after) {
    if (before.get(p) !== hash) out.push(p);
  }
  for (const p of before.keys()) {
    if (!after.has(p)) out.push(p);
  }
  return out;
};

// Index failure is NEVER treated as success: retry once, then report.
// A second concurrent index run is refused outright: stacked `scc index`
// processes contend on the store lock and, killed mid-write, corrupt it.
// The skipped paths re-cover on the next mutation (freshness is eventual).
// trace:exempt reason=internal-helper
let indexInFlight = false;
// trace:exempt reason=internal-helper
const indexPaths = async (
  pi: ExtensionAPI,
  cwd: string,
  paths: string[],
  full = false,
  signal?: AbortSignal,
): Promise<{ ok: boolean; err: string }> => {
  const unique = [...new Set(paths.filter(Boolean))];
  if (!unique.length && !full) return { ok: true, err: "" };
  if (indexInFlight) {
    logEvent(cwd, "index-skip-busy", { paths: unique.length });
    return { ok: true, err: "" };
  }
  indexInFlight = true;
  const args = unique.length && !full ? ["index", "--paths", ...unique, "--quiet"] : ["index", "--quiet"];
  let r;
  try {
    r = await scc(pi, args, cwd, INDEX_MS, signal);
    if (r.code !== 0) {
      r = await scc(pi, args, cwd, INDEX_MS, signal);
    }
  } finally {
    indexInFlight = false;
  }
  if (r.code !== 0) {
    const err = `scc index --paths failed (exit ${r.code}): ${r.err || r.out || "no output"}`;
    try {
      pi.logger?.error?.(err);
    } catch {
      // logger is optional
    }
    return { ok: false, err };
  }
  return { ok: true, err: "" };
};

// trace:exempt reason=internal-helper
const loadStartupAndCheckpoint = async (
  pi: ExtensionAPI,
  cwd: string,
  signal?: AbortSignal,
): Promise<{ lines: string[]; startupOk: boolean }> => {
  const lines: string[] = [];
  const startup = await scc(pi, ["context", "startup"], cwd, CONTEXT_MS, signal);
  const startupOk = startup.code === 0 && Boolean(startup.out.trim());
  if (startupOk) lines.push(startup.out.trim());
  const checkpoint = await scc(pi, ["checkpoint", "load", "--inject"], cwd, CONTEXT_MS);
  if (checkpoint.code === 0 && checkpoint.out.trim()) lines.push(checkpoint.out.trim());
  return { lines, startupOk };
};

// trace:exempt reason=scc-installed-tooling (authoring marker from the SCC source repo removed at install)
export default function hook(pi: ExtensionAPI): void {
  const resetHandler = async (_event: unknown, ctx: ExtensionContext, signal?: AbortSignal) => {
    resetInjection(ctx);
    await scc(pi, ["state-path"], ctx.cwd, FAST_MS, signal);
  };

  // session_start: precompute/state only. This is a notification — it
  // CANNOT return model context. Reset the injection marker (a fresh
  // session, resume, or newly loaded session re-injects startup on its
  // first real prompt) and do a lightweight state-path presence check.
  pi.on("session_start", async (_event: SessionStartEvent, ctx: ExtensionContext) => {
    const dl = withDeadline();
    try {
      await resetHandler(_event, ctx, dl.signal);
      await maybeNotifyUpdate(pi, ctx, dl.signal);
    } finally {
      dl.done();
    }
  });
  onEvent(pi, "session_switch", async (_event: unknown, ctx: ExtensionContext) => {
    const dl = withDeadline();
    try {
      await resetHandler(_event, ctx, dl.signal);
      await maybeNotifyUpdate(pi, ctx, dl.signal);
    } finally {
      dl.done();
    }
  });
  onEvent(pi, "session_branch", resetHandler);
  onEvent(pi, "session_tree", resetHandler);

  // before_agent_start: THE model-visible injection point for normal turns.
  pi.on("before_agent_start", async (event: BeforeAgentStartEvent, ctx: ExtensionContext) => {
    const prompt = event.prompt;
    if (isConversational(prompt)) return;
    const dl = withDeadline();
    try {

    let content = "";
    let injectedStartup = false;
    // Startup capsule: once per branch (branch-aware key) AND skipped on
    // resumed conversations that already carry an SCC startup capsule
    // (a `-c`/`-r` resume in a fresh process has an empty Set but the
    // architecture is already in context — verified via the session
    // entry scan; re-injecting ~7k tokens would pure-duplicate it).
    const key = injectionKey(ctx);
    if (!startupInjected.has(key) && !sessionHasStartup(ctx)) {
      const startup = await scc(pi, ["context", "startup"], ctx.cwd, CONTEXT_MS, dl.signal);
      if (startup.code === 0 && startup.out.trim()) {
        content += startup.out.trim() + "\n\n";
        startupInjected.add(key);
        injectedStartup = true;
      }
    }
    if (prompt) {
      // No `--hook`: that flag is the passive Claude-hook opt-in gate
      // (silent no-op unless context.inject_task_focus=true, default
      // false) — with it, task context NEVER reaches the model (verified:
      // `--hook` prints nothing, the direct call prints the pack). This
      // extension IS the injection decision-maker; the 1500-token focus
      // budget matches hook mode's cap.
      const task = await scc(pi, ["context", "task", prompt, "--budget", "1500"], ctx.cwd, CONTEXT_MS, dl.signal);
      if (task.code === 0 && task.out.trim()) {
        content += task.out.trim();
      }
    }
    if (!content.trim()) return;
    return {
      message: {
        customType: "scc-context",
        content,
        display: false,
        details: { source: "scc", injectedAt: Date.now(), hasStartup: injectedStartup },
      },
    };
    } finally {
      dl.done();
    }
  });

  // tool_call: snapshot dirty files before opaque mutations so the post
  // hook can index whatever bash/patch/generators actually changed.
  pi.on("tool_call", async (event, ctx) => {
    if (!isOpaqueMutation(event.toolName)) return;
    const id = event.toolCallId;
    if (!id) return;
    const dl = withDeadline();
    try {
      dirtySnapshots.set(id, await snapshotDirty(pi, ctx.cwd, dl.signal));
    } finally {
      dl.done();
    }
  });

  // tool_result: post-mutation incremental refresh. edit/write index the
  // touched path; opaque tools index the dirty-fingerprint diff. A failed
  // index is retried then surfaced to the model — never silently ignored.
  pi.on("tool_result", async (event: ToolResultEvent, ctx: ExtensionContext) => {
    const dl = withDeadline();
    try {
    const paths: string[] = [];
    if (isFileMutation(event.toolName)) {
      const path = editedPath(event.input as Record<string, unknown>);
      if (path) paths.push(path);
    }
    let fullRefresh = false;
    if (isOpaqueMutation(event.toolName) || isFileMutation(event.toolName)) {
      const id = event.toolCallId;
      const before = id ? dirtySnapshots.get(id) : undefined;
      if (id) dirtySnapshots.delete(id);
      const after = await snapshotDirty(pi, ctx.cwd, dl.signal);
      if (before) {
        paths.push(...fileFingerprintDiff(before.files, after.files));
        if (before.head && after.head && before.head !== after.head) {
          const revDiff = await execBin(
            pi,
            "git",
            ["diff", "--name-only", "-z", before.head, after.head],
            ctx.cwd,
            SNAPSHOT_MS,
            dl.signal,
          );
          if (revDiff.code === 0) {
            for (const p of revDiff.out.split("\0")) {
              if (p) paths.push(p);
            }
          }
          if (!paths.length) fullRefresh = true;
        }
      } else if (isOpaqueMutation(event.toolName)) {
        paths.push(...after.files.keys());
      }
    }
    if (!paths.length && !fullRefresh) return;
    const result = await indexPaths(pi, ctx.cwd, paths, fullRefresh, dl.signal);
    if (result.ok) return;
    const content = Array.isArray(event.content) ? [...event.content] : [];
    content.push({
      type: "text",
      text: `\n\n<SCC>\n${result.err}\nPost-edit index did not succeed; subsequent task context may be stale. Re-run \`scc index --paths\`.\n</SCC>`,
    });
    return { content };
    } finally {
      dl.done();
    }
  });

  // session_before_compact: persist a checkpoint so architecture + task
  // state can be restored into the compaction result. Registered via
  // onEvent because older published typings omit this event name.
  onEvent(pi, "session_before_compact", async (_event: unknown, ctx: ExtensionContext) => {
    const dl = withDeadline();
    try {
      await scc(pi, ["checkpoint", "save"], ctx.cwd, FAST_MS, dl.signal);
    } finally {
      dl.done();
    }
  });

  // session.compacting: the compaction-result seam. Inject startup +
  // checkpoint NOW so architecture/task state survive immediately — do
  // not wait for the next user prompt. Returns { context: string[] }.
  onEvent(pi, "session.compacting", async (_event: unknown, ctx: ExtensionContext) => {
    const dl = withDeadline();
    try {
    const { lines, startupOk } = await loadStartupAndCheckpoint(pi, ctx.cwd, dl.signal);
    compactingRehydrated = lines.length > 0;
    const key = injectionKey(ctx);
    if (startupOk) {
      startupInjected.add(key);
    } else {
      startupInjected.delete(key);
    }
    if (lines.length) {
      return { context: lines };
    }
    return {};
    } finally {
      dl.done();
    }
  });

  // session_compact: post-compaction notification. If session.compacting
  // already put SCC context into the summary, keep the injection marker.
  // Otherwise clear it so the next real prompt re-injects (older OMP).
  pi.on("session_compact", (_event: SessionCompactEvent, ctx: ExtensionContext) => {
    if (!compactingRehydrated) {
      resetInjection(ctx);
    }
    compactingRehydrated = false;
  });
}
