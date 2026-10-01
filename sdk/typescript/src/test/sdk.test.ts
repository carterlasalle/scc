/**
 * SDK integration tests against a real `scc` binary and a throwaway fixture
 * repository. Skipped when no `scc` binary is available (via $SCC_BIN or PATH).
 */

import { test, after } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { SCC } from "../index";

function resolveSccBin(): string | null {
  const fromEnv = process.env.SCC_BIN;
  if (fromEnv) return fromEnv;
  const probe = spawnSync("scc", ["--version"], { stdio: "ignore" });
  return probe.status === 0 ? "scc" : null;
}

const sccBin = resolveSccBin();
const skip = sccBin === null;
const skipReason = "scc binary not found (set SCC_BIN or add scc to PATH)";

const fixtureDir = skip ? null : mkdtempSync(join(tmpdir(), "scc-sdk-ts-"));

if (!skip) {
  const gitDir = join(fixtureDir!, ".git");
  mkdirSync(gitDir);
  writeFileSync(join(fixtureDir!, "a.py"), [
    "def add(a, b):",
    "    return a + b",
    "",
    "class Calculator:",
    "    def multiply(self, x, y):",
    "        return x * y",
    "",
  ].join("\n"));
  writeFileSync(join(fixtureDir!, "b.py"), [
    "from a import add, Calculator",
    "",
    "result = add(1, 2)",
    "calc = Calculator()",
    "prod = calc.multiply(3, 4)",
    "",
  ].join("\n"));
}

after(() => {
  for (const c of clients) c.close();
  if (fixtureDir) rmSync(fixtureDir, { recursive: true, force: true });
});

const clients: SCC[] = [];
function scc(cwd?: string): SCC {
  const c = new SCC({ bin: sccBin ?? undefined, cwd: cwd ?? fixtureDir ?? undefined });
  clients.push(c);
  return c;
}

test("index() builds the index and reports ok", { skip: skip ? skipReason : false }, async () => {
  const result = await scc().index();
  assert.deepEqual(result, { ok: true });
});

test("systemOverview() content identifies the repository", { skip: skip ? skipReason : false }, async () => {
  const pack = await scc().systemOverview();
  assert.equal(pack.kind, "overview");
  assert.match(pack.content, /IDENTITY/);
  assert.ok(Array.isArray(pack.entity_ids));
});

test("taskContext() returns the complete artifact with a nested pack", { skip: skip ? skipReason : false }, async () => {
  const artifact = await scc().taskContext("transcript");
  assert.equal(artifact.pack.kind, "task");
  assert.ok(Array.isArray(artifact.pack.entity_ids));
  assert.match(artifact.pack.content, /Goal: transcript/);
  assert.equal(typeof artifact.delta, "string");
  assert.ok(Array.isArray(artifact.delta_ids));
  assert.equal(typeof artifact.token_count, "number");
});

test("taskContext() honors files/symbols/tokenBudget options", { skip: skip ? skipReason : false }, async () => {
  const artifact = await scc().taskContext("add numbers", {
    files: ["a.py", "b.py"],
    symbols: ["add"],
    tokenBudget: 500,
  });
  assert.match(artifact.pack.content, /Explicit files: a\.py, b\.py/);
  assert.match(artifact.pack.content, /Explicit symbols: add/);
});

test("componentContext() resolves a component", { skip: skip ? skipReason : false }, async () => {
  const pack = await scc().componentContext("root");
  assert.equal(pack.kind, "component");
  assert.ok(pack.entity_ids.length > 0);
  assert.match(pack.content, /RESPONSIBILITY/);
});

test("flowContext() resolves a flow", { skip: skip ? skipReason : false }, async () => {
  const pack = await scc().flowContext("architecture");
  assert.equal(pack.kind, "flow");
  assert.ok(pack.entity_ids.length > 0);
  assert.match(pack.content, /STEPS/);
});

test("impactContext() returns an impact pack", { skip: skip ? skipReason : false }, async () => {
  const pack = await scc().impactContext(["a.py"], ["add"]);
  assert.equal(pack.kind, "impact");
  assert.match(pack.content, /RISK/);
});

