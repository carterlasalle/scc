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
    return this.invoke<TaskContextArtifact>("context.task", {
      goal,
      files: opts?.files ?? [],
      symbols: opts?.symbols ?? [],
      budget: opts?.tokenBudget,
      hook: false,
    });
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
