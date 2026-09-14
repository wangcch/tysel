use std::fs;
use std::path::PathBuf;

fn main() {
    let crate_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workspace = crate_dir.join("../..");
    let read = |path: PathBuf| -> toml::Value {
        println!("cargo:rerun-if-changed={}", path.display());
        fs::read_to_string(path)
            .expect("read Wasmtime dependency metadata")
            .parse()
            .expect("parse Wasmtime dependency metadata")
    };
    let root = read(workspace.join("Cargo.toml"));
    let local = read(crate_dir.join("Cargo.toml"));
    let lock = read(workspace.join("Cargo.lock"));
    let dependencies = &root["workspace"]["dependencies"];
    let version = dependencies["wasmtime"]["version"]
        .as_str()
        .and_then(|value| value.strip_prefix('='))
        .expect("AOT provenance requires an exact workspace Wasmtime version pin");
    for name in ["wasmtime", "wasmtime-wasi"] {
        assert_eq!(
            local["dependencies"][name]["workspace"].as_bool(),
            Some(true),
            "{name} must inherit the workspace dependency for AOT provenance"
        );
        assert_eq!(
            dependencies[name]["version"].as_str(),
            Some(format!("={version}").as_str()),
            "Wasmtime and WASI must use the same exact version"
        );
        let versions = lock["package"]
            .as_array()
            .expect("lockfile packages")
            .iter()
            .filter(|package| package["name"].as_str() == Some(name))
            .map(|package| package["version"].as_str().expect("locked version"))
            .collect::<Vec<_>>();
        assert_eq!(versions, [version], "{name} lockfile provenance must match the workspace pin");
    }
    println!("cargo:rustc-env=TYSEL_WASMTIME_VERSION={version}");
}