test("verifyContext() content reports freshness", { skip: skip ? skipReason : false }, async () => {
  const pack = await scc().verifyContext();
  assert.equal(pack.kind, "verify");
  assert.match(pack.content, /FRESHNESS/);
});

test("contextStartup() returns the startup triple", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().contextStartup();
  assert.match(out.text, /# SCC SYSTEM CONTEXT/);
  assert.match(out.text, /## SYSTEM ATLAS/);
  assert.match(out.text, /## SYSTEM SURFACE MAP/);
  assert.ok(out.artifact !== undefined);
});

test("surfaceMap() returns the surface result", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().surfaceMap();
  assert.match(out.text, /SCC SYSTEM SURFACE MAP/);
  const personalized = await scc().surfaceMap("add numbers");
  assert.match(personalized.text, /task-personalized: add numbers/);
});

test("structuralSource() renders units for files and goals", { skip: skip ? skipReason : false }, async () => {
  const text = await scc().structuralSource(["a.py"]);
  assert.match(text, /source: a\.py:L/);
  const byGoal = await scc().structuralSource(undefined, "multiply calculator");
  assert.match(byGoal, /representation:/);
});

test("ranking() returns items with feature decomposition", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().ranking({ goal: "add numbers", limit: 5 });
  assert.ok(out.items.length > 0);
  const first = out.items[0];
  assert.ok(first.id);
  assert.equal(typeof first.rank, "number");
  for (const feat of ["task_ppr", "global_ppr", "lexical", "semantic", "confidence", "criticality", "change_risk", "novelty"] as const) {
    assert.equal(typeof first.features[feat], "number", `missing feature ${feat}`);
  }
  assert.ok(first.plugin_features !== undefined);
});

test("rankingSeeds() returns the seed merge", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().rankingSeeds("add numbers");
  assert.ok(out.seeds.length > 0);
  for (const seed of out.seeds) {
    assert.ok(seed.id && seed.kind && typeof seed.weight === "number");
  }
});

test("rankGraph() returns universe, edges, reference graph", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().rankGraph();
  const nodes = (out.universe as { nodes: unknown[] }).nodes;
  const edges = (out.edges as { edges: unknown[] }).edges;
  assert.ok(nodes.length > 0);
  assert.ok(edges.length > 0);
});

test("rankingFeatures() returns feature rows", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().rankingFeatures({ goal: "add numbers", limit: 5 });
  assert.ok(out.features.length > 0);
  for (const feat of ["task_ppr", "global_ppr", "lexical", "semantic", "confidence", "criticality", "change_risk", "novelty"] as const) {
    assert.equal(typeof out.features[0][feat], "number", `missing feature ${feat}`);
  }
});

test("rankingTrace() returns items plus seed/required inputs", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().rankingTrace({ goal: "add numbers", limit: 5 });
  assert.ok(out.items.length > 0);
  assert.ok(Array.isArray(out.seeds) && Array.isArray(out.required));
});

test("pagerankVectors() returns global and task vectors", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().pagerankVectors("add numbers");
  assert.ok(out.global.vector.length > 0);
  assert.ok(out.task.vector.length > 0);
});

test("rankingCandidates() returns scored entities", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().rankingCandidates("add numbers", 5);
  assert.ok(out.candidates.length > 0);
  const first = out.candidates[0];
  assert.ok(first.id && first.kind && typeof first.score === "number" && first.reason !== undefined);
});

test("rankingEntry() returns the Surface object", { skip: skip ? skipReason : false }, async () => {
  const syms = await scc().ranking({ goal: "add numbers", limit: 5 });
  assert.ok(syms.items.length > 0);
  const entry = await scc().rankingEntry(syms.items[0].id) as Record<string, unknown>;
  assert.ok("id" in entry);
  assert.ok("source_signature" in entry);
  assert.ok("canonical_signature" in entry);
  assert.ok("rank" in entry);
});

test("selectionOptimize() picks by value density", { skip: skip ? skipReason : false }, async () => {
  const rows = [
    { id: "a", value: 3.0, token_cost: 10 },
    { id: "b", value: 2.0, token_cost: 10 },
    { id: "c", value: 1.0, token_cost: 10 },
  ];
  const out = await scc().selectionOptimize(rows);
  assert.deepEqual(new Set(out.selected), new Set(["a", "b", "c"]));
});

