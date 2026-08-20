use std::path::PathBuf;

use tysel_engine_wasm::{CompiledComponent, ComponentEngineConfig, WasmComponentEngine};

fn compile_fixture(variable: &str) -> Option<(WasmComponentEngine, CompiledComponent)> {
    let mut path = std::env::var_os(variable).map(PathBuf::from)?;
    if path.is_relative() {
        path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(path);
    }
    let source = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    let engine = WasmComponentEngine::new(ComponentEngineConfig::default()).unwrap();
    let component = engine.compile(&source).unwrap();
    Some((engine, component))
}

#[test]
fn rust_sdk_fixture_matches_the_host_contract() {
    let Some((engine, component)) = compile_fixture("TYSEL_RUST_COMPONENT_FIXTURE") else {
        return;
    };
    assert!(component.required_imports().is_empty());
    assert!(component.wasi_runtime_imports().is_empty());
    assert_eq!(
        engine.invoke_json(&component, r#"{"value":{"language":"rust"}}"#).unwrap(),
        r#"{"value":{"language":"rust"}}"#
    );
}

#[test]
fn go_sdk_fixture_matches_the_host_contract_with_restricted_wasi() {
    let Some((engine, component)) = compile_fixture("TYSEL_GO_COMPONENT_FIXTURE") else {
        return;
    };
    assert!(component.required_imports().is_empty());
    assert!(!component.wasi_runtime_imports().is_empty());
    assert_eq!(
        engine.invoke_json(&component, r#"{"value":{"language":"go"}}"#).unwrap(),
        r#"{"value":{"language":"go"}}"#
    );
}
