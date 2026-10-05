//! `scc update` self-update tests: the command resolves through the same
//! installer mechanism `docs/INSTALL.md` documents, against a fixture
//! release layout (no network). A fixture `SCC_DOWNLOAD_BASE` with a
//! pinned --version must dry-run cleanly and print the installer's plan.

mod common;

use std::io::Write;

#[test]
// trace:v1 id=test.scc-cli-update-dry-run verifies=REQ-SI-503JSBGP exercises=impl.scc-cli-update
fn update_dry_run_against_fixture_release() {
    let dir = tempfile::TempDir::new().unwrap();
    // Fixture release layout: <base>/v9.9.9/install.sh answering --dry-run.
    let rel = dir.path().join("releases").join("download").join("v9.9.9");
    std::fs::create_dir_all(&rel).unwrap();
    let mut f = std::fs::File::create(rel.join("install.sh")).unwrap();
    f.write_all(b"#!/bin/sh\necho 'scc installer'\necho \"  release:    v9.9.9\"\necho 'dry run: nothing downloaded, nothing written'\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(rel.join("install.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let base = format!("file://{}", dir.path().join("releases").join("download").display());
    let out = std::process::Command::new(common::scc())
        .args(["update", "--version", "9.9.9", "--dry-run"])
        .current_dir(dir.path())
        .env("SCC_DOWNLOAD_BASE", &base)
        .output()
        .expect("scc binary runs");
    assert!(out.status.success(), "update --dry-run failed: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("v9.9.9"), "installer plan missing version: {stdout}");
}
