use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use tysel_manifest::Manifest;

pub fn run(manifest_path: &Path) -> Result<()> {
    let manifest = Manifest::from_path(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let root = manifest_path.parent().unwrap_or(Path::new("."));
    let entry = root.join(&manifest.app.entry);
    if !entry.is_file() {
        return Err(anyhow!("entry not found: {}", entry.display()));
    }
    let is_component = entry.extension().and_then(|extension| extension.to_str()) == Some("wasm");
    let (artifact_kind, artifact_bytes, types) = if is_component {
        let source =
            std::fs::read(&entry).with_context(|| format!("failed to read {}", entry.display()))?;
        tysel_build::validate_component_for_manifest(&manifest, &source)
            .with_context(|| format!("failed to validate Component {}", entry.display()))?;
        ("component", source.len(), Typecheck::Skipped("Wasm Component"))
    } else {
        let (bundle, _map) = tysel_build::read_bundle(&entry)
            .with_context(|| format!("failed to bundle {}", entry.display()))?;
        ("bundle", bundle.len(), typecheck(root))
    };
    print!("{}", manifest.inspect_report());
    println!("check");
    println!("  manifest  ok");
    println!("  {artifact_kind:<10}ok ({artifact_bytes} bytes)");
    match &types {
        Typecheck::Ok => println!("  types     ok"),
        Typecheck::Skipped(reason) => println!("  types     skipped ({reason})"),
        Typecheck::Failed(_) => println!("  types     fail"),
    }
    if let Typecheck::Failed(output) = types {
        eprint!("{output}");
        return Err(anyhow!("TypeScript check failed"));
    }
    if !is_component {
        let specifiers = crate::node_scan::scan_file(&entry)?;
        if specifiers.is_empty() {
            println!("  node      ok");
        } else {
            println!("  node      fail");
            for specifier in &specifiers {
                eprintln!("unsupported Node builtin '{specifier}'");
            }
            return Err(anyhow!("Node builtins are not available in Tysel"));
        }
    }
    Ok(())
}

pub(crate) enum Typecheck {
    Ok,
    Skipped(&'static str),
    Failed(String),
}

pub(crate) fn typecheck(root: &Path) -> Typecheck {
    let tysel_config = root.join("tsconfig.tysel.json");
    let tsconfig = if tysel_config.is_file() { tysel_config } else { root.join("tsconfig.json") };
    if !tsconfig.is_file() {
        return Typecheck::Skipped("no tsconfig.tysel.json or tsconfig.json");
    }
    let Some(tsc) = find_tsc(root) else {
        return Typecheck::Skipped("typescript not found");
    };
    let output = Command::new(&tsc).args(["--noEmit", "-p"]).arg(&tsconfig).output();
    match output {
        Ok(output) if output.status.success() => Typecheck::Ok,
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            Typecheck::Failed(text)
        }
        Err(_) => Typecheck::Skipped("typescript not found"),
    }
}

fn find_tsc(start: &Path) -> Option<PathBuf> {
    let mut dir = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    loop {
        let mut candidate = dir.join("node_modules/typescript/bin/tsc");
        if cfg!(windows) {
            candidate.set_extension("cmd");
            if candidate.is_file() {
                return Some(candidate);
            }
            candidate.set_extension("");
        }
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}
