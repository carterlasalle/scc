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

  /**
   * FTS entity search with LIKE fallback (graph.search): raw Reality
   * Graph entities matching the query. Lexical lookup — use
   * traverse()/query() for structured multi-step traversal.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.graph-search work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async graphSearch(query: string, limit?: number): Promise<{ entities: Array<{ id: string; kind: string; name: string }> }> {
    return { entities: await this.invoke("graph.search", { query, limit: limit ?? 100 }) as Array<{ id: string; kind: string; name: string }> };
  }

  /**
   * Raw relationship facts (§15): the verbatim edge list behind
   * traverse(). Use graphEntity/explain for trust verdicts.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.graph-relationships work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async graphRelationships(): Promise<{ relationships: Array<{ subject: string; predicate: string; object: string; provenance: string; confidence: number }> }> {
    return { relationships: await this.invoke("graph.relationships", {}) as Array<{ subject: string; predicate: string; object: string; provenance: string; confidence: number }> };
  }

  /**
   * One canonical entity by id (§15) plus its trust verdict:
   * `{entity, trusted, reason}`. `entity` is the raw Reality Graph
   * node; `trusted` reports the TrustedGraphView decision.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.graph-entity work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async graphEntity(id: string): Promise<{ entity: unknown; trusted: boolean; reason: string | null }> {
    return this.invoke("graph.entity.get", { id });
  }

  /**
   * Index status, stats, and freshness (workspace.status): repo,
   * revision, freshness, stale files, entity/relationship counts.
   * The live counterpart to the pinned workspaceSession identity.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.workspace-status work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async workspaceStatus(): Promise<{ repository: string; revision: string; freshness: string; stale_count: number; stats: Record<string, number> }> {
    return this.invoke("workspace.status", {});
  }

  /**
   * Pinned model-session identity (§6): repo, revision, epoch, config
   * hash, plugin set, ranking pipeline. The anchor every result's model
   * identity refers to.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.workspace-session work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async workspaceSession(): Promise<{ repo_id: string; revision: string; epoch: string; config_hash: string }> {
    return this.invoke("workspace.session", {});
  }

  /**
   * Full model access (§14): everything SCC knows as structured state
   * — repository, entities, relationships, evidence, components, flows,
   * invariants. Not a rendered Atlas.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.model-get work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async modelGet(): Promise<unknown> {
    return this.invoke("model.get", {});
  }

  /**
   * System Atlas as its structured type (§74): components, hierarchy,
   * ownership, flows, invariants, boundaries, drift. Distinct from
   * contextStartup (fused session artifact).
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.context-atlas work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async contextAtlas(budget?: number, full?: boolean, unbounded?: boolean): Promise<unknown> {
    return this.invoke("context.atlas", { budget: budget ?? null, full: full ?? false, unbounded: unbounded ?? false });
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
   * Fast where-to-pay-attention answer (§18/§96): top Surface entries,
   * optionally scoped to a component and personalized to a task goal.
   * `tasked` reports whether the call was task-personalized.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.surface-important work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async surfaceImportant(args?: { goal?: string; limit?: number; component?: string }): Promise<{ entries: unknown[]; tasked: boolean }> {
    return this.invoke("surface.important", {
      task: args?.goal ?? null, limit: args?.limit ?? 10, component: args?.component ?? null,
    });
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
   * Feature-score introspection (§123.13): per-symbol core + plugin
   * feature decomposition before the blend — the same computation as
   * ranking.symbols, projected to feature rows.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.ranking-features work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async rankingFeatures(args: { goal?: string; limit?: number; explain?: boolean; profile?: string }): Promise<{ features: Array<{ id: string; position: number; task_ppr: number; global_ppr: number; lexical: number; semantic: number; confidence: number; criticality: number; change_risk: number; novelty: number; specificity: number; plugin_features: Record<string, number> }> }> {
    return this.invoke("ranking.features", {
      goal: args.goal ?? null, limit: args.limit ?? 50,
      explain: args.explain ?? false, profile: args.profile ?? null,
    });
  }

  /**
   * Full ranking trace (§19): RankResult items plus the seed and required
   * inputs the blend consumed. Same computation as ranking.symbols; the
   * envelope is the audit path.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.ranking-trace work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async rankingTrace(args: { goal?: string; limit?: number; explain?: boolean; profile?: string }): Promise<RankResult & { seeds: string[]; required: string[] }> {
    return this.invoke("ranking.trace", {
      goal: args.goal ?? null, limit: args.limit ?? 50,
      explain: args.explain ?? false, profile: args.profile ?? null,
    });
  }

  /**
   * Raw PPR stage vectors (§123.10): the global vector and the task
   * vector for a goal. No plugin hooks by contract — the structure the
   * symbol projection blends from.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.pagerank-vectors work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async pagerankVectors(goal?: string): Promise<{ global: { vector: Array<{ id: string; score: number }> }; task: { vector: Array<{ id: string; score: number }> } }> {
    const [global, task] = await Promise.all([
      this.invoke("ranking.pagerank.global", {}),
      this.invoke("ranking.pagerank.task", { goal: goal ?? null }),
    ]);
    return { global: global as { vector: Array<{ id: string; score: number }> }, task: task as { vector: Array<{ id: string; score: number }> } };
  }

  /**
   * Candidate-stage introspection: scored entities (id/kind/name/score/
   * reason) merged from the core Surface compiler plus plugin candidate
   * providers — the same merge ranking.symbols consumes.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.ranking-candidates work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async rankingCandidates(goal?: string, limit?: number): Promise<{ candidates: Array<{ id: string; kind: string; name: string; score: number; reason: string }> }> {
    return this.invoke("ranking.candidates", { goal: goal ?? null, limit: limit ?? 50 });
  }

  /**
   * Selection-effects introspection (§123.14): per-stage survivors
   * (after_mmr, after_quotas, after_budget) over caller-supplied rows.
   * Default math only: shows WHERE each id drops out.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.selection-preview work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async selectionPreview(ranked: Array<{ id: string; value: number; token_cost?: number; kind?: string; group?: string | null }>, opts?: { budget?: number; quotas?: Array<{ kind: string; fraction: number }>; lambda?: number }): Promise<{ after_mmr: string[]; after_quotas: string[]; after_budget: string[] }> {
    return this.invoke("selection.preview", {
      ranked, budget: opts?.budget ?? 0, quotas: opts?.quotas ?? null, lambda: opts?.lambda ?? null,
    });
  }

  /**
   * MMR/diversity stage (§124 item 24) over caller rows. Same math the
   * Surface pipeline blends through; a plugin diversity declarer
   * replaces it wholesale.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.selection-mmr work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async selectionMmr(ranked: Array<{ id: string; value: number; group?: string | null }>, lambda?: number): Promise<{ selected: string[] }> {
    return this.invoke("selection.mmr", { ranked, budget: 0, lambda: lambda ?? null });
  }

  /**
   * Quota stage (§124 item 25) over caller rows with per-kind fraction
   * caps. Rank order preserved, unknown kinds uncapped; plugin quota
   * overrides merged first (one math path).
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.selection-quotas work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async selectionQuotas(ranked: Array<{ id: string; value: number; token_cost?: number; kind?: string; group?: string | null }>, quotas?: Array<{ kind: string; fraction: number }>): Promise<{ selected: string[] }> {
    return this.invoke("selection.quotas", { ranked, budget: 0, quotas: quotas ?? null });
  }

  /**
   * One authoritative SurfaceEntry by id (§1.6/§18): identity,
   * signatures, flows, contracts, rank decomposition. The full
   * programmatic Surface object.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.ranking-entry work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async rankingEntry(id: string): Promise<unknown> {
    return this.invoke("ranking.entry", { id });
  }

  /**
   * Architectural specificity multiplier (§1.4/§45): exported/public
   * symbols score 1.15, others 1.0. Pure probe of the projection-stage
   * multiplier ranking.symbols applies.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.arch-specificity work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async architecturalSpecificity(id?: string, exported?: boolean): Promise<{ specificity: number; id: string }> {
    return this.invoke("ranking.architectural_specificity", { id: id ?? "", exported: exported ?? false });
  }

  /**
   * Full edge weight (§45/§49): predicate × provenance × confidence ×
   * rarity. Provenance is Extracted on this arm; plugin edge-weight
   * hooks alter live ranking, not this probe.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.edge-weight work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async edgeWeight(args?: { predicate?: string; confidence?: number; totalSymbols?: number; targetInDegree?: number }): Promise<{ weight: number }> {
    return this.invoke("ranking.edge_weight", {
      predicate: args?.predicate ?? "calls", confidence: args?.confidence ?? 1.0,
      total_symbols: args?.totalSymbols ?? 1, target_in_degree: args?.targetInDegree ?? 0,
    });
  }

  /**
   * Default blend (§51) over explicit feature values. Pure math, no
   * store, no hooks — the weights ranking.symbols blends from.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.final-importance work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async finalImportance(features: { task_ppr?: number; global_ppr?: number; lexical?: number; semantic?: number; confidence?: number; criticality?: number; change_risk?: number; novelty?: number }, hasTask?: boolean): Promise<{ score: number }> {
    return this.invoke("ranking.final_importance", { ...features, has_task: hasTask ?? true });
  }

  /**
   * Pure per-entry blend (§123 intermediate) over explicit feature
   * rows. No store, no hooks — the same math ranking.symbols blends
   * from, exposed for audit and for callers scoring own rows.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.score-entries work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async scoreEntries(entries: Array<{ id: string; task_ppr?: number; global_ppr?: number; lexical?: number; semantic?: number; confidence?: number; criticality?: number; change_risk?: number; novelty?: number; has_task?: boolean }>): Promise<{ scores: Array<{ id: string; score: number }> }> {
    return this.invoke("ranking.score_entries", { entries });
  }

  /**
   * Budget-optimizer stage (§124 item 27) over caller rows:
   * value-density knapsack (value/token_cost desc). At most one plugin
   * declarer replaces it with the full selected id list.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.selection-optimize work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async selectionOptimize(ranked: Array<{ id: string; value: number; token_cost?: number; kind?: string; group?: string | null }>): Promise<{ selected: string[] }> {
    return this.invoke("selection.optimize", { ranked, budget: 0 });
  }

  /**
   * Required-coverage set (§123 never-omit): engine required_ids plus
   * plugin coverage providers, unioned. Same inputs ranking.symbols
   * blends criticality from.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.selection-required work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async selectionRequired(goal?: string): Promise<{ required: string[]; plugin_contributed: number }> {
    return this.invoke("selection.required", { goal: goal ?? null });
  }

  /**
   * Projection introspection (§123 intermediate): map a universe vector
   * to per-symbol scores. Pass explicit `vector` rows, or
   * `source: "global"`, or a task `goal`. Same projection
   * ranking.symbols blends from.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.project-symbols work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
  async projectSymbols(args?: { vector?: Array<{ id: string; score: number }>; source?: string; goal?: string }): Promise<{ symbols: Array<{ id: string; score: number }> }> {
    return this.invoke("ranking.project_symbols", {
      vector: args?.vector ?? null, source: args?.source ?? null, goal: args?.goal ?? null,
    });
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
