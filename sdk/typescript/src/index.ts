/**
 * @scc/sdk — structured TypeScript SDK for the SCC engine (`scc rpc --stdio`).
 *
 * One persistent `scc rpc` child per `SCC` instance; every method speaks the
 * operation registry (no CLI-text scraping). The binary is resolved from the
 * `bin` option, then the `SCC_BIN` environment variable, then `scc` on PATH.
 * `invoke()` reaches every registered operation, including plugin operations.
 */

import { spawn, type ChildProcess } from "node:child_process";

// trace:v1 id=impl.scc.sdk.typescript work=WORK-SCC-014 satisfies=REQ-SCC-IR

/** A compiled context pack emitted by the scc CLI. */
// trace:v1 id=impl.sdk-typescript-src-index.context-pack work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
export interface ContextPack {
  kind: string;
  repository_revision: string;
  content: string;
  entity_ids: string[];
  evidence_summary: Record<string, number>;
  warnings: string[];
  tokens: number;
  budget: number;
  truncated: boolean;
}

// trace:v1 id=impl.sdk-typescript-src-index.scc-options work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
export interface SCCOptions {
  /** Path to the scc binary (default: $SCC_BIN or `scc` on PATH). */
  bin?: string;
  /** Repository root passed as `--root` (default: process.cwd()). */
  cwd?: string;
}

// trace:v1 id=impl.sdk-typescript-src-index.task-context-options work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
export interface TaskContextOptions {
  files?: string[];
  symbols?: string[];
  tokenBudget?: number;
  /** Spec §77: false = inspect without display; skips the ledger write. */
  recordVisibility?: boolean;
}

// trace:v1 id=impl.sdk-typescript-src-index.task-context-artifact work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
export interface TaskContextArtifact {
  /** The enriched task pack (`context task --json` → field `pack`). */
  // (fields documented inline below)
  pack: ContextPack;
  /** The task-personalized Surface delta (new relevant APIs vs the ledger). */
  delta: string;
  /** Entry ids the delta rendered (ledger recording). */
  delta_ids: string[];
  /** Actual token count of the complete rendered artifact (pack + delta). */
  token_count: number;
}

/** Per-symbol feature decomposition behind one ranked entry (plus plugin features). */
// trace:v1 id=impl.sdk-typescript-src-index.rank-features work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
export interface RankFeatures {
  task_ppr: number; global_ppr: number; lexical: number; semantic: number;
  confidence: number; criticality: number; change_risk: number; novelty: number;
}

/** One ranked symbol: blended score, position, decomposition, plugin features. */
// trace:v1 id=impl.sdk-typescript-src-index.rank-item work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
export interface RankItem {
  id: string; rank: number; position: number; features: RankFeatures;
  specificity: number; reasons: string[]; plugin_features: Record<string, number>;
}

/** Verbatim `ranking.symbols` envelope: items plus omission diagnostics. */
// trace:v1 id=impl.sdk-typescript-src-index.rank-result work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
export interface RankResult {
  items: RankItem[]; omitted_ids: string[]; warnings: string[];
}

/** Result of `scc index`. */
// trace:v1 id=impl.sdk-typescript-src-index.index-result work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
export interface IndexResult {
  ok: boolean;
}

/**
 * Client for the scc CLI. Each method runs the binary as a subprocess and
 * resolves with the parsed JSON result; a non-zero exit rejects with an
 * Error carrying the process's stderr.
 */
// trace:v1 id=impl.sdk-typescript-src-index-scc work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
export class SCC {
  // trace:v1 id=impl.sdk-typescript-src-index-scc.constructor work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  constructor(private opts: SCCOptions = {}) {}

