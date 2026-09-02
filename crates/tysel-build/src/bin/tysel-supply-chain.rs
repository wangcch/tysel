use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::Deserialize;
use tysel_build::{RuntimeInventory, SUPPLY_CHAIN_VERSION, SupplyChainComponent};

const ROOT_PACKAGES: &[&str] = &["tysel-cli", "tysel-isolate", "tysel-runtime"];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeCompatibility {
    quickjs_adapter: String,
    quickjs: QuickJsProvenance,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuickJsProvenance {
    release_status: String,
    allowed_release_channels: Vec<String>,
    adapter: ComponentIdentity,
    engine: ComponentIdentity,
}

#[derive(Deserialize)]
struct ComponentIdentity {
    name: String,
    version: String,
    repository: String,
    revision: String,
}

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
    manifest_path: PathBuf,
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
    let mut check = false;
    let mut release_channel = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--check" => check = true,
            "--release-channel" => {
                release_channel =
                    Some(arguments.next().context("--release-channel requires a value")?);
            }
            _ => bail!("unknown argument {argument}"),
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let destination = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/runtime-components.json");
    let generated = generate(&root, release_channel.as_deref())?;
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

fn git_source(repository: &str, revision: &str) -> String {
    format!("git+{repository}?rev={revision}#{revision}")
}

fn github_purl(component: &ComponentIdentity) -> Result<String> {
    let path = component
        .repository
        .strip_prefix("https://github.com/")
        .and_then(|path| path.strip_suffix(".git"))
        .context("QuickJS engine repository must be a canonical GitHub clone URL")?;
    ensure!(
        path.split('/').count() == 2,
        "QuickJS engine repository must identify one GitHub owner and repository"
    );
    Ok(format!("pkg:github/{path}@{}", component.version))
}

fn validate_release_channel(provenance: &QuickJsProvenance, channel: Option<&str>) -> Result<()> {
    let Some(channel) = channel else {
        return Ok(());
    };
    ensure!(
        provenance.allowed_release_channels.iter().any(|allowed| allowed == channel),
        "QuickJS adapter status `{}` does not permit `{channel}` releases; allowed channels: {}",
        provenance.release_status,
        provenance.allowed_release_channels.join(", ")
    );
    Ok(())
}

fn parse_gitlink_revision<'a>(output: &'a str, path: &str) -> Result<&'a str> {
    let (metadata, actual_path) = output
        .trim()
        .split_once('\t')
        .with_context(|| format!("git tree entry for {path} is malformed"))?;
    ensure!(actual_path == path, "git tree entry resolved an unexpected path {actual_path}");
    let mut fields = metadata.split_whitespace();
    ensure!(fields.next() == Some("160000"), "{path} is not a gitlink");
    ensure!(fields.next() == Some("commit"), "{path} does not reference a commit");
    let revision = fields.next().context("gitlink entry has no revision")?;
    ensure!(fields.next().is_none(), "gitlink entry has unexpected metadata");
    ensure!(
        revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "gitlink revision is not a full hexadecimal commit"
    );
    Ok(revision)
}

