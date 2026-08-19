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
    let (bundle, _map) = tysel_build::read_bundle(&entry)
        .with_context(|| format!("failed to bundle {}", entry.display()))?;
    let types = typecheck(root);
    print!("{}", manifest.inspect_report());
    println!("check");
    println!("  manifest  ok");
    println!("  bundle    ok ({} bytes)", bundle.len());
    match &types {
        Typecheck::Ok => println!("  types     ok"),
        Typecheck::Skipped(reason) => println!("  types     skipped ({reason})"),
        Typecheck::Failed(_) => println!("  types     fail"),
    }
    if let Typecheck::Failed(output) = types {
        eprint!("{output}");
        return Err(anyhow!("TypeScript check failed"));
    }
    Ok(())
}

enum Typecheck {
    Ok,
    Skipped(&'static str),
    Failed(String),
}

fn typecheck(root: &Path) -> Typecheck {
    let tsconfig = root.join("tsconfig.json");
    if !tsconfig.is_file() {
        return Typecheck::Skipped("no tsconfig.json");
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
