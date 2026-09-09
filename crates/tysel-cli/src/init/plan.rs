use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use tysel_manifest::{Manifest, ManifestFormat};

use super::project::{
    application_name, ensure_generated_entry_extension, ensure_safe_destination,
    ensure_tsconfig_files, gitignore_with_entries, is_application_name, normalize_entry,
    package_declares_dependency, package_with_tysel_scripts,
};
use super::templates::{
    GITIGNORE, generated_package_json, generated_tsconfig, manifest, tysel_env_import,
};
use super::{Options, PackageJsonMode, PackageManager, Template};

pub(super) struct InitPlan {
    pub(super) root: PathBuf,
    pub(super) name: String,
    pub(super) files: Vec<(PathBuf, String)>,
    pub(super) package_exists: bool,
    pub(super) package_update: Option<(PathBuf, Vec<u8>, String)>,
    pub(super) gitignore_update: Option<(PathBuf, Vec<u8>, String)>,
    pub(super) entry_existed: bool,
    pub(super) entry: PathBuf,
    pub(super) include_tests: bool,
    pub(super) dry_run: bool,
    pub(super) json: bool,
    pub(super) diff: bool,
    pub(super) create_package: bool,
    pub(super) adopting: bool,
    pub(super) template: Template,
    pub(super) manifest_format: ManifestFormat,
    pub(super) package_action: &'static str,
    pub(super) package_manager: PackageManager,
    pub(super) install: bool,
    pub(super) verify: bool,
    pub(super) manifest_name: &'static str,
    pub(super) update_summaries: Vec<(PathBuf, Vec<String>)>,
}

impl InitPlan {
    pub(super) fn update_summary(&self, path: &Path) -> &[String] {
        self.update_summaries
            .iter()
            .find(|(candidate, _)| candidate == path)
            .map_or(&[], |(_, summary)| summary)
    }
}

