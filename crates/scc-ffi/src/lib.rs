//! Minimal stable C ABI over [`scc_engine::invoke`].
//!
//! Three functions, JSON over raw pointers — every FFI-capable language
//! embeds SCC without a per-language native ABI. Ownership: returned
//! strings are heap-allocated via `CString::into_raw` and MUST be released
//! with [`scc_string_free`]. Input pointers are borrowed for the call only.
//! NULL inputs yield a JSON error string (never NULL, never UB).

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

// trace:exempt reason=internal-detail
fn to_json_string(value: &serde_json::Value) -> *mut c_char {
    match CString::new(value.to_string()) {
        Ok(s) => s.into_raw(),
        Err(_) => CString::new(r#"{"error":"output contained NUL"}"#)
            .unwrap()
            .into_raw(),
    }
}

// trace:exempt reason=internal-detail
fn c_str(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string()) }
}

/// Invoke any registered engine operation as JSON.
/// Returns heap JSON the caller frees with [`scc_string_free`]:
/// success `{"output": ...}`, failure `{"error": "..."}`.
#[no_mangle]
// trace:v1 id=impl.scc-ffi.invoke work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub extern "C" fn scc_invoke_json(
    root: *const c_char,
    operation: *const c_char,
    input_json: *const c_char,
) -> *mut c_char {
    let Some(root) = c_str(root) else {
        return to_json_string(&serde_json::json!({"error": "NULL root"}));
    };
    let Some(operation) = c_str(operation) else {
        return to_json_string(&serde_json::json!({"error": "NULL operation"}));
    };
    let input: serde_json::Value = match c_str(input_json) {
        None => serde_json::json!({}),
        Some(s) if s.trim().is_empty() => serde_json::json!({}),
        Some(s) => match serde_json::from_str(&s) {
            Ok(v) => v,
            Err(e) => return to_json_string(&serde_json::json!({"error": format!("invalid input JSON: {e}")})),
        },
    };
    match scc_engine::invoke(std::path::Path::new(&root), &operation, input) {
        Ok(output) => to_json_string(&serde_json::json!({"output": output})),
        Err(e) => to_json_string(&serde_json::json!({"error": e.to_string()})),
    }
}

/// Release a string returned by [`scc_invoke_json`] or [`scc_operations_json`].
///
/// # Safety
///
/// `s` must be a pointer returned by this crate (or null); anything else is
/// undefined behavior. The trace marker stays directly above the item per
/// the adjacency rule.
#[no_mangle]
// trace:v1 id=impl.scc-ffi.free work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub unsafe extern "C" fn scc_string_free(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            drop(CString::from_raw(s));
        }
    }
}

/// The operation registry as JSON. Same ownership as [`scc_invoke_json`].
#[no_mangle]
// trace:v1 id=impl.scc-ffi.operations work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
pub extern "C" fn scc_operations_json() -> *mut c_char {
    let ops: Vec<serde_json::Value> = scc_engine::OPERATIONS
        .iter()
        .map(|d| {
            serde_json::json!({
                "id": d.id,
                "description": d.description,
                "mutation": format!("{:?}", d.mutation),
                "streaming": d.streaming,
            })
        })
        .collect();
    to_json_string(&serde_json::json!({
        "operations": ops,
        "api_version": scc_api::API_VERSION,
    }))
}