test("selectionQuotas() caps per-kind fractions", { skip: skip ? skipReason : false }, async () => {
  const rows = [
    { id: "a", value: 3.0, token_cost: 10, kind: "symbol" },
    { id: "b", value: 2.0, token_cost: 10, kind: "symbol" },
    { id: "c", value: 1.0, token_cost: 10, kind: "test" },
  ];
  const out = await scc().selectionQuotas(rows);
  assert.deepEqual(new Set(out.selected), new Set(["a", "b", "c"]));
});

test("selectionMmr() selects diverse ids", { skip: skip ? skipReason : false }, async () => {
  const rows = [
    { id: "a", value: 3.0, group: "x" },
    { id: "b", value: 2.0, group: "x" },
    { id: "c", value: 1.0, group: "y" },
  ];
  const out = await scc().selectionMmr(rows);
  assert.deepEqual(new Set(out.selected), new Set(["a", "b", "c"]));
});

test("projectSymbols() maps a universe vector to symbols", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().projectSymbols({ source: "global" });
  assert.ok(Array.isArray(out.symbols));
});

test("selectionRequired() returns the never-omit set", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().selectionRequired("add numbers");
  assert.ok(Array.isArray(out.required));
});

test("selectionPreview() shows per-stage survivors", { skip: skip ? skipReason : false }, async () => {
  const rows = [
    { id: "a", value: 3.0, token_cost: 10, kind: "symbol" },
    { id: "b", value: 2.0, token_cost: 10, kind: "symbol" },
    { id: "c", value: 1.0, token_cost: 10, kind: "symbol" },
  ];
  const out = await scc().selectionPreview(rows, { budget: 100 });
  assert.deepEqual(new Set(out.after_budget), new Set(["a", "b", "c"]));
});

test("explainRanking() returns the item audit", { skip: skip ? skipReason : false }, async () => {
  const ranked = await scc().ranking({ goal: "add numbers", limit: 5 });
  const firstId = ranked.items[0].id;
  const item = await scc().explainRanking(firstId, "add numbers");
  assert.equal(item.id, firstId);
  assert.equal(typeof item.rank, "number");
  assert.ok(item.features && item.plugin_features !== undefined);
});

test("operations() lists the registry via RPC", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().operations();
  assert.ok(out.operations.length > 50);
  assert.ok(out.operations.includes("context.task"));
  assert.ok(out.operations.includes("sidecar.put"));
  assert.ok(out.operations.includes("plugins.promote"));
  assert.ok(out.operations.includes("viewer.panels"));
});

test("taskContext({recordVisibility:false}) skips the ledger", { skip: skip ? skipReason : false }, async () => {
  const { mkdtempSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const dir = mkdtempSync(join(tmpdir(), "scc-sdk-vis-"));
  mkdirSync(join(dir, ".git"));
  writeFileSync(join(dir, "a.py"), "def add(a, b):\n    return a + b\n");
  const client = new SCC({ bin: sccBin ?? undefined, cwd: dir });
  clients.push(client);
  await client.index();
  try {
    const first = await client.taskContext("add numbers", { recordVisibility: false });
    assert.ok(first.delta_ids.length > 0);
    const second = await client.taskContext("add numbers", { recordVisibility: false });
    assert.deepEqual(first.delta_ids, second.delta_ids);
    const third = await client.taskContext("add numbers");
    assert.deepEqual(first.delta_ids, third.delta_ids);
    const fourth = await client.taskContext("add numbers");
    assert.ok(fourth.delta_ids.length < third.delta_ids.length);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("non-zero scc exit rejects with stderr", { skip: skip ? skipReason : false }, async () => {
  // A fake binary that fails with a distinctive stderr message.
  const fakeBin = join(fixtureDir!, "fake-scc");
  writeFileSync(fakeBin, "#!/bin/sh\necho 'boom: exploded' >&2\nexit 3\n");
  chmodSync(fakeBin, 0o755);
  const client = new SCC({ bin: fakeBin, cwd: fixtureDir! });
  await assert.rejects(() => client.systemOverview(), /boom: exploded/);
});
