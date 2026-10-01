"""Integration tests for the scc-sdk against a real ``scc`` binary and a
throwaway fixture repository. Skipped when no ``scc`` binary is available
(via the SCC_BIN environment variable or PATH)."""

import os
import shutil
import stat
import tempfile
import unittest
from pathlib import Path

from scc_sdk import SCC, SCCError

A_PY = """def add(a, b):
    return a + b

class Calculator:
    def multiply(self, x, y):
        return x * y
"""

B_PY = """from a import add, Calculator

result = add(1, 2)
calc = Calculator()
prod = calc.multiply(3, 4)
"""


# trace:exempt reason=unit-test
def resolve_scc_bin():
    from_env = os.environ.get("SCC_BIN")
    if from_env:
        return from_env
    return shutil.which("scc")


BIN = resolve_scc_bin()


# trace:v1 id=test.scc.sdk.python verifies=REQ-SCC-IR exercises=impl.scc.sdk.python
@unittest.skipUnless(BIN, "scc binary not found (set SCC_BIN or add scc to PATH)")
class TestSCCSDK(unittest.TestCase):
    # trace:exempt reason=unit-test
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="scc-sdk-py-")
        (Path(cls.tmp) / ".git").mkdir()
        (Path(cls.tmp) / "a.py").write_text(A_PY)
        (Path(cls.tmp) / "b.py").write_text(B_PY)
        cls.scc = SCC(bin=BIN, cwd=cls.tmp)
        cls.scc.index()

    # trace:exempt reason=unit-test
    @classmethod
    def tearDownClass(cls):
        try:
            cls.scc.close()
        except Exception:
            pass
        shutil.rmtree(cls.tmp, ignore_errors=True)

    # trace:exempt reason=unit-test
    def test_index_returns_ok(self):
        result = self.scc.index()
        self.assertEqual(result, {"ok": True})

    # trace:exempt reason=unit-test
    def test_system_overview_content_identifies_repository(self):
        pack = self.scc.systemOverview()
        self.assertEqual(pack["kind"], "overview")
        self.assertIn("IDENTITY", pack["content"])
        self.assertIsInstance(pack["entity_ids"], list)

    # trace:exempt reason=unit-test
    def test_graph_relationships_lists_edges(self):
        got = self.scc.graphRelationships()
        self.assertIn("relationships", got, f"missing rels: {got}")
        self.assertTrue(got["relationships"], f"empty rels: {got}")
        rel = got["relationships"][0]
        for key in ("subject", "predicate", "object"):
            self.assertIn(key, rel, f"missing {key}: {rel}")

    # trace:exempt reason=unit-test
    def test_workspace_session_pins_identity(self):
        sess = self.scc.workspaceSession()
        for key in ("repo_id", "revision", "epoch", "config_hash"):
            self.assertIn(key, sess, f"missing {key}: {sess}")

    # trace:exempt reason=unit-test
    def test_graph_entity_reports_trust_verdict(self):
        syms = self.scc.ranking(goal="add numbers", limit=5)
        self.assertTrue(syms["items"], f"no ranked items: {syms}")
        got = self.scc.graphEntity(syms["items"][0]["id"])
        self.assertIn("entity", got, f"missing entity: {got}")
        self.assertIn("trusted", got, f"missing trusted: {got}")
        self.assertIsNotNone(got["entity"], f"null entity: {got}")

    # trace:exempt reason=unit-test
    def test_model_get_returns_structured_state(self):
        model = self.scc.modelGet()
        self.assertIsInstance(model, dict, f"not a dict: {model}")
        self.assertTrue(model, f"empty model: {model}")

    # trace:exempt reason=unit-test
    def test_context_atlas_has_components(self):
        atlas = self.scc.contextAtlas()
        self.assertIsInstance(atlas, dict, f"not a dict: {atlas}")
        self.assertTrue(atlas, f"empty atlas: {atlas}")

    # trace:exempt reason=internal-detail  # sdk integration test; behavior traced at impl.crates-scc-cli-src-commands.build-task-context
    def test_task_context_has_entity_ids_array(self):
        artifact = self.scc.taskContext("transcript")
        self.assertEqual(artifact["pack"]["kind"], "task")
        self.assertIsInstance(artifact["pack"]["entity_ids"], list)
        self.assertIn("Goal: transcript", artifact["pack"]["content"])
        self.assertIsInstance(artifact["delta"], str)
        # delta_ids is ALWAYS serialized now (empty array, never omitted) —
        # the exact public contract the TS type declares.
        self.assertIsInstance(artifact["delta_ids"], list)
        self.assertIsInstance(artifact["token_count"], int)

    # trace:exempt reason=internal-detail  # sdk integration test; behavior traced at impl.crates-scc-cli-src-commands.build-task-context
    def test_task_context_honors_options(self):
        artifact = self.scc.taskContext(
            "add numbers", files=["a.py", "b.py"], symbols=["add"], tokenBudget=500
        )
        self.assertIn("Explicit files: a.py, b.py", artifact["pack"]["content"])
        self.assertIn("Explicit symbols: add", artifact["pack"]["content"])

    # trace:exempt reason=unit-test
    def test_component_context_resolves_component(self):
        pack = self.scc.componentContext("root")
        self.assertEqual(pack["kind"], "component")
        self.assertTrue(pack["entity_ids"])
        self.assertIn("RESPONSIBILITY", pack["content"])

    # trace:exempt reason=unit-test
    def test_flow_context_resolves_flow(self):
        pack = self.scc.flowContext("architecture")
        self.assertEqual(pack["kind"], "flow")
        self.assertTrue(pack["entity_ids"])
        self.assertIn("STEPS", pack["content"])

    # trace:exempt reason=unit-test
    def test_impact_context_returns_impact_pack(self):
        pack = self.scc.impactContext(files=["a.py"], symbols=["add"])
        self.assertEqual(pack["kind"], "impact")
        self.assertIn("RISK", pack["content"])

    # trace:exempt reason=unit-test
    def test_verify_context_content_reports_freshness(self):
        pack = self.scc.verifyContext()
        self.assertEqual(pack["kind"], "verify")
        self.assertIn("FRESHNESS", pack["content"])

    # trace:exempt reason=internal-detail  # sdk integration test; behavior traced at impl.scc.cli
    def test_context_startup_renders_fused_artifact(self):
        pack = self.scc.contextStartup()
        self.assertEqual(pack["kind"], "startup")
        self.assertIn("# SCC SYSTEM CONTEXT", pack["content"])
        self.assertIn("## SYSTEM ATLAS", pack["content"])
        self.assertIn("## SYSTEM SURFACE MAP", pack["content"])

    # trace:exempt reason=internal-detail  # sdk integration test; behavior traced at impl.scc.cli
    def test_surface_map_renders_global_and_personalized(self):
        out = self.scc.surfaceMap()
        self.assertIn("SCC SYSTEM SURFACE MAP", out["text"])
        personalized = self.scc.surfaceMap(goal="add numbers")
        self.assertIn("task-personalized: add numbers", personalized["text"])

    # trace:exempt reason=internal-detail  # sdk integration test; behavior traced at impl.scc.cli
    def test_structural_source_renders_files_and_goal(self):
        text = self.scc.structuralSource(files=["a.py"])
        self.assertIn("source: a.py:L", text)
        by_goal = self.scc.structuralSource(goal="multiply calculator")
        self.assertIn("representation:", by_goal)

    # trace:exempt reason=unit-test
    def test_ranking_returns_items_with_feature_decomposition(self):
        out = self.scc.ranking(goal="add numbers", limit=5)
        self.assertIn("items", out)
        self.assertTrue(out["items"], f"empty ranking: {out}")
        first = out["items"][0]
        for key in ("id", "rank", "position", "features", "specificity", "reasons", "plugin_features"):
            self.assertIn(key, first, f"missing {key}: {first}")
        for feat in ("task_ppr", "global_ppr", "lexical", "semantic", "confidence", "criticality", "change_risk", "novelty"):
            self.assertIn(feat, first["features"], f"missing feature {feat}: {first['features']}")

    # trace:exempt reason=unit-test
    def test_ranking_seeds_returns_seed_merge(self):
        out = self.scc.rankingSeeds(goal="add numbers")
        self.assertIn("seeds", out)
        self.assertTrue(out["seeds"], f"empty seeds: {out}")
        for seed in out["seeds"]:
            for key in ("id", "kind", "weight"):
                self.assertIn(key, seed, f"missing {key}: {seed}")

    # trace:exempt reason=unit-test
    def test_rank_graph_returns_universe_edges_reference(self):
        out = self.scc.rankGraph()
        self.assertIn("nodes", out["universe"])
        self.assertTrue(out["universe"]["nodes"], f"empty universe: {out}")
        self.assertIn("edges", out["edges"])
        self.assertTrue(out["edges"]["edges"], f"empty edges: {out}")

    # trace:exempt reason=unit-test
    def test_ranking_features_returns_feature_rows(self):
        out = self.scc.rankingFeatures(goal="add numbers", limit=5)
        self.assertIn("features", out)
        self.assertTrue(out["features"], f"empty features: {out}")
        for feat in ("task_ppr", "global_ppr", "lexical", "semantic", "confidence", "criticality", "change_risk", "novelty"):
            self.assertIn(feat, out["features"][0], f"missing feature {feat}: {out['features'][0]}")

    # trace:exempt reason=unit-test
    def test_ranking_trace_returns_items_plus_inputs(self):
        out = self.scc.rankingTrace(goal="add numbers", limit=5)
        self.assertIn("items", out)
        self.assertTrue(out["items"], f"empty trace: {out}")
        self.assertIn("seeds", out)
        self.assertIn("required", out)

    # trace:exempt reason=unit-test
    def test_pagerank_vectors_return_global_and_task(self):
        out = self.scc.pagerankVectors(goal="add numbers")
        for key in ("global", "task"):
            self.assertIn("vector", out[key], f"missing vector in {key}: {out[key]}")
            self.assertTrue(out[key]["vector"], f"empty {key} vector: {out[key]}")

    # trace:exempt reason=unit-test
    def test_ranking_candidates_returns_scored_entities(self):
        out = self.scc.rankingCandidates(goal="add numbers", limit=5)
        self.assertIn("candidates", out)
        self.assertTrue(out["candidates"], f"empty candidates: {out}")
        for key in ("id", "kind", "name", "score", "reason"):
            self.assertIn(key, out["candidates"][0], f"missing {key}: {out['candidates'][0]}")

    # trace:exempt reason=unit-test
    # trace:exempt reason=unit-test
    def test_surface_important_returns_entries(self):
        out = self.scc.surfaceImportant(goal="add numbers", limit=3)
        self.assertIn("entries", out, f"missing entries: {out}")
        self.assertTrue(out["entries"], f"empty entries: {out}")
        self.assertIn("tasked", out, f"missing tasked: {out}")

    # trace:exempt reason=unit-test
    def test_ranking_entry_returns_surface_object(self):
        syms = self.scc.ranking(goal="add numbers", limit=5)
        self.assertTrue(syms["items"], f"no ranked items: {syms}")
        entry = self.scc.rankingEntry(syms["items"][0]["id"])
        self.assertIn("id", entry, f"missing id: {entry}")
        self.assertIn("source_signature", entry, f"missing source_signature: {entry}")
        self.assertIn("canonical_signature", entry, f"missing canonical_signature: {entry}")
        self.assertIn("rank", entry, f"missing rank: {entry}")

    # trace:exempt reason=unit-test
    def test_architectural_specificity_rewards_exported(self):
        pub = self.scc.architecturalSpecificity(id="x", exported=True)
        priv = self.scc.architecturalSpecificity(id="x", exported=False)
        self.assertEqual(pub["specificity"], 1.15, f"exported: {pub}")
        self.assertEqual(priv["specificity"], 1.0, f"private: {priv}")

    # trace:exempt reason=unit-test
    def test_edge_weight_distinguishes_calls_from_imports(self):
        calls = self.scc.edgeWeight(predicate="calls", confidence=1.0)
        imports = self.scc.edgeWeight(predicate="imports", confidence=1.0)
        self.assertIn("weight", calls, f"missing weight: {calls}")
        self.assertGreater(
            calls["weight"], imports["weight"],
            f"calls should outweigh imports: {calls} vs {imports}")

    # trace:exempt reason=unit-test
    def test_final_importance_matches_score_entries(self):
        feats = {"task_ppr": 0.9, "global_ppr": 0.5, "lexical": 0.8,
                 "semantic": 0.7, "confidence": 0.9, "criticality": 1.0,
                 "change_risk": 0.4, "novelty": 0.5}
        one = self.scc.finalImportance(feats, has_task=True)
        rows = self.scc.scoreEntries([{"id": "a", **feats, "has_task": True}])
        self.assertIn("score", one, f"missing score: {one}")
        self.assertAlmostEqual(
            one["score"], rows["scores"][0]["score"], places=9,
            msg=f"blend mismatch: {one} vs {rows}")

    # trace:exempt reason=unit-test
    def test_score_entries_blends_explicit_rows(self):
        rows = [
            {"id": "a", "task_ppr": 0.9, "global_ppr": 0.5, "lexical": 0.8,
             "semantic": 0.7, "confidence": 0.9, "criticality": 1.0,
             "change_risk": 0.4, "novelty": 0.5, "has_task": True},
            {"id": "b", "task_ppr": 0.1, "global_ppr": 0.1, "lexical": 0.1,
             "semantic": 0.1, "confidence": 0.1, "criticality": 0.0,
             "change_risk": 0.0, "novelty": 0.0, "has_task": True},
        ]
        out = self.scc.scoreEntries(rows)
        self.assertIn("scores", out, f"missing scores: {out}")
        by_id = {r["id"]: r["score"] for r in out["scores"]}
        self.assertGreater(by_id["a"], by_id["b"], f"blend order wrong: {out}")

    # trace:exempt reason=unit-test
    def test_selection_optimize_picks_value_density(self):
        rows = [
            {"id": "a", "value": 3.0, "token_cost": 10},
            {"id": "b", "value": 2.0, "token_cost": 10},
            {"id": "c", "value": 1.0, "token_cost": 10},
        ]
        out = self.scc.selectionOptimize(rows)
        self.assertIn("selected", out, f"missing selected: {out}")
        self.assertEqual(set(out["selected"]), {"a", "b", "c"})

    # trace:exempt reason=unit-test
    def test_selection_quotas_caps_per_kind(self):
        rows = [
            {"id": "a", "value": 3.0, "token_cost": 10, "kind": "symbol"},
            {"id": "b", "value": 2.0, "token_cost": 10, "kind": "symbol"},
            {"id": "c", "value": 1.0, "token_cost": 10, "kind": "test"},
        ]
        out = self.scc.selectionQuotas(rows)
        self.assertIn("selected", out, f"missing selected: {out}")
        self.assertEqual(set(out["selected"]), {"a", "b", "c"})

    # trace:exempt reason=unit-test
    def test_selection_mmr_selects_diverse_ids(self):
        rows = [
            {"id": "a", "value": 3.0, "group": "x"},
            {"id": "b", "value": 2.0, "group": "x"},
            {"id": "c", "value": 1.0, "group": "y"},
        ]
        out = self.scc.selectionMmr(rows)
        self.assertIn("selected", out, f"missing selected: {out}")
        self.assertEqual(set(out["selected"]), {"a", "b", "c"})

    # trace:exempt reason=unit-test
    def test_project_symbols_maps_universe_vector(self):
        out = self.scc.projectSymbols(source="global")
        self.assertIn("symbols", out, f"missing symbols: {out}")
        self.assertIsInstance(out["symbols"], list)

    # trace:exempt reason=unit-test
    def test_selection_required_returns_never_omit_set(self):
        out = self.scc.selectionRequired(goal="add numbers")
        self.assertIn("required", out, f"missing required: {out}")
        self.assertIsInstance(out["required"], list)

    def test_selection_preview_shows_stage_survivors(self):
        rows = [
            {"id": "a", "value": 3.0, "token_cost": 10, "kind": "symbol"},
            {"id": "b", "value": 2.0, "token_cost": 10, "kind": "symbol"},
            {"id": "c", "value": 1.0, "token_cost": 10, "kind": "symbol"},
        ]
        out = self.scc.selectionPreview(rows, budget=100)
        for key in ("after_mmr", "after_quotas", "after_budget"):
            self.assertIn(key, out, f"missing {key}: {out}")
        self.assertEqual(set(out["after_budget"]), {"a", "b", "c"})

    def test_explain_ranking_returns_item_audit(self):
        ranked = self.scc.ranking(goal="add numbers", limit=5)
        first_id = ranked["items"][0]["id"]
        item = self.scc.explainRanking(first_id, goal="add numbers")
        self.assertEqual(item["id"], first_id)
        for key in ("rank", "position", "features", "specificity", "reasons", "plugin_features"):
            self.assertIn(key, item, f"missing {key}: {item}")

    # trace:exempt reason=unit-test
    # trace:exempt reason=unit-test
    def test_task_context_record_visibility_false_skips_ledger(self):
        # Fresh repo: nothing visible yet, so the delta is non-empty.
        tmp = tempfile.mkdtemp(prefix="scc-sdk-vis-")
        (Path(tmp) / ".git").mkdir()
        (Path(tmp) / "a.py").write_text(A_PY)
        vis = SCC(bin=BIN, cwd=tmp)
        vis.index()
        try:
            first = vis.taskContext("add numbers", recordVisibility=False)
            self.assertTrue(first["delta_ids"], f"expected delta ids: {first}")
            second = vis.taskContext("add numbers", recordVisibility=False)
            self.assertEqual(first["delta_ids"], second["delta_ids"],
                             "unrecorded ids resurface verbatim")
            # Recording consumes the delta: a default call then sees less.
            third = vis.taskContext("add numbers")
            self.assertEqual(first["delta_ids"], third["delta_ids"],
                             "default sees the same unrecorded ids")
            fourth = vis.taskContext("add numbers")
            self.assertTrue(
                set(fourth["delta_ids"]) < set(third["delta_ids"]),
                f"recorded ids must shrink the next delta: {third['delta_ids']} vs {fourth['delta_ids']}",
            )
        finally:
            try:
                vis.close()
            except Exception:
                pass

    def test_operations_lists_registry(self):
        out = self.scc.operations()
        self.assertIn("context.task", out["operations"])
        self.assertIn("ranking.symbols", out["operations"])
        self.assertIn("sidecar.put", out["operations"])
        self.assertIn("plugins.promote", out["operations"])
        self.assertIn("viewer.panels", out["operations"])

    # trace:exempt reason=internal-detail  # sdk parity test; behavior traced at impl.crates-scc-cli-src-commands.build-task-context
    def test_task_context_mirrors_cli_json_exactly(self):
        """Parity: the SDK's taskContext() must return the CLI's
        `scc context task --json` output verbatim (nested artifact shape,
        no flattening) — the CLI JSON, Python SDK, and TypeScript SDK all
        expose the same {pack, delta, delta_ids, token_count} contract for
        the same fixture/goal."""
        import json
        import subprocess

        goal = "transcript"
        sdk_artifact = self.scc.taskContext(goal)

        cli = subprocess.run(
            [BIN, "--root", str(self.tmp), "context", "task", goal, "--json"],
            capture_output=True,
            text=True,
            check=True,
        )
        cli_artifact = json.loads(cli.stdout)

        # The SDK MUST expose the CLI shape: top-level pack (not flattened).
        self.assertEqual(sorted(sdk_artifact.keys()), sorted(cli_artifact.keys()))
        self.assertEqual(sdk_artifact["pack"], cli_artifact["pack"])
        self.assertEqual(sdk_artifact["delta"], cli_artifact["delta"])
        if "delta_ids" in cli_artifact:
            self.assertEqual(sdk_artifact["delta_ids"], cli_artifact["delta_ids"])
        self.assertEqual(sdk_artifact["token_count"], cli_artifact["token_count"])
        # And the nested pack fields are the flat pack contract.
        self.assertEqual(sdk_artifact["pack"]["kind"], "task")
        self.assertIn("Goal: transcript", sdk_artifact["pack"]["content"])

    # trace:exempt reason=unit-test
    def test_nonzero_exit_raises_scc_error(self):
        fake_bin = Path(self.tmp) / "fake-scc"
        fake_bin.write_text("#!/bin/sh\necho 'boom: exploded' >&2\nexit 3\n")
        fake_bin.chmod(fake_bin.stat().st_mode | stat.S_IEXEC)
        client = SCC(bin=str(fake_bin), cwd=self.tmp)
        with self.assertRaises(SCCError) as ctx:
            client.systemOverview()
        self.assertIn("boom: exploded", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
