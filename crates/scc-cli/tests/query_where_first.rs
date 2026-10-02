//! Issue #13: `scc query` must lead with defining file:line.
//! The correct answer previously landed at rank 23-28 (below a ~20-line
//! entity block); a `— where —` header now renders first so the top of
//! the output answers "where is X defined".
mod common;
use common::*;
#[test]
// trace:v1 id=test.query-where-first-ordering work=WORK-SI-Z1KJWXDQ satisfies=REQ-SI-503JSBGP exercises=impl.query-where-first-header
fn query_leads_with_where_header() {
    let repo = copy_fixture("http-service-python");
    run_ok(&workdir(repo.path()), &["index", "--quiet"]);
    let out = run_ok(&workdir(repo.path()), &["query", "Normalizer"]);
    let where_pos = out.find("— where —").expect("where header present: {out}");
    let entities_pos = out.find("— entities —").expect("entities block present: {out}");
    let symbols_pos = out.find("— symbols —").expect("symbols block present: {out}");
    assert!(where_pos < entities_pos && where_pos < symbols_pos, "where leads: {out}");
    assert!(out.contains("where: "), "where lines present: {out}");
    // defining site carries file:line
    let first_where = out.lines().find(|l| l.starts_with("where: ")).unwrap();
    assert!(first_where.contains(".py"), "where names the file: {first_where}");
}
