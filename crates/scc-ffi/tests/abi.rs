//! FFI contract tests: the C ABI in-process (same symbols the cdylib
//! exposes). Covers JSON round-trip, NULL-safety, and registry exposure.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

// trace:exempt reason=test-helper
fn call(root: &str, op: &str, input: &str) -> serde_json::Value {
    let r = CString::new(root).unwrap();
    let o = CString::new(op).unwrap();
    let i = CString::new(input).unwrap();
    let p = scc_ffi::scc_invoke_json(r.as_ptr(), o.as_ptr(), i.as_ptr());
    assert!(!p.is_null());
    let s = unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() };
    unsafe { scc_ffi::scc_string_free(p); }
    serde_json::from_str(&s).unwrap()
}

// trace:exempt reason=internal-detail
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.py"), "def hello():\n    return 1\n").unwrap();
    scc_engine::index::full(&root, &scc_indexer::Config::default()).unwrap();
    (dir, root)
}

#[test]
// trace:v1 id=test.scc-ffi.abi-round-trip verifies=REQ-SI-503JSBGP exercises=impl.scc-ffi.invoke
fn invoke_returns_output_envelope() {
    let (_dir, root) = fixture();
    let v = call(root.to_str().unwrap(), "workspace.status", "{}");
    assert!(v.get("output").is_some(), "{v}");
    assert!(v.get("error").is_none(), "{v}");
}

#[test]
// trace:v1 id=test.scc-ffi.null-safety verifies=REQ-SI-503JSBGP exercises=impl.scc-ffi.invoke
fn null_inputs_yield_error_json_never_null() {
    let p = scc_ffi::scc_invoke_json(std::ptr::null(), c"workspace.status".as_ptr(), c"{}".as_ptr());
    assert!(!p.is_null());
    let s = unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() };
    unsafe { scc_ffi::scc_string_free(p); }
    assert!(s.contains("error"), "{s}");
    let p = scc_ffi::scc_invoke_json(
        CString::new(".").unwrap().as_ptr(),
        std::ptr::null() as *const c_char,
        std::ptr::null(),
    );
    assert!(!p.is_null());
    unsafe { scc_ffi::scc_string_free(p); }
}

#[test]
// trace:v1 id=test.scc-ffi.registry verifies=REQ-SI-503JSBGP exercises=impl.scc-ffi.operations
fn operations_lists_registry() {
    let p = scc_ffi::scc_operations_json();
    assert!(!p.is_null());
    let s = unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() };
    unsafe { scc_ffi::scc_string_free(p); }
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert!(v["operations"].as_array().unwrap().len() >= 50, "{v}");
}
