use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use crate::node_scan;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompatKind {
    Compatible,
    Shim,
    Unsupported,
    Unknown,
}

impl CompatKind {
    fn label(self) -> &'static str {
        match self {
            Self::Compatible => "compatible",
            Self::Shim => "shim",
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
        }
    }
}

struct Finding {
    name: String,
    kind: CompatKind,
    reason: &'static str,
}

const SHIM: &[&str] = &["buffer", "path", "util", "events", "assert", "querystring"];
const COMPATIBLE: &[&str] =
    &["@standard-schema/spec", "hono", "itty-router", "typescript", "valibot", "zod"];
const UNSUPPORTED: &[(&str, &str)] = &[
    ("sharp", "Node native addon"),
    ("express", "requires ownership of a node:http server"),
    ("next", "Node SSR framework"),
    ("nuxt", "Node SSR framework"),
    ("electron", "desktop runtime"),
    ("node-gyp", "native build toolchain"),
    ("sqlite3", "Node native addon"),
    ("bcrypt", "Node native addon"),
    ("fsevents", "Node native addon"),
];

pub fn run(
    manifest_path: &Path,
    json_output: bool,
    strict: bool,
    deny_unknown: bool,
) -> Result<()> {
    let root = manifest_path.parent().unwrap_or(Path::new("."));
    let package = root.join("package.json");
    let text = fs::read_to_string(&package)
        .with_context(|| format!("failed to read {}", package.display()))?;
    let value: Value =
        serde_json::from_str(&text).with_context(|| format!("invalid {}", package.display()))?;
    let mut names = Vec::new();
    collect_deps(&value["dependencies"], &mut names);
    collect_deps(&value["devDependencies"], &mut names);
    names.sort();
    names.dedup();

    let mut findings: Vec<_> = names.into_iter().map(|name| classify(&name)).collect();
    for specifier in entry_imports(root, manifest_path)? {
        if !findings.iter().any(|finding| finding.name == specifier) {
            findings.push(classify(&specifier));
        }
    }
    findings.sort_by(|left, right| left.name.cmp(&right.name));

    if json_output {
        let rows: Vec<_> = findings
            .iter()
            .map(|finding| {
                json!({
                    "name": finding.name,
                    "status": finding.kind.label(),
                    "reason": finding.reason,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schemaVersion": 1,
                "source": package.display().to_string(),
                "summary": summary(&findings),
                "packages": rows,
            }))?
        );
    } else {
        print_human(&findings);
    }

    let unsupported =
        findings.iter().filter(|finding| finding.kind == CompatKind::Unsupported).count();
    let unknown = findings.iter().filter(|finding| finding.kind == CompatKind::Unknown).count();
    if strict && (unsupported > 0 || (deny_unknown && unknown > 0)) {
        return Err(anyhow!(
            "compatibility policy failed: {unsupported} unsupported, {unknown} unknown"
        ));
    }
    Ok(())
}

fn entry_imports(root: &Path, manifest_path: &Path) -> Result<Vec<String>> {
    let manifest = tysel_manifest::Manifest::from_path(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    node_scan::scan_file(&root.join(manifest.app.entry))
}

fn collect_deps(value: &Value, names: &mut Vec<String>) {
    let Some(map) = value.as_object() else { return };
    names.extend(map.keys().cloned());
}

fn classify(name: &str) -> Finding {
    let bare = if node_scan::is_node_builtin(name) {
        node_scan::builtin_root(name).unwrap_or(name)
    } else {
        package_root(name)
    };
    if let Some((_, reason)) = UNSUPPORTED.iter().find(|(pkg, _)| *pkg == bare) {
        return Finding { name: name.into(), kind: CompatKind::Unsupported, reason };
    }
    if SHIM.contains(&bare) {
        return Finding {
            name: name.into(),
            kind: CompatKind::Shim,
            reason: "requires a Web/Tysel shim",
        };
    }
    if node_scan::is_node_builtin(bare) {
        return Finding {
            name: name.into(),
            kind: CompatKind::Unsupported,
            reason: "Node builtin is not available in Tysel",
        };
    }
    if COMPATIBLE.contains(&bare) {
        return Finding {
            name: name.into(),
            kind: CompatKind::Compatible,
            reason: "known Web-standard or build-time package",
        };
    }
    Finding {
        name: name.into(),
        kind: CompatKind::Unknown,
        reason: "not yet present in the Tysel compatibility catalog",
    }
}

fn package_root(specifier: &str) -> &str {
    let specifier = specifier.trim();
    if specifier.starts_with('@') {
        let second_slash =
            specifier.char_indices().filter_map(|(index, ch)| (ch == '/').then_some(index)).nth(1);
        return second_slash.map_or(specifier, |index| &specifier[..index]);
    }
    specifier.split('/').next().unwrap_or(specifier)
}

fn summary(findings: &[Finding]) -> Value {
    let count = |kind| findings.iter().filter(|finding| finding.kind == kind).count();
    json!({
        "compatible": count(CompatKind::Compatible),
        "shim": count(CompatKind::Shim),
        "unsupported": count(CompatKind::Unsupported),
        "unknown": count(CompatKind::Unknown),
    })
}

fn print_human(findings: &[Finding]) {
    println!("Compatibility Report\n");
    print_group("Compatible", CompatKind::Compatible, findings);
    print_group("Requires Shim", CompatKind::Shim, findings);
    print_group("Unsupported", CompatKind::Unsupported, findings);
    print_group("Unknown", CompatKind::Unknown, findings);
}

fn print_group(title: &str, kind: CompatKind, findings: &[Finding]) {
    println!("{title}");
    let rows: Vec<_> = findings.iter().filter(|finding| finding.kind == kind).collect();
    if rows.is_empty() {
        println!("  (none)\n");
        return;
    }
    for finding in rows {
        println!("  {}", finding.name);
        println!("    reason: {}", finding.reason);
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_scoped_packages_and_subpath_imports_by_package_root() {
        assert_eq!(classify("@standard-schema/spec").kind, CompatKind::Compatible);
        assert_eq!(classify("hono/cors").kind, CompatKind::Compatible);
        assert_eq!(classify("node:fs/promises").kind, CompatKind::Unsupported);
    }

    #[test]
    fn source_scanner_and_classifier_share_the_builtin_catalog() {
        for builtin in ["diagnostics_channel", "domain", "punycode", "repl", "string_decoder"] {
            assert!(node_scan::is_node_builtin(builtin));
            assert_eq!(classify(builtin).kind, CompatKind::Unsupported);
        }
        assert_eq!(classify("node:path/posix").kind, CompatKind::Shim);
    }
}
