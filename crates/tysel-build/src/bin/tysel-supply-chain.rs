use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::Deserialize;
use tysel_build::{RuntimeInventory, SUPPLY_CHAIN_VERSION, SupplyChainComponent};

const ROOT_PACKAGES: &[&str] = &["tysel-cli", "tysel-isolate", "tysel-runtime"];

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<MetadataPackage>,
    resolve: Option<Resolve>,
}

#[derive(Deserialize)]
struct MetadataPackage {
    id: String,
    name: String,
    version: String,
    license: Option<String>,
    source: Option<String>,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<ResolveNode>,
}

#[derive(Deserialize)]
struct ResolveNode {
    id: String,
    deps: Vec<ResolveDependency>,
}

#[derive(Deserialize)]
struct ResolveDependency {
    pkg: String,
    dep_kinds: Vec<DependencyKind>,
}

#[derive(Deserialize)]
struct DependencyKind {
    kind: Option<String>,
}

#[derive(Deserialize)]
struct CargoLock {
    package: Vec<LockedPackage>,
}

#[derive(Deserialize)]
struct LockedPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
}

fn main() -> Result<()> {
    let check = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--check") => true,
        Some(argument) => bail!("unknown argument {argument}; expected --check"),
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let destination = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/runtime-components.json");
    let generated = generate(&root)?;
    let mut json = serde_json::to_vec_pretty(&generated)?;
    json.push(b'\n');
    if check {
        let existing = fs::read(&destination)
            .with_context(|| format!("failed to read {}", destination.display()))?;
        ensure!(
            existing == json,
            "runtime supply-chain inventory is stale; run `cargo run -p tysel-build --bin tysel-supply-chain`"
        );
        println!("verified {} production components", generated.components.len());
    } else {
        fs::write(&destination, json)
            .with_context(|| format!("failed to write {}", destination.display()))?;
        println!("generated {} production components", generated.components.len());
    }
    Ok(())
}

fn generate(root: &Path) -> Result<RuntimeInventory> {
    let lock_bytes = fs::read(root.join("Cargo.lock")).context("failed to read Cargo.lock")?;
    let lock: CargoLock =
        toml::from_str(std::str::from_utf8(&lock_bytes).context("Cargo.lock is not UTF-8")?)?;
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--format-version", "1"])
        .current_dir(root)
        .output()
        .context("failed to run cargo metadata")?;
    if !output.status.success() {
        return Err(anyhow!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let metadata: Metadata = serde_json::from_slice(&output.stdout)?;
    let resolve = metadata.resolve.context("cargo metadata did not resolve dependencies")?;
    let packages: BTreeMap<_, _> =
        metadata.packages.iter().map(|package| (package.id.as_str(), package)).collect();
    let nodes: BTreeMap<_, _> = resolve.nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut pending = VecDeque::new();
    for root_name in ROOT_PACKAGES {
        let package = metadata
            .packages
            .iter()
            .find(|package| package.name == *root_name && package.source.is_none())
            .with_context(|| format!("release root package {root_name} is missing"))?;
        pending.push_back(package.id.as_str());
    }
    let mut reachable = BTreeSet::new();
    while let Some(id) = pending.pop_front() {
        if !reachable.insert(id) {
            continue;
        }
        let node = nodes.get(id).with_context(|| format!("missing resolve node {id}"))?;
        for dependency in &node.deps {
            let production_edge =
                dependency.dep_kinds.iter().any(|kind| kind.kind.as_deref() != Some("dev"));
            if production_edge {
                pending.push_back(dependency.pkg.as_str());
            }
        }
    }
    let locked: BTreeMap<_, _> = lock
        .package
        .iter()
        .map(|package| {
            ((package.name.as_str(), package.version.as_str(), package.source.as_deref()), package)
        })
        .collect();
    let mut components = Vec::with_capacity(reachable.len());
    for id in reachable {
        let package = packages.get(id).with_context(|| format!("missing package {id}"))?;
        let license = package
            .license
            .as_deref()
            .filter(|license| !license.trim().is_empty())
            .with_context(|| {
                format!("{} {} has no SPDX license expression", package.name, package.version)
            })?
            .replace('/', " OR ");
        let source = package.source.as_deref().unwrap_or("workspace");
        let locked_package = locked
            .get(&(package.name.as_str(), package.version.as_str(), package.source.as_deref()))
            .with_context(|| {
                format!("{} {} is missing from Cargo.lock", package.name, package.version)
            })?;
        components.push(SupplyChainComponent {
            name: package.name.clone(),
            version: package.version.clone(),
            license,
            purl: format!("pkg:cargo/{}@{}", package.name, package.version),
            source: source.to_owned(),
            checksum: locked_package.checksum.clone(),
        });
    }
    components.sort_by(|left, right| left.purl.cmp(&right.purl));
    for pair in components.windows(2) {
        ensure!(pair[0].purl != pair[1].purl, "duplicate package URL {}", pair[0].purl);
    }
    Ok(RuntimeInventory {
        inventory_version: SUPPLY_CHAIN_VERSION,
        cargo_lock_sha256: tysel_package::bundle_hash(&lock_bytes),
        roots: ROOT_PACKAGES.iter().map(ToString::to_string).collect(),
        components,
    })
}
