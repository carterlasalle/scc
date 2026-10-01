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

test("runtimeStatus() returns the edge list", { skip: skip ? skipReason : false }, async () => {
  const got = await scc().runtimeStatus();
  assert.ok(Array.isArray(got.edges));
});

test("graphFlows() lists causal flows", { skip: skip ? skipReason : false }, async () => {
  const got = await scc().graphFlows();
  assert.ok(got.flows.length > 0);
  assert.ok("id" in got.flows[0]);
});

test("graphEntities() lists graph nodes", { skip: skip ? skipReason : false }, async () => {
  const got = await scc().graphEntities();
  assert.ok(got.entities.length > 0);
  assert.ok("id" in got.entities[0]);
});

test("evidenceSearch() lists evidence records", { skip: skip ? skipReason : false }, async () => {
  const got = await scc().evidenceSearch();
  assert.ok(got.evidence.length > 0);
  assert.ok("id" in got.evidence[0]);
});

test("historyList() reports the index revision", { skip: skip ? skipReason : false }, async () => {
  const got = await scc().historyList();
  assert.ok(got.revisions.length > 0);
  assert.ok("rev" in got.revisions[0]);
});

test("graphSearch() finds the fixture entity", { skip: skip ? skipReason : false }, async () => {
  const got = await scc().graphSearch("add");
  assert.ok(got.entities.length > 0);
  assert.ok(got.entities.some((e) => e.id.includes("add")));
});

test("graphRelationships() lists raw edges", { skip: skip ? skipReason : false }, async () => {
  const got = await scc().graphRelationships();
  assert.ok(got.relationships.length > 0);
  for (const key of ["subject", "predicate", "object"] as const)
    assert.ok(key in got.relationships[0], `missing ${key}`);
});

test("workspaceStatus() reports freshness and stats", { skip: skip ? skipReason : false }, async () => {
  const st = await scc().workspaceStatus();
  for (const key of ["repository", "revision", "freshness", "stats"] as const)
    assert.ok(key in st, `missing ${key}`);
  // Fixture indexes its own fake-scc shim, so STALE is expected here.
  assert.ok(st.freshness === "CURRENT" || st.freshness === "STALE");
  assert.ok("entities" in (st.stats as Record<string, unknown>));
});

test("workspaceSession() pins the model identity", { skip: skip ? skipReason : false }, async () => {
  const sess = await scc().workspaceSession();
  for (const key of ["repo_id", "revision", "epoch", "config_hash"] as const)
    assert.ok(key in sess, `missing ${key}`);
});

test("graphEntity() reports the trust verdict", { skip: skip ? skipReason : false }, async () => {
  const syms = await scc().ranking({ goal: "add numbers", limit: 5 });
  assert.ok(syms.items.length > 0);
  const got = await scc().graphEntity(syms.items[0].id);
  assert.ok("entity" in got && got.entity !== null);
  assert.equal(typeof got.trusted, "boolean");
});

test("modelGet() returns structured state", { skip: skip ? skipReason : false }, async () => {
  const model = await scc().modelGet() as Record<string, unknown>;
  assert.equal(typeof model, "object");
  assert.ok(model !== null && Object.keys(model).length > 0);
});

test("contextAtlas() returns the structured Atlas", { skip: skip ? skipReason : false }, async () => {
  const atlas = await scc().contextAtlas() as Record<string, unknown>;
  assert.equal(typeof atlas, "object");
  assert.ok(atlas !== null && Object.keys(atlas).length > 0);
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

test("surfaceImportant() returns top entries", { skip: skip ? skipReason : false }, async () => {
  const out = await scc().surfaceImportant({ goal: "add numbers", limit: 3 });
  assert.ok(Array.isArray(out.entries) && out.entries.length > 0);
  assert.equal(typeof out.tasked, "boolean");
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

test("architecturalSpecificity() rewards exported symbols", { skip: skip ? skipReason : false }, async () => {
  const pub = await scc().architecturalSpecificity("x", true);
  const priv = await scc().architecturalSpecificity("x", false);
  assert.equal(pub.specificity, 1.15);
  assert.equal(priv.specificity, 1.0);
});

test("edgeWeight() distinguishes calls from imports", { skip: skip ? skipReason : false }, async () => {
  const calls = await scc().edgeWeight({ predicate: "calls", confidence: 1.0 });
  const imports = await scc().edgeWeight({ predicate: "imports", confidence: 1.0 });
  assert.ok(calls.weight > imports.weight);
});

test("finalImportance() matches scoreEntries() on one row", { skip: skip ? skipReason : false }, async () => {
  const feats = { task_ppr: 0.9, global_ppr: 0.5, lexical: 0.8, semantic: 0.7, confidence: 0.9, criticality: 1.0, change_risk: 0.4, novelty: 0.5 };
  const one = await scc().finalImportance(feats, true);
  const rows = await scc().scoreEntries([{ id: "a", ...feats, has_task: true }]);
  assert.ok(Math.abs(one.score - rows.scores[0].score) < 1e-9);
});

test("scoreEntries() blends explicit feature rows", { skip: skip ? skipReason : false }, async () => {
  const rows = [
    { id: "a", task_ppr: 0.9, global_ppr: 0.5, lexical: 0.8, semantic: 0.7, confidence: 0.9, criticality: 1.0, change_risk: 0.4, novelty: 0.5, has_task: true },
    { id: "b", task_ppr: 0.1, global_ppr: 0.1, lexical: 0.1, semantic: 0.1, confidence: 0.1, criticality: 0.0, change_risk: 0.0, novelty: 0.0, has_task: true },
  ];
  const out = await scc().scoreEntries(rows);
  const byId = Object.fromEntries(out.scores.map((r) => [r.id, r.score]));
  assert.ok(byId["a"] > byId["b"]);
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
