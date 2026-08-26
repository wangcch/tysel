use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

pub(super) fn application_name(root: &Path) -> Result<String> {
    let resolved = if root.exists() {
        fs::canonicalize(root).with_context(|| format!("resolve {}", root.display()))?
    } else if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir().context("resolve current directory")?.join(root)
    };
    resolved
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("cannot derive an application name from {}", root.display()))
}

pub(super) fn normalize_entry(entry: &Path) -> Result<PathBuf> {
    let raw = entry.to_str().ok_or_else(|| anyhow!("entry must be valid UTF-8"))?;
    if raw.chars().any(char::is_control) {
        return Err(anyhow!("entry cannot contain control characters"));
    }
    #[cfg(not(windows))]
    if raw.contains('\\') {
        return Err(anyhow!("entry must use '/' as its path separator"));
    }

    let mut normalized = PathBuf::new();
    for component in entry.components() {
        match component {
            std::path::Component::Normal(value) => normalized.push(value),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(anyhow!("entry must be a project-relative path without '..'"));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(anyhow!("entry must name a project-relative file"));
    }
    Ok(normalized)
}

pub(super) fn ensure_safe_destination(root: &Path, destination: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("resolve project root {}", root.display()))?;
    let mut existing_parent = destination.parent().unwrap_or(root);
    while !existing_parent.exists() {
        existing_parent = existing_parent.parent().ok_or_else(|| {
            anyhow!("cannot resolve parent directory for {}", destination.display())
        })?;
    }
    let canonical_parent = fs::canonicalize(existing_parent)
        .with_context(|| format!("resolve destination parent {}", existing_parent.display()))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(anyhow!(
            "refusing to create {} through a path outside project root {}",
            destination.display(),
            canonical_root.display()
        ));
    }
    Ok(())
}

pub(super) fn package_declares_dependency(path: &Path, name: &str) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let Ok(package) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    ["dependencies", "devDependencies", "optionalDependencies", "peerDependencies"]
        .iter()
        .any(|section| package[*section].get(name).is_some())
}

pub(super) fn package_with_tysel_scripts(
    path: &Path,
    include_tests: bool,
    typed_entry: bool,
) -> Result<Option<(Vec<u8>, String)>> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut package: serde_json::Value =
        serde_json::from_slice(&bytes).with_context(|| format!("invalid {}", path.display()))?;
    let object = package
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} must contain a JSON object", path.display()))?;
    let scripts = object
        .entry("scripts")
        .or_insert_with(|| serde_json::Value::Object(Default::default()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("scripts in {} must be a JSON object", path.display()))?;
    let mut desired = vec![
        ("tysel:dev", "tysel dev"),
        (
            "tysel:check",
            if typed_entry { "tysel types --check && tysel check" } else { "tysel check" },
        ),
        ("tysel:build", "tysel build --release"),
    ];
    if include_tests {
        desired.push(("tysel:test", "tysel test"));
    }
    let mut changed = false;
    for (name, command) in desired {
        match scripts.get(name) {
            Some(value) if value.as_str() == Some(command) => {}
            Some(_) => {
                return Err(anyhow!(
                    "refusing to replace existing package script {name:?} in {}",
                    path.display()
                ));
            }
            None => {
                scripts.insert(name.into(), command.into());
                changed = true;
            }
        }
    }
    if !changed {
        return Ok(None);
    }
    let mut rendered = serde_json::to_string_pretty(&package)?;
    rendered.push('\n');
    Ok(Some((bytes, rendered)))
}

pub(super) fn is_application_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|ch| ch.is_ascii_alphanumeric())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}
