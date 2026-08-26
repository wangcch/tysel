use std::fs;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use tysel_manifest::{Manifest, ManifestFormat};

use super::project::{
    application_name, ensure_safe_destination, is_application_name, normalize_entry,
    package_declares_dependency, package_with_tysel_scripts,
};
use super::templates::{
    GITIGNORE, generated_package_json, generated_tsconfig, manifest, tysel_env_import,
};
use super::{Options, PackageJsonMode};

pub(super) struct InitPlan {
    pub(super) root: PathBuf,
    pub(super) name: String,
    pub(super) files: Vec<(PathBuf, String)>,
    pub(super) package_exists: bool,
    pub(super) package_update: Option<(PathBuf, Vec<u8>, String)>,
    pub(super) entry_existed: bool,
    pub(super) entry: PathBuf,
    pub(super) include_tests: bool,
    pub(super) dry_run: bool,
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
    let has_public_types =
        create_package || package_declares_dependency(&package_path, "@tysel/types");
    let has_runtime_sdk = create_package || package_declares_dependency(&package_path, "tysel");
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
    let tsconfig_path = if existing_js_project {
        PathBuf::from("tsconfig.tysel.json")
    } else {
        PathBuf::from("tsconfig.json")
    };
    if !root.join(&tsconfig_path).exists() {
        files.push((
            tsconfig_path,
            generated_tsconfig(
                &entry,
                existing_js_project || !create_package,
                options.include_tests,
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
    let test_path = if existing_js_project {
        PathBuf::from("tests/tysel.test.ts")
    } else {
        PathBuf::from("tests/app.test.ts")
    };
    if options.include_tests && !root.join(&test_path).exists() {
        files.push((test_path, options.template.test_source(&entry)));
    }
    files.push((PathBuf::from(manifest_name), manifest_contents));
    if !root.join(".gitignore").exists() {
        files.push((PathBuf::from(".gitignore"), GITIGNORE.to_owned()));
    }

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

    Ok(InitPlan {
        root,
        name,
        files,
        package_exists,
        package_update,
        entry_existed,
        entry,
        include_tests: options.include_tests,
        dry_run: options.dry_run,
    })
}
