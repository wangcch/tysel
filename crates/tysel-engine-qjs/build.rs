use std::fs;
use std::path::PathBuf;

fn required_string<'a>(value: &'a serde_json::Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("runtime compatibility is missing {pointer}"))
}

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../runtime-js/compatibility.json");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let compatibility: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).expect("read runtime compatibility"))
            .expect("parse runtime compatibility");

    let adapter_id = required_string(&compatibility, "/quickjsAdapter");
    let adapter_version = required_string(&compatibility, "/quickjs/adapter/version");
    let adapter_revision = required_string(&compatibility, "/quickjs/adapter/revision");
    let engine_version = required_string(&compatibility, "/quickjs/engine/version");
    let engine_revision = required_string(&compatibility, "/quickjs/engine/revision");
    assert!(
        adapter_revision.len() >= 7 && engine_revision.len() >= 7,
        "QuickJS provenance revisions must contain at least seven characters"
    );
    let expected_id = format!(
        "rquickjs-{adapter_version}+{}/quickjs-ng-{engine_version}+{}",
        &adapter_revision[..7],
        &engine_revision[..7]
    );
    assert_eq!(adapter_id, expected_id, "QuickJS adapter identity is inconsistent");

    println!("cargo:rustc-env=TYSEL_QUICKJS_ENGINE_VERSION={engine_version}");
    println!("cargo:rustc-env=TYSEL_QUICKJS_ADAPTER_ID={adapter_id}");
}
