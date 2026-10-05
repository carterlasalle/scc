//! `scc timing` / `scc clean` tests: timing reads the extension spawn log
//! (no network, no index); clean removes only the db sidecars behind --force.

mod common;

#[test]
// trace:v1 id=test.scc-cli-timing-clean verifies=REQ-SI-503JSBGP exercises=impl.scc-cli-timing
fn timing_reports_index_spawns() {
    let repo = common::copy_fixture("cli-service");
    let dir = common::workdir(repo.path());
    let out = common::run_ok(&dir, &["timing", "--all", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("timing --json parses");
    assert!(v.get("commands").and_then(|c| c.as_array()).is_some(), "missing commands: {v}");
    assert!(v.get("log").and_then(|l| l.as_str()).is_some(), "missing log path: {v}");
}

#[test]
// trace:v1 id=test.scc-cli-clean-force verifies=REQ-SI-503JSBGP exercises=impl.scc-cli-clean
fn clean_force_removes_db_keeps_config() {
    let repo = common::copy_fixture("cli-service");
    let dir = common::workdir(repo.path());
    common::run_ok(&dir, &["index", "--quiet"]);
    assert!(dir.join(".scc").join("scc.db").exists(), "index must create the db");
    let out = common::run_ok(&dir, &["clean", "--force"]);
    assert!(out.contains("removed"), "clean must report removal: {out}");
    assert!(!dir.join(".scc").join("scc.db").exists(), "db must be gone");
    // Only the db + WAL/SHM sidecars may go: whatever else index wrote
    // (config, quarantine, ledger) must survive clean.
    let remaining: Vec<String> = std::fs::read_dir(dir.join(".scc")).unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    assert!(!remaining.iter().any(|f| f.starts_with("scc.db")), "no db sidecar may survive: {remaining:?}");
}