fn gitlink_revision(manifest_path: &Path, adapter_revision: &str, path: &str) -> Result<String> {
    let adapter_root =
        manifest_path.parent().context("QuickJS adapter manifest has no parent directory")?;
    let output = Command::new("git")
        .args(["-C"])
        .arg(adapter_root)
        .args(["ls-tree", adapter_revision, path])
        .output()
        .context("failed to inspect the QuickJS adapter git tree")?;
    ensure!(
        output.status.success(),
        "failed to inspect QuickJS gitlink: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let stdout = std::str::from_utf8(&output.stdout).context("git ls-tree output is not UTF-8")?;
    Ok(parse_gitlink_revision(stdout, path)?.to_owned())
}

fn quickjs_component(
    components: &[SupplyChainComponent],
    compatibility: &RuntimeCompatibility,
    actual_engine_revision: &str,
) -> Result<SupplyChainComponent> {
    let adapter = &compatibility.quickjs.adapter;
    let expected_adapter_source = git_source(&adapter.repository, &adapter.revision);
    let locked_adapter = components
        .iter()
        .find(|component| component.name == adapter.name && component.version == adapter.version)
        .with_context(|| {
            format!(
                "declared QuickJS adapter {} {} is not in the production dependency graph",
                adapter.name, adapter.version
            )
        })?;
    ensure!(
        locked_adapter.source == expected_adapter_source,
        "QuickJS adapter source does not match runtime compatibility: expected {expected_adapter_source}, found {}",
        locked_adapter.source
    );
    let engine = &compatibility.quickjs.engine;
    ensure!(
        actual_engine_revision == engine.revision,
        "QuickJS engine revision does not match adapter gitlink: declared {}, found {actual_engine_revision}",
        engine.revision
    );
    ensure!(
        adapter.revision.len() >= 7 && engine.revision.len() >= 7,
        "QuickJS provenance revisions must contain at least seven characters"
    );
    let expected_identity = format!(
        "{}-{}+{}/{}-{}+{}",
        adapter.name,
        adapter.version,
        &adapter.revision[..7],
        engine.name,
        engine.version,
        &engine.revision[..7]
    );
    ensure!(
        compatibility.quickjs_adapter == expected_identity,
        "QuickJS adapter identity does not match declared provenance"
    );
    Ok(SupplyChainComponent {
        name: engine.name.clone(),
        version: engine.version.clone(),
        license: "MIT".into(),
        purl: github_purl(engine)?,
        source: git_source(&engine.repository, &engine.revision),
        checksum: None,
    })
}

fn generate(root: &Path, release_channel: Option<&str>) -> Result<RuntimeInventory> {
    let lock_bytes = fs::read(root.join("Cargo.lock")).context("failed to read Cargo.lock")?;
    let lock: CargoLock =
        toml::from_str(std::str::from_utf8(&lock_bytes).context("Cargo.lock is not UTF-8")?)?;
    let compatibility: RuntimeCompatibility = serde_json::from_slice(
        &fs::read(root.join("runtime-js/compatibility.json"))
            .context("failed to read runtime compatibility")?,
    )
    .context("failed to parse runtime compatibility")?;
    validate_release_channel(&compatibility.quickjs, release_channel)?;
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
    let adapter = &compatibility.quickjs.adapter;
    let expected_adapter_source = git_source(&adapter.repository, &adapter.revision);
    let adapter_package = metadata
        .packages
        .iter()
        .find(|package| {
            package.name == adapter.name
                && package.version == adapter.version
                && package.source.as_deref() == Some(expected_adapter_source.as_str())
        })
        .context("QuickJS adapter package metadata is missing")?;
    let actual_engine_revision =
        gitlink_revision(&adapter_package.manifest_path, &adapter.revision, "sys/quickjs")?;
    components.push(quickjs_component(&components, &compatibility, &actual_engine_revision)?);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn provenance(channels: &[&str]) -> QuickJsProvenance {
        QuickJsProvenance {
            release_status: "candidate".into(),
            allowed_release_channels: channels.iter().map(ToString::to_string).collect(),
            adapter: ComponentIdentity {
                name: "rquickjs".into(),
                version: "0.12.2".into(),
                repository: "https://example.com/rquickjs.git".into(),
                revision: "a".repeat(40),
            },
            engine: ComponentIdentity {
                name: "quickjs-ng".into(),
                version: "0.16.2".into(),
                repository: "https://example.com/quickjs.git".into(),
                revision: "b".repeat(40),
            },
        }
    }

    #[test]
    fn candidate_release_policy_rejects_stable() {
        let provenance = provenance(&["canary"]);
        assert!(validate_release_channel(&provenance, Some("canary")).is_ok());
        assert!(
            validate_release_channel(&provenance, Some("stable"))
                .unwrap_err()
                .to_string()
                .contains("does not permit `stable`")
        );
    }

    #[test]
    fn quickjs_binding_rejects_a_lockfile_revision_mismatch() {
        let provenance = provenance(&["canary"]);
        let compatibility = RuntimeCompatibility {
            quickjs_adapter: "rquickjs-0.12.2+aaaaaaa/quickjs-ng-0.16.2+bbbbbbb".into(),
            quickjs: provenance,
        };
        let components = vec![SupplyChainComponent {
            name: "rquickjs".into(),
            version: "0.12.2".into(),
            license: "MIT".into(),
            purl: "pkg:cargo/rquickjs@0.12.2".into(),
            source: git_source("https://example.com/rquickjs.git", &"c".repeat(40)),
            checksum: None,
        }];
        assert!(
            quickjs_component(&components, &compatibility, &"b".repeat(40))
                .unwrap_err()
                .to_string()
                .contains("source does not match")
        );
    }

    #[test]
    fn quickjs_binding_rejects_a_false_engine_revision() {
        let provenance = provenance(&["canary"]);
        let compatibility = RuntimeCompatibility {
            quickjs_adapter: "rquickjs-0.12.2+aaaaaaa/quickjs-ng-0.16.2+bbbbbbb".into(),
            quickjs: provenance,
        };
        let components = vec![SupplyChainComponent {
            name: "rquickjs".into(),
            version: "0.12.2".into(),
            license: "MIT".into(),
            purl: "pkg:cargo/rquickjs@0.12.2".into(),
            source: git_source("https://example.com/rquickjs.git", &"a".repeat(40)),
            checksum: None,
        }];
        assert!(
            quickjs_component(&components, &compatibility, &"c".repeat(40))
                .unwrap_err()
                .to_string()
                .contains("does not match adapter gitlink")
        );
    }

    #[test]
    fn gitlink_parser_requires_an_exact_submodule_entry() {
        let revision = "b".repeat(40);
        let entry = format!("160000 commit {revision}\tsys/quickjs\n");
        assert_eq!(parse_gitlink_revision(&entry, "sys/quickjs").unwrap(), revision);
        assert!(parse_gitlink_revision(&entry, "sys/other").is_err());
        assert!(
            parse_gitlink_revision(
                &format!("100644 blob {revision}\tsys/quickjs\n"),
                "sys/quickjs"
            )
            .is_err()
        );
    }
}
