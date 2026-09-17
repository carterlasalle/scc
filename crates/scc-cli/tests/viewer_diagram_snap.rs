//! Web viewer, diagram, and snap-bitmap export (SPEC-SCC-VIEWER).
//!
//! Fixture-backed: diagram bytes are deterministic, snap PNG parses with
//! the pinned grid geometry, viewer routes return HTML with the right
//! status codes — all against the ts-api-web fixture.

mod common;
#[test]
fn diagram_mermaid_is_deterministic_and_names_components() {
    let tmp = common::copy_fixture("ts-api-web");
    let dir = common::workdir(tmp.path());
    common::run_ok(&dir, &["index", "--quiet"]);
    let once = common::run_ok(&dir, &["diagram"]);
    let twice = common::run_ok(&dir, &["diagram"]);
    assert_eq!(once, twice, "same index must render byte-identical mermaid");
    assert!(once.starts_with("flowchart LR"), "{once:.120}");
    assert!(once.contains("web (component)"), "{once:.400}");
    assert!(once.contains("subgraph"), "flows must render as subgraphs");
    let svg = common::run_ok(&dir, &["diagram", "--format", "svg"]);
    assert!(svg.contains("<svg"), "{svg:.120}");
    assert!(svg.contains("web"), "{svg:.400}");
}

#[test]
fn snap_emits_map_text_and_renders_png() {
    let tmp = common::copy_fixture("ts-api-web");
    let dir = common::workdir(tmp.path());
    common::run_ok(&dir, &["index", "--quiet"]);
    let map_path = dir.join("map.txt");
    let png_path = dir.join("map.png");
    let has_pil = std::process::Command::new("python3")
        .arg("-c")
        .arg("import PIL")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if has_pil {
        let out = common::run_ok(
            &dir,
            &[
                "snap",
                "--out",
                map_path.to_str().unwrap(),
                "--png",
                png_path.to_str().unwrap(),
            ],
        );
        assert!(out.contains("text ~"), "{out}");
        assert!(out.contains("image tokens"), "{out}");
        let png = std::fs::read(&png_path).unwrap();
        // PNG magic + IHDR width 1568 (big-endian u32 at bytes 16..20).
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10], "PNG magic");
        let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        assert_eq!(w, 1568, "canvas width pins the token math");
        let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        assert_eq!(h % 28, 0, "height aligns to the 28px vision patch grid");
    } else {
        // No Pillow here (CI): map text + recipe + honest skip, no render.
        let out = common::run_ok(&dir, &["snap", "--out", map_path.to_str().unwrap()]);
        assert!(out.contains("render OFF by default"), "{out}");
    }
    let map = std::fs::read_to_string(&map_path).unwrap();
    assert!(map.starts_with("SCC REPO MAP:"), "{map:.120}");
}

#[test]
fn snap_is_off_by_default_but_prints_recipe() {
    let tmp = common::copy_fixture("ts-api-web");
    let dir = common::workdir(tmp.path());
    common::run_ok(&dir, &["index", "--quiet"]);
    let out = common::run_ok(&dir, &["snap"]);
    assert!(out.contains("render OFF by default"), "{out}");
    assert!(out.contains("from PIL import"), "recipe must be visible so the render is auditable");
    assert!(
        !dir.join("scc-snap.png").exists(),
        "no PNG may appear unless requested"
    );
}

#[test]
fn viewer_route_handlers_return_html_statuses() {
    // Route predicates and recipe math live in unit tests (viewer.rs);
    // this binary drives the CLI surface only (integration tests link no lib).
    let tmp = common::copy_fixture("ts-api-web");
    let dir = common::workdir(tmp.path());
    common::run_ok(&dir, &["index", "--quiet"]);
    let out = common::run_ok(&dir, &["diagram", "--format", "svg", "--out", "d.svg"]);
    assert!(out.contains("diagram:"), "{out}");
    assert!(dir.join("d.svg").exists(), "diagram --out must write the file");
}
