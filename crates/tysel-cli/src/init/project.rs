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

pub(super) fn validate_new_project_root(input: &str) -> Result<PathBuf> {
    let input = input.trim();
    if input.is_empty() {
        return Err(anyhow!("project directory cannot be empty"));
    }
    if input.chars().any(char::is_control) {
        return Err(anyhow!("project directory cannot contain control characters"));
    }
    let root = PathBuf::from(input);
    if root == Path::new(".") {
        return Err(anyhow!("choose 'Add Tysel to the current directory' to use ."));
    }
    if root.exists() {
        if !root.is_dir() {
            return Err(anyhow!("project destination is not a directory"));
        }
        if fs::read_dir(&root).with_context(|| format!("read {}", root.display()))?.next().is_some()
        {
            return Err(anyhow!(
                "project destination is not empty; choose the current-directory adoption flow or another path"
            ));
        }
    }
    let name = application_name(&root)?;
    if !is_application_name(&name) {
        return Err(anyhow!(
            "project name must start with a letter or digit and use only letters, digits, '-', '_' or '.'"
        ));
    }
    Ok(root)
}

pub(super) fn validate_entry_input(root: &Path, input: &str) -> Result<PathBuf> {
    let entry = normalize_entry(Path::new(input.trim()))?;
    let destination = root.join(&entry);
    if destination.exists() && !destination.is_file() {
        return Err(anyhow!("entry is not a file: {}", destination.display()));
    }
    if !destination.is_file() {
        ensure_generated_entry_extension(&entry)?;
    }
    ensure_safe_destination(root, &destination)?;
    Ok(entry)
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

pub(super) fn ensure_generated_entry_extension(entry: &Path) -> Result<()> {
    let extension = entry.extension().and_then(|value| value.to_str());
    if matches!(extension, Some("ts" | "tsx" | "mts")) {
        return Ok(());
    }
    Err(anyhow!(
        "generated application entry must use a TypeScript extension (.ts, .tsx, or .mts): {}",
        entry.display()
    ))
}

pub(super) fn ensure_tsconfig_files(path: &Path, required: &[&Path]) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut source =
        String::from_utf8(bytes).with_context(|| format!("{} must be UTF-8", path.display()))?;
    json_strip_comments::strip(&mut source)
        .with_context(|| format!("invalid {}", path.display()))?;
    let config: serde_json::Value =
        serde_json::from_str(&source).with_context(|| format!("invalid {}", path.display()))?;
    let Some(files) = config.get("files") else {
        return Ok(());
    };
    let files =
        files.as_array().ok_or_else(|| anyhow!("files in {} must be an array", path.display()))?;
    let files = files
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(normalize_config_path)
        .collect::<Vec<_>>();
    for required in required {
        let required = normalize_config_path(&required.to_string_lossy());
        if !files.iter().any(|candidate| candidate == &required) {
            return Err(anyhow!(
                "{} does not include {required}; add it to files or remove the config and rerun init",
                path.display()
            ));
        }
    }
    Ok(())
}

fn normalize_config_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_owned()
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

pub(super) fn gitignore_with_entries(
    path: &Path,
    desired: &str,
) -> Result<Option<(Vec<u8>, String)>> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let original =
        std::str::from_utf8(&bytes).with_context(|| format!("{} must be UTF-8", path.display()))?;
    let normalized =
        original.lines().map(|line| normalize_ignore_pattern(line.trim())).collect::<Vec<_>>();
    // Negation rules are order-sensitive. Keep the schema exception block at
    // the end so an existing `.tysel/` rule cannot hide the shared schema.
    let (unordered, ordered) = desired
        .split_once("!.tysel/\n")
        .map(|(head, tail)| (head, format!("!.tysel/\n{tail}")))
        .unwrap_or((desired, String::new()));
    let append_ordered = !ordered.is_empty() && !original.trim_end().ends_with(ordered.trim_end());
    let missing = unordered
        .lines()
        .filter(|entry| {
            let entry = normalize_ignore_pattern(entry);
            !normalized.iter().any(|existing| existing == &entry)
        })
        .collect::<Vec<_>>();
    if missing.is_empty() && !append_ordered {
        return Ok(None);
    }

    let mut contents = original.to_owned();
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    if !contents.is_empty() && !contents.ends_with("\n\n") {
        contents.push('\n');
    }
    contents.push_str("# Tysel\n");
    for entry in missing {
        contents.push_str(entry);
        contents.push('\n');
    }
    if append_ordered {
        contents.push_str(&ordered);
    }
    Ok(Some((bytes, contents)))
}

fn normalize_ignore_pattern(pattern: &str) -> &str {
    pattern.trim_start_matches('/').trim_start_matches("./")
}

pub(super) fn is_application_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|ch| ch.is_ascii_alphanumeric())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}