pub(super) fn build(options: Options) -> Result<InitPlan> {
    let root = if options.root.as_os_str().is_empty() { PathBuf::from(".") } else { options.root };
    if root.exists() && !root.is_dir() {
        return Err(anyhow!("project root is not a directory: {}", root.display()));
    }
    let name = application_name(&root)?;
    if !is_application_name(&name) {
        return Err(anyhow!(
            "application name '{name}' must start with a letter or digit and contain only letters, digits, '-', '_' or '.'"
        ));
    }

    let package_path = root.join("package.json");
    let package_exists = package_path.is_file();
    if options.add_scripts
        && (!package_exists
            || matches!(options.package_json, PackageJsonMode::Create | PackageJsonMode::None))
    {
        return Err(anyhow!(
            "--add-scripts requires an existing package.json and --package-json auto or reuse"
        ));
    }
    let create_package = match options.package_json {
        PackageJsonMode::Auto => !package_exists,
        PackageJsonMode::Create if package_exists => {
            return Err(anyhow!("refusing to overwrite existing {}", package_path.display()));
        }
        PackageJsonMode::Create => true,
        PackageJsonMode::Reuse if !package_exists => {
            return Err(anyhow!("--package-json reuse requires {}", package_path.display()));
        }
        PackageJsonMode::Reuse | PackageJsonMode::None => false,
    };
    if options.install && !create_package {
        return Err(anyhow!(
            "--install requires init to create package.json; use --package-json auto or create"
        ));
    }
    if options.install && !options.dry_run && !options.package_manager.is_available() {
        return Err(anyhow!(
            "{} was not found on PATH; install it or choose another --package-manager",
            options.package_manager.command()
        ));
    }
    if options.verify && create_package && !options.install {
        return Err(anyhow!(
            "--verify requires --install when init creates package.json so type checking is not skipped"
        ));
    }
    let has_public_types =
        create_package || package_declares_dependency(&package_path, "@tysel/types");
    let has_runtime_sdk =
        create_package || package_declares_dependency(&package_path, "@tysel/sdk");
    let has_test_types =
        create_package || package_declares_dependency(&package_path, "@tysel/test");
    let typed_entry =
        has_public_types && (!options.template.needs_runtime_sdk() || has_runtime_sdk);
    let package_update = if options.add_scripts && package_exists {
        if fs::symlink_metadata(&package_path)?.file_type().is_symlink() {
            return Err(anyhow!("refusing to modify symlinked {}", package_path.display()));
        }
        package_with_tysel_scripts(&package_path, options.include_tests, typed_entry)?
            .map(|(original, contents)| (package_path.clone(), original, contents))
    } else {
        None
    };
    let existing_js_project = package_exists
        || root.join("tsconfig.json").is_file()
        || root.join("tsconfig.tysel.json").is_file();
    let entry = options.entry.unwrap_or_else(|| {
        if existing_js_project {
            PathBuf::from("src/tysel.ts")
        } else {
            PathBuf::from("src/index.ts")
        }
    });
    let entry = normalize_entry(&entry)?;
    let env_import = tysel_env_import(&entry);
    let entry_existed = root.join(&entry).is_file();
    if !entry_existed {
        ensure_generated_entry_extension(&entry)?;
    }
    let manifest_name = match options.manifest_format {
        ManifestFormat::Toml => "tysel.toml",
        ManifestFormat::Json => "tysel.json",
    };
    for candidate in crate::project::MANIFEST_NAMES {
        let path = root.join(candidate);
        if path.exists() {
            return Err(anyhow!("refusing to overwrite existing {}", path.display()));
        }
    }

    let manifest_contents =
        manifest(&name, &entry, options.manifest_format, options.template, options.include_tests)?;
    let generated_env = if typed_entry {
        let parsed = Manifest::parse_with_format(&manifest_contents, options.manifest_format)?;
        Some(crate::typegen::render(&parsed)?)
    } else {
        None
    };

    let mut files = Vec::new();
    if create_package {
        files.push((
            PathBuf::from("package.json"),
            generated_package_json(&name, options.include_tests, options.template)?,
        ));
    }
    let test_path = if existing_js_project {
        PathBuf::from("tests/tysel.test.ts")
    } else {
        PathBuf::from("tests/app.test.ts")
    };
    let tsconfig_path = if existing_js_project {
        PathBuf::from("tsconfig.tysel.json")
    } else {
        PathBuf::from("tsconfig.json")
    };
    let existing_tsconfig = root.join(&tsconfig_path);
    if existing_tsconfig.is_file() && tsconfig_path.as_path() == Path::new("tsconfig.tysel.json") {
        let mut required = vec![entry.as_path()];
        if options.include_tests && has_test_types {
            required.push(test_path.as_path());
        }
        ensure_tsconfig_files(&existing_tsconfig, &required)?;
    }
    if !existing_tsconfig.exists() {
        files.push((
            tsconfig_path,
            generated_tsconfig(
                &entry,
                typed_entry,
                options.include_tests.then_some(test_path.as_path()),
                has_test_types,
            )?,
        ));
    }
    if !entry_existed {
        files.push((entry.clone(), options.template.source(typed_entry, &env_import)));
    }
    if let Some(contents) = generated_env
        && !root.join("tysel-env.d.ts").exists()
    {
        files.push((PathBuf::from("tysel-env.d.ts"), contents));
    }
    if options.include_tests && !root.join(&test_path).exists() {
        files.push((test_path, options.template.test_source(&entry)));
    }
    files.push((PathBuf::from(manifest_name), manifest_contents));
    if options.manifest_format == ManifestFormat::Toml {
        files.push((
            PathBuf::from(".tysel/manifest.schema.json"),
            tysel_manifest::JSON_SCHEMA.to_owned(),
        ));
    }
    let gitignore_path = root.join(".gitignore");
    let gitignore_update = if gitignore_path.is_file() {
        if fs::symlink_metadata(&gitignore_path)?.file_type().is_symlink() {
            return Err(anyhow!("refusing to modify symlinked {}", gitignore_path.display()));
        }
        gitignore_with_entries(&gitignore_path, GITIGNORE)?
            .map(|(original, contents)| (gitignore_path, original, contents))
    } else {
        files.push((PathBuf::from(".gitignore"), GITIGNORE.to_owned()));
        None
    };

    for (relative, _) in &files {
        ensure_safe_destination(&root, &root.join(relative))?;
    }
    let conflicts = files
        .iter()
        .map(|(relative, _)| root.join(relative))
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    if !conflicts.is_empty() {
        let paths = conflicts.iter().map(|path| path.display().to_string()).collect::<Vec<_>>();
        return Err(anyhow!("refusing to overwrite existing files: {}", paths.join(", ")));
    }

    let package_action = if create_package {
        "create package.json"
    } else if package_update.is_some() {
        "update package.json scripts"
    } else if package_exists {
        "preserve package.json"
    } else {
        "none"
    };
    let mut update_summaries = Vec::new();
    if let Some((path, original, contents)) = &package_update {
        update_summaries.push((path.clone(), summarize_package_update(original, contents)));
    }
    if let Some((path, original, contents)) = &gitignore_update {
        update_summaries.push((path.clone(), summarize_gitignore_update(original, contents)));
    }
    Ok(InitPlan {
        root,
        name,
        files,
        package_exists,
        package_update,
        gitignore_update,
        entry_existed,
        entry,
        include_tests: options.include_tests,
        dry_run: options.dry_run,
        json: options.json,
        diff: options.diff,
        create_package,
        adopting: existing_js_project || entry_existed,
        template: options.template,
        manifest_format: options.manifest_format,
        package_action,
        package_manager: options.package_manager,
        install: options.install,
        verify: options.verify,
        manifest_name,
        update_summaries,
    })
}

fn summarize_package_update(original: &[u8], contents: &str) -> Vec<String> {
    let Ok(before) = serde_json::from_slice::<serde_json::Value>(original) else {
        return Vec::new();
    };
    let Ok(after) = serde_json::from_str::<serde_json::Value>(contents) else {
        return Vec::new();
    };
    let Some(scripts) = after.get("scripts").and_then(serde_json::Value::as_object) else {
        return Vec::new();
    };
    let mut changes = scripts
        .iter()
        .filter(|(name, value)| before["scripts"].get(*name) != Some(*value))
        .map(|(name, value)| format!("+ scripts.{name} = {value}"))
        .collect::<Vec<_>>();
    changes.push("~ package.json is reserialized with normalized formatting".into());
    changes
}

fn summarize_gitignore_update(original: &[u8], contents: &str) -> Vec<String> {
    let before = String::from_utf8_lossy(original);
    let before = before.lines().map(str::trim).collect::<Vec<_>>();
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| !before.contains(line))
        .map(|line| format!("+ {line}"))
        .collect()
}