  /** Resolve the scc binary: explicit option, then $SCC_BIN, then PATH. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.bin work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  private get bin(): string {
    return this.opts.bin ?? process.env.SCC_BIN ?? "scc";
  }

  // trace:v1 id=impl.sdk-typescript-src-index-scc.cwd work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  private get cwd(): string {
    return this.opts.cwd ?? process.cwd();
  }

  private rpcProc: ChildProcess | null = null;
  private rpcId = 1;
  private rpcQueue: Promise<unknown> = Promise.resolve();

  /** Lazily spawn the persistent `scc rpc --stdio` child. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.rpc work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  private rpc(): ChildProcess {
    if (!this.rpcProc) {
      const proc = spawn(this.bin, ["rpc", "--stdio"], {
        cwd: this.cwd,
        stdio: ["pipe", "pipe", "pipe"],
      });
      // Accumulate stderr for failure messages (the fake-bin test asserts
      // the child's stderr surfaces on early exit).
      const buf: string[] = [];
      proc.stderr?.on("data", (chunk: Buffer) => {
        buf.push(chunk.toString());
      });
      (proc as unknown as { stderrBuf: string[] }).stderrBuf = buf;
      // Don't hold the event loop: one-shot callers (and test runners)
      // must exit without an explicit close().
      proc.unref();
      proc.on("error", () => {
        this.rpcProc = null;
      });
      this.rpcProc = proc;
    }
    return this.rpcProc;
  }

  /** Terminate the RPC child. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.close work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  close(): void {
    this.rpcProc?.kill();
    this.rpcProc = null;
  }

  /**
   * Call any registered engine operation (including plugin operations).
   * Resolves with the operation's structured `output` verbatim.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.invoke work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async invoke<T = unknown>(operation: string, input: Record<string, unknown> = {}): Promise<T> {
    // Serialize requests: one in-flight frame per process (line-delimited).
    // trace:exempt reason=internal-detail
    const run = async (): Promise<T> => {
      const proc = this.rpc();
      const id = this.rpcId++;
      const frame = JSON.stringify({ id, operation, input }) + "\n";
      const stdout = proc.stdout!;
      const line: string = await new Promise((resolve, reject) => {
        let buf = "";
        // trace:exempt reason=internal-detail
        const onData = (chunk: Buffer) => {
          buf += chunk.toString();
          const nl = buf.indexOf("\n");
          if (nl >= 0) {
            cleanup();
            resolve(buf.slice(0, nl));
          }
        };
        // trace:exempt reason=internal-detail
        const onError = (err: Error) => {
          cleanup();
          reject(new Error(`failed to spawn ${this.bin}: ${err.message}`));
        };
        // trace:exempt reason=internal-detail
        const onClose = () => {
          cleanup();
          const err = ((proc as unknown as { stderrBuf?: string[] }).stderrBuf ?? []).join("");
          reject(new Error(err.trim() || `${this.bin} rpc exited`));
        };
        // trace:exempt reason=internal-detail
        const cleanup = () => {
          stdout.off("data", onData);
          proc.off("error", onError);
          proc.off("close", onClose);
        };
        stdout.on("data", onData);
        proc.on("error", onError);
        proc.on("close", onClose);
        proc.stdin!.write(frame, (err) => {
          if (err) {
            cleanup();
            reject(new Error(`${this.bin} rpc write failed: ${err.message}`));
          }
        });
      });
      let msg: { id: number; output?: T; error?: string };
      try {
        msg = JSON.parse(line) as typeof msg;
      } catch {
        throw new Error(`${this.bin} rpc invalid JSON: ${line.slice(0, 200)}`);
      }
      if (msg.id !== id) throw new Error(`${this.bin} rpc id mismatch`);
      if (msg.error !== undefined) throw new Error(msg.error);
      return msg.output as T;
    };
    // Chain onto the queue so concurrent invoke() calls serialize frames.
    const chained = this.rpcQueue.then(run, run);
    this.rpcQueue = chained.then(() => undefined, () => undefined);
    return chained;
  }

  /** Compile the system overview capsule. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.system-overview work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async systemOverview(): Promise<ContextPack> {
    return this.invoke<ContextPack>("context.overview", {});
  }

  /**
   * Compile the complete task context artifact for a goal: the enriched
   * task pack plus the task-personalized Surface delta.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.task-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async taskContext(goal: string, opts?: TaskContextOptions): Promise<TaskContextArtifact> {
    const input: Record<string, unknown> = {
      goal,
      files: opts?.files ?? [],
      symbols: opts?.symbols ?? [],
      budget: opts?.tokenBudget,
      hook: false,
    };
    // Omit when unset: the engine field is non-nullable bool with a
    // server-side default; an explicit null fails deserialization.
    if (opts?.recordVisibility !== undefined) input.record_visibility = opts.recordVisibility;
    return this.invoke<TaskContextArtifact>("context.task", input);
  }

  /** Compile the context pack for one component (by id or name). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.component-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async componentContext(id: string): Promise<ContextPack> {
    return this.invoke<ContextPack>("context.component", { id });
  }

  /** Compile the context pack for one flow (by id or name). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.flow-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async flowContext(id: string): Promise<ContextPack> {
    return this.invoke<ContextPack>("context.flow", { id });
  }

  /** Compile an impact analysis pack for a set of files/symbols. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.impact-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async impactContext(files?: string[], symbols?: string[]): Promise<ContextPack> {
    return this.invoke<ContextPack>("context.impact", {
      files: files ?? [],
      symbols: symbols ?? [],
    });
  }

  /** Run the freshness/evidence verification (structured pack, via RPC). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.verify-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async verifyContext(): Promise<ContextPack> {
    return this.invoke<ContextPack>("context.verify", {});
  }

  /** Compile the fused session-startup artifact (startup triple, via RPC). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.context-startup work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async contextStartup(budget?: number): Promise<{ text: string; budget: unknown; artifact: unknown }> {
    return this.invoke("context.startup", { budget });
  }

  /** Compile the System Surface Map, global or task-personalized (via RPC). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.surface-map work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async surfaceMap(goal?: string, budget?: number): Promise<{ text: string; result: unknown }> {
    return this.invoke("surface.build", { task: goal ?? null, budget, explain: false });
  }

  /**
   * Rank symbols for a goal through the full blend (task/global PPR,
   * lexical/semantic, confidence, criticality, change risk, novelty,
   * plus any active plugin features). Returns the RankResult envelope
   * verbatim: `{items: [{id, rank, position, features, specificity,
   * reasons, plugin_features}], omitted_ids, warnings}`.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.ranking-symbols work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async ranking(args: { goal?: string; limit?: number; explain?: boolean; profile?: string }): Promise<RankResult> {
    return this.invoke("ranking.symbols", {
      goal: args.goal ?? null, limit: args.limit ?? 50,
      explain: args.explain ?? false, profile: args.profile ?? null,
    });
  }

  /**
   * Explain one ranked symbol (§19/§96): the RankItem audit — blended
   * score, position, 8-feature decomposition, specificity, reasons, and
   * plugin_features. Same item shape as ranking() items.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.ranking-explain work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async explainRanking(id: string, goal?: string): Promise<RankItem> {
    return this.invoke("ranking.explain", { id, goal: goal ?? null });
  }

  /**
   * Task-seed introspection (§123.11): lexical seeds merged with plugin
   * seed providers (weight sums by id) — the same merge ranking.symbols
   * consumes. Read-only stage view.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.ranking-seeds work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async rankingSeeds(goal?: string): Promise<{ seeds: Array<{ id: string; kind: string; weight: number }> }> {
    return this.invoke("ranking.seeds", { goal: goal ?? null });
  }

  /**
   * Rank-universe stage views (§123.8/12): universe nodes, rank edges
   * (pre-aggregation weights), and the reference graph — the structure
   * the PPR vectors diffuse over. Three read-only invokes.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.ranking-graph work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async rankGraph(): Promise<{ universe: unknown; edges: unknown; reference_graph: unknown }> {
    const [universe, edges, reference_graph] = await Promise.all([
      this.invoke("ranking.universe", {}),
      this.invoke("ranking.edges", {}),
      this.invoke("ranking.reference_graph", {}),
    ]);
    return { universe, edges, reference_graph };
  }

  /**
   * Compile the Structural Source representation of files: pass `files`
   * explicitly, or a `goal` to select the task-matched files via the
   * PPR->Surface pipeline (via RPC).
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.structural-source work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async structuralSource(files?: string[], goal?: string, budget?: number): Promise<string> {
    const out = await this.invoke<string | { text: string }>("context.structural", {
      files: files ?? [], task: goal ?? null, budget,
    });
    // Engine returns {"text": ...}; unwrap to the rendered string.
    if (typeof out === "object" && out !== null && "text" in out) return out.text;
    return out as string;
  }

  /** Multi-step graph traversal (§16): dir out|in|both, optional predicate + where_kind. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.traverse work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async traverse(args: { kind?: string; name?: string; from_ids?: string[]; steps?: Array<{ dir: string; predicate?: string; where_kind?: string; limit?: number }>; limit?: number; trusted_only?: boolean }): Promise<{ entities: unknown[]; relationships: unknown[]; trusted_only: boolean }> {
    return this.invoke("graph.traverse", {
      kind: args.kind ?? null, name: args.name ?? null,
      from_ids: args.from_ids ?? [], steps: args.steps ?? [], limit: args.limit ?? 100, trusted_only: args.trusted_only ?? true,
    });
  }

  /** Start a fluent traversal: scc.query({kind, name}).out(...).execute(). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.query-builder work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  query(args?: { kind?: string; name?: string }): GraphQuery {
    return new GraphQuery(this, args?.kind, args?.name);
  }

  /** Overlay diagnostics: every assertion behind one edge plus the trusted verdict. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.explain work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async explain(subject: string, predicate: string, object: string): Promise<unknown> {
    return this.invoke("graph.explain", { subject, predicate, object });
  }

  /** JSON Schema for one operation's input (from request types). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.schema work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async operationSchema(id: string): Promise<unknown> {
    return this.invoke("operations.schema", { id });
  }

  /** Capability vocabulary: permissions, extension points, versions. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.capabilities work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async capabilities(): Promise<unknown> {
    return this.invoke("operations.capabilities", {});
  }

  /** Deterministic extension order per type. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.plugin-graph work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async pluginGraph(): Promise<{ groups: Record<string, unknown[]> }> {
    return this.invoke("plugins.graph", {});
  }

  /** Write one raw sidecar fact (never authoritative; promote explicitly). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.sidecar-put work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async sidecarPut(plugin: string, graph: string, key: string, value: unknown): Promise<unknown> {
    return this.invoke("sidecar.put", { plugin, graph, key, value });
  }

  /** Read one raw sidecar fact. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.sidecar-get work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async sidecarGet(plugin: string, graph: string, key: string): Promise<unknown> {
    return this.invoke("sidecar.get", { plugin, graph, key });
  }

  /** Scan raw sidecar facts by prefix within one plugin graph. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.sidecar-scan work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async sidecarScan(plugin: string, graph: string, prefix = "", limit = 100): Promise<unknown> {
    return this.invoke("sidecar.scan", { plugin, graph, prefix, limit });
  }

  /** Promote selected sidecar findings to canonical relationships. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.promote work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async promote(plugin: string, assertions: unknown[]): Promise<unknown> {
    return this.invoke("plugins.promote", { plugin, assertions });
  }

  /** Plugin viewer data panels (structured title/html + provenance). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.viewer-panels work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async viewerPanels(): Promise<unknown> {
    return this.invoke("viewer.panels", {});
  }

  /** Index the repository (idempotent; incremental after the first run). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.index work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async index(): Promise<IndexResult> {
    await this.invoke("index.full", {});
    return { ok: true };
  }

  /** List registered engine operations (introspection, via RPC). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.operations work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async operations(): Promise<{ operations: string[]; api_version: string }> {
    return this.invoke("operations.list", {});
  }
}

/** Fluent builder for graph.traverse: collects steps, executes on demand. */
// trace:v1 id=impl.sdk-typescript-src-index-graph-query work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
export class GraphQuery {
  private steps: Array<{ dir: string; predicate?: string; where_kind?: string; limit?: number }> = [];
  constructor(private scc: SCC, private kind?: string, private name?: string) {}
  out(predicate?: string, where_kind?: string): this { this.steps.push({ dir: "out", predicate, where_kind }); return this; }
  in_(predicate?: string, where_kind?: string): this { this.steps.push({ dir: "in", predicate, where_kind }); return this; }
  both(predicate?: string, where_kind?: string): this { this.steps.push({ dir: "both", predicate, where_kind }); return this; }
  execute(limit = 100): Promise<{ entities: unknown[]; relationships: unknown[] }> {
    return this.scc.traverse({ kind: this.kind, name: this.name, steps: this.steps, limit });
  }
}
