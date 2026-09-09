use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use tysel_manifest::Manifest;

pub fn run(manifest_path: &Path) -> Result<()> {
    run_with_options(manifest_path, false)
}

pub(crate) fn run_requiring_types(manifest_path: &Path) -> Result<()> {
    run_with_options(manifest_path, true)
}

fn run_with_options(manifest_path: &Path, require_types: bool) -> Result<()> {
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
    match types {
        Typecheck::Failed(output) => {
            eprint!("{output}");
            return Err(anyhow!("TypeScript check failed"));
        }
        Typecheck::Skipped(reason) if require_types => {
            return Err(anyhow!("TypeScript check was skipped: {reason}"));
        }
        Typecheck::Ok | Typecheck::Skipped(_) => {}
    }
    if !is_component {
        // The build resolver validates runtime imports across the entire graph.
        // Re-scanning raw TS here would incorrectly reject erased type imports.
        println!("  node      ok");
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn strict_check_rejects_a_skipped_typecheck() {
        let root = std::env::temp_dir().join(format!(
            "tysel-strict-check-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("tysel.toml"),
            "[app]\nname = \"strict-check\"\nentry = \"src/index.ts\"\n",
        )
        .unwrap();
        fs::write(
            root.join("src/index.ts"),
            "export default { fetch() { return new Response('ok'); } };\n",
        )
        .unwrap();
        fs::write(root.join("tsconfig.json"), "{}\n").unwrap();

        let error = run_requiring_types(&root.join("tysel.toml")).unwrap_err();
        assert!(error.to_string().contains("TypeScript check was skipped"));
        fs::remove_dir_all(root).unwrap();
    }
}
