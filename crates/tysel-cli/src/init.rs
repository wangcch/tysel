use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use tysel_manifest::ManifestFormat;

mod plan;
mod project;
mod prompt;
mod templates;
mod transaction;

use prompt::{configure, configure_tty, confirm_tty};
use transaction::ProjectTransaction;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageJsonMode {
    Auto,
    Create,
    Reuse,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    Http,
    Worker,
    Mcp,
    Minimal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    fn detect(root: &Path) -> Self {
        let mut cursor = if root.is_dir() { Some(root) } else { root.parent() };
        while let Some(directory) = cursor {
            for (lockfile, manager) in [
                ("pnpm-lock.yaml", Self::Pnpm),
                ("bun.lock", Self::Bun),
                ("bun.lockb", Self::Bun),
                ("yarn.lock", Self::Yarn),
                ("package-lock.json", Self::Npm),
            ] {
                if directory.join(lockfile).is_file() {
                    return manager;
                }
            }
            cursor = directory.parent();
        }
        Self::Npm
    }

    fn command(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        }
    }

    fn is_available(self) -> bool {
        let Some(path) = std::env::var_os("PATH") else {
            return false;
        };
        std::env::split_paths(&path)
            .any(|directory| command_is_executable(&directory, self.command()))
    }
}

#[cfg(unix)]
fn command_is_executable(directory: &Path, command: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(directory.join(command))
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(windows)]
fn command_is_executable(directory: &Path, command: &str) -> bool {
    ["exe", "cmd", "bat", "com"]
        .iter()
        .any(|extension| directory.join(command).with_extension(extension).is_file())
}

#[cfg(not(any(unix, windows)))]
fn command_is_executable(directory: &Path, command: &str) -> bool {
    directory.join(command).is_file()
}

pub struct Request {
    pub root: Option<PathBuf>,
    pub template: Option<Template>,
    pub manifest_format: Option<ManifestFormat>,
    pub entry: Option<PathBuf>,
    pub package_json: Option<PackageJsonMode>,
    pub add_scripts: bool,
    pub package_manager: Option<PackageManager>,
    pub install: Option<bool>,
    pub verify: Option<bool>,
    pub include_tests: Option<bool>,
    pub dry_run: bool,
    pub json: bool,
    pub diff: bool,
    pub yes: bool,
    pub no_interactive: bool,
}

struct Options {
    root: PathBuf,
    template: Template,
    manifest_format: ManifestFormat,
    entry: Option<PathBuf>,
    package_json: PackageJsonMode,
    add_scripts: bool,
    package_manager: PackageManager,
    install: bool,
    verify: bool,
    include_tests: bool,
    dry_run: bool,
    json: bool,
    diff: bool,
}

pub fn run(request: Request) -> Result<()> {
    let interactive = !request.yes
        && !request.no_interactive
        && !request.dry_run
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal();
    let (options, confirm) = if interactive {
        if std::env::var("TERM").is_ok_and(|term| term == "dumb") {
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            configure(request, &mut stdin.lock(), &mut stdout.lock())?
        } else {
            configure_tty(request)?
        }
    } else {
        (options_from_request(request), false)
    };
    run_options(options, confirm)
}

fn options_from_request(request: Request) -> Options {
    let root = request.root.unwrap_or_else(|| PathBuf::from("."));
    Options {
        package_manager: request.package_manager.unwrap_or_else(|| PackageManager::detect(&root)),
        root,
        template: request.template.unwrap_or(Template::Http),
        manifest_format: request.manifest_format.unwrap_or(ManifestFormat::Toml),
        entry: request.entry,
        package_json: request.package_json.unwrap_or(PackageJsonMode::Auto),
        add_scripts: request.add_scripts,
        install: request.install.unwrap_or(false),
        verify: request.verify.unwrap_or(false),
        include_tests: request.include_tests.unwrap_or(true),
        dry_run: request.dry_run,
        json: request.json,
        diff: request.diff,
    }
}

fn run_options(options: Options, confirm: bool) -> Result<()> {
    let plan = plan::build(options)?;
    if plan.dry_run && plan.json {
        print_plan_json(&plan)?;
    } else if plan.dry_run || confirm {
        print_plan(&plan);
    }
    if plan.dry_run {
        return Ok(());
    }
    if confirm && !confirm_tty("Create this project?", true)? {
        println!("cancelled; no files were changed");
        return Ok(());
    }

    let mut transaction = ProjectTransaction::default();
    for (relative, contents) in plan.files {
        let destination = plan.root.join(relative);
        if let Some(parent) = destination.parent() {
            transaction
                .create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        transaction
            .write(&destination, contents.as_bytes())
            .with_context(|| format!("write {}", destination.display()))?;
    }
    if let Some((path, original, contents)) = plan.package_update {
        transaction
            .replace(&path, &original, contents.as_bytes())
            .with_context(|| format!("update {}", path.display()))?;
    }
    if let Some((path, original, contents)) = plan.gitignore_update {
        transaction
            .replace(&path, &original, contents.as_bytes())
            .with_context(|| format!("update {}", path.display()))?;
    }
    transaction.commit();

    println!("created {} in {}", plan.name, plan.root.display());
    if plan.install {
        install_dependencies(&plan.root, plan.package_manager)?;
    }
    if plan.verify {
        println!("validating generated project...");
        let manifest = plan.root.join(plan.manifest_name);
        let result = if plan.create_package {
            crate::check::run_requiring_types(&manifest)
        } else {
            crate::check::run(&manifest)
        };
        result.with_context(
            || "project files were created, but generated project validation failed",
        )?;
    }
    let root = shell_arg(&plan.root);
    println!("next:");
    println!("  cd {root}");
    if plan.create_package && !plan.install {
        println!("  {} install", plan.package_manager.command());
    }
    if !plan.verify {
        println!("  tysel check");
    }
    if plan.include_tests {
        println!("  tysel test");
    }
    Ok(())
}

fn print_plan(plan: &plan::InitPlan) {
    println!("\nTysel init plan");
    println!("  Project   {}", plan.root.display());
    println!("  Mode      {}", if plan.adopting { "Adopt" } else { "Create" });
    println!("  Template  {}", plan.template.label());
    println!(
        "  Manifest  {}",
        match plan.manifest_format {
            ManifestFormat::Toml => "TOML",
            ManifestFormat::Json => "JSON",
        }
    );
    println!(
        "  Entry     {} ({})",
        plan.entry.display(),
        if plan.entry_existed { "reuse" } else { "create" }
    );
    println!("  Package   {}", plan.package_action);
    if plan.create_package {
        println!(
            "  Install   {} install ({})",
            plan.package_manager.command(),
            if plan.install { "run" } else { "skip" }
        );
    }
    println!("  Verify    {}", if plan.verify { "run" } else { "skip" });
    println!("  Tests     {}", if plan.include_tests { "create" } else { "skip" });
    println!("\nChanges");
    for (relative, _) in &plan.files {
        println!("  create {}", relative.display());
    }
    if plan.package_exists && plan.package_update.is_none() {
        println!("  preserve package.json");
    }
    for (path, changes) in &plan.update_summaries {
        let path = path.strip_prefix(&plan.root).unwrap_or(path);
        println!("  update {}", path.display());
        for change in changes {
            println!("    {change}");
        }
    }
    if plan.entry_existed {
        println!("  reuse {}", plan.entry.display());
    }
    if plan.diff {
        println!("\nDiff");
        for (relative, contents) in &plan.files {
            print_full_diff(relative, None, contents);
        }
        if let Some((path, original, contents)) = &plan.package_update {
            let relative = path.strip_prefix(&plan.root).unwrap_or(path);
            print_full_diff(relative, Some(&String::from_utf8_lossy(original)), contents);
        }
        if let Some((path, original, contents)) = &plan.gitignore_update {
            let relative = path.strip_prefix(&plan.root).unwrap_or(path);
            print_full_diff(relative, Some(&String::from_utf8_lossy(original)), contents);
        }
    }
}

fn print_plan_json(plan: &plan::InitPlan) -> Result<()> {
    let mut changes = plan
        .files
        .iter()
        .map(|(path, contents)| {
            serde_json::json!({
                "operation": "create",
                "path": path.display().to_string(),
                "before": null,
                "after": contents,
            })
        })
        .collect::<Vec<_>>();
    for (path, original, contents, summary) in [
        plan.package_update.as_ref().map(|(path, original, contents)| {
            (path, original, contents, plan.update_summary(path))
        }),
        plan.gitignore_update.as_ref().map(|(path, original, contents)| {
            (path, original, contents, plan.update_summary(path))
        }),
    ]
    .into_iter()
    .flatten()
    {
        let path = path.strip_prefix(&plan.root).unwrap_or(path);
        changes.push(serde_json::json!({
            "operation": "update",
            "path": path.display().to_string(),
            "before": String::from_utf8_lossy(original),
            "after": contents,
            "summary": summary,
        }));
    }
    if plan.package_exists && plan.package_update.is_none() {
        changes.push(serde_json::json!({
            "operation": "preserve",
            "path": "package.json",
        }));
    }
    if plan.entry_existed {
        changes.push(serde_json::json!({
            "operation": "reuse",
            "path": plan.entry.display().to_string(),
        }));
    }
    let output = serde_json::json!({
        "schemaVersion": 1,
        "project": plan.root.display().to_string(),
        "mode": if plan.adopting { "adopt" } else { "create" },
        "template": plan.template.key(),
        "manifestFormat": match plan.manifest_format {
            ManifestFormat::Toml => "toml",
            ManifestFormat::Json => "json",
        },
        "entry": {
            "path": plan.entry.display().to_string(),
            "operation": if plan.entry_existed { "reuse" } else { "create" },
        },
        "package": {
            "action": plan.package_action,
            "manager": plan.package_manager.command(),
            "install": plan.install,
        },
        "verify": plan.verify,
        "tests": plan.include_tests,
        "changes": changes,
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn print_full_diff(path: &Path, before: Option<&str>, after: &str) {
    let before = before.unwrap_or("");
    let before_lines = diff_lines(before);
    let after_lines = diff_lines(after);
    if before_lines.is_empty() {
        println!("--- /dev/null");
    } else {
        println!("--- a/{}", path.display());
    }
    println!("+++ b/{}", path.display());
    println!(
        "@@ -{},{} +{},{} @@",
        if before_lines.is_empty() { 0 } else { 1 },
        before_lines.len(),
        if after_lines.is_empty() { 0 } else { 1 },
        after_lines.len()
    );
    for line in before_lines {
        println!("-{line}");
    }
    for line in after_lines {
        println!("+{line}");
    }
}

fn diff_lines(contents: &str) -> Vec<&str> {
    contents.split_terminator('\n').collect()
}

fn install_dependencies(root: &Path, manager: PackageManager) -> Result<()> {
    println!("installing dependencies with {}...", manager.command());
    let status = Command::new(manager.command())
        .arg("install")
        .current_dir(root)
        .status()
        .with_context(|| {
            format!("project files were created, but {} could not be started", manager.command())
        })?;
    if !status.success() {
        return Err(anyhow::anyhow!(
            "project files were created, but {} install exited with {status}",
            manager.command()
        ));
    }
    println!("installed dependencies");
    Ok(())
}

fn shell_arg(path: &Path) -> String {
    let value = path.display().to_string();
    if value.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-')) {
        value
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;

    use tysel_manifest::Manifest;

    use super::project::{
        ensure_generated_entry_extension, ensure_tsconfig_files, gitignore_with_entries,
        is_application_name, normalize_entry,
    };
    use super::prompt::prompt_yes_no;
    use super::templates::{
        generated_package_json, generated_tsconfig, manifest, tysel_env_import,
    };
    use super::*;

    fn request(root: PathBuf) -> Request {
        Request {
            root: Some(root),
            template: None,
            manifest_format: None,
            entry: None,
            package_json: None,
            add_scripts: false,
            package_manager: None,
            install: None,
            verify: None,
            include_tests: None,
            dry_run: false,
            json: false,
            diff: false,
            yes: false,
            no_interactive: false,
        }
    }

    #[test]
    fn transaction_rolls_back_created_files_and_directories() {
        let root = std::env::temp_dir().join(format!(
            "tysel-init-rollback-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        {
            let mut transaction = ProjectTransaction::default();
            transaction.create_dir_all(&root.join("src")).unwrap();
            transaction.write(&root.join("src/index.ts"), b"partial").unwrap();
        }
        assert!(!root.exists());
    }

    #[test]
    fn transaction_refuses_to_replace_a_file_that_changed_after_planning() {
        let root = std::env::temp_dir().join(format!(
            "tysel-init-concurrent-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let package = root.join("package.json");
        fs::write(&package, b"new contents").unwrap();
        let mut transaction = ProjectTransaction::default();
        let error = transaction.replace(&package, b"old contents", b"replacement").unwrap_err();
        assert!(error.to_string().contains("changed while init was running"));
        assert_eq!(fs::read(&package).unwrap(), b"new contents");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transaction_rolls_back_an_atomic_replacement() {
        let root = std::env::temp_dir().join(format!(
            "tysel-init-replace-rollback-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("package.json");
        fs::write(&path, b"original").unwrap();
        {
            let mut transaction = ProjectTransaction::default();
            transaction.replace(&path, b"original", b"replacement").unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"replacement");
        }
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generated_project_pins_matching_public_type_packages() {
        let package: serde_json::Value =
            serde_json::from_str(&generated_package_json("app", true, Template::Http).unwrap())
                .unwrap();
        let expected = env!("CARGO_PKG_VERSION");
        assert_eq!(package["devDependencies"]["@tysel/types"], expected);
        assert_eq!(package["devDependencies"]["@tysel/test"], expected);
        assert!(package["devDependencies"].get("@tysel/sdk").is_none());
        let mcp_package: serde_json::Value =
            serde_json::from_str(&generated_package_json("app", true, Template::Mcp).unwrap())
                .unwrap();
        assert_eq!(mcp_package["devDependencies"]["@tysel/sdk"], expected);
        let tsconfig: serde_json::Value = serde_json::from_str(
            &generated_tsconfig(
                Path::new("src/index.ts"),
                true,
                Some(Path::new("tests/app.test.ts")),
                true,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            tsconfig["compilerOptions"]["types"],
            serde_json::json!(["@tysel/types", "@tysel/test"])
        );
        let source = Template::Http.source(true, "../tysel-env.js");
        assert!(source.contains("import type { TyselApp } from \"@tysel/types\""));
        assert!(source.contains("import type { TyselEnv } from \"../tysel-env.js\""));
        assert!(source.contains("} satisfies TyselApp<TyselEnv>;"));
        assert!(source.contains("async fetch(request)"));
    }

    #[test]
    fn dependency_free_templates_keep_explicit_web_handler_types() {
        let source = Template::Http.source(false, "../tysel-env.js");
        assert!(!source.contains("@tysel/types"));
        assert!(!source.contains("satisfies TyselApp"));
        assert!(source.contains("async fetch(request: Request): Promise<Response>"));

        let mcp = Template::Mcp.source(false, "../tysel-env.js");
        assert!(!mcp.contains("from \"@tysel/sdk\""));
        assert!(mcp.contains("kind: \"mcp\""));
        assert!(mcp.contains("async handler(input: { value: string })"));
    }

    #[test]
    fn typed_mcp_template_uses_schema_driven_inference() {
        let source = Template::Mcp.source(true, "../tysel-env.js");
        assert!(source.contains("import { defineApp } from \"@tysel/sdk\""));
        assert!(source.contains("export default defineApp<TyselEnv>()({"));
        assert!(source.contains("lookup: {"));
        assert!(source.contains("kind: \"mcp\""));
        assert!(source.contains("async handler(input)"));
        assert!(!source.contains("InferMcpInput"));
    }

    #[test]
    fn generated_environment_import_tracks_entry_depth() {
        assert_eq!(tysel_env_import(Path::new("index.ts")), "./tysel-env.js");
        assert_eq!(tysel_env_import(Path::new("src/index.ts")), "../tysel-env.js");
        assert_eq!(tysel_env_import(Path::new("src/services/index.ts")), "../../tysel-env.js");
    }

    #[test]
    fn generated_package_name_is_valid_for_uppercase_or_underscored_directories() {
        let package: serde_json::Value = serde_json::from_str(
            &generated_package_json("My_App.v2", true, Template::Http).unwrap(),
        )
        .unwrap();
        assert_eq!(package["name"], "my-app.v2");
        assert!(is_application_name("My_App.v2"));
    }

    #[test]
    fn interactive_quick_start_uses_reproducible_defaults() {
        let root = PathBuf::from("quick-start");
        let mut input = Cursor::new(b"\n\n");
        let mut output = Vec::new();
        let (options, confirm) = configure(request(root.clone()), &mut input, &mut output).unwrap();
        assert_eq!(options.root, root);
        assert_eq!(options.template, Template::Http);
        assert_eq!(options.manifest_format, ManifestFormat::Toml);
        assert_eq!(options.package_json, PackageJsonMode::Auto);
        assert_eq!(options.package_manager, PackageManager::detect(&root));
        assert!(!options.install);
        assert!(options.entry.is_none());
        assert!(confirm);
        assert!(String::from_utf8(output).unwrap().contains("Quick start"));
    }

    #[test]
    fn interactive_init_without_a_path_prompts_for_a_project_directory() {
        let mut input = Cursor::new(b"\nnew-service\n\n\n");
        let mut output = Vec::new();
        let (options, confirm) =
            configure(request_without_root(), &mut input, &mut output).unwrap();
        assert_eq!(options.root, Path::new("new-service"));
        assert!(confirm);
        assert!(String::from_utf8(output).unwrap().contains("Project directory"));
    }

    #[test]
    fn interactive_init_can_adopt_the_current_directory() {
        let mut input = Cursor::new(b"2\n\n\n");
        let mut output = Vec::new();
        let (options, _) = configure(request_without_root(), &mut input, &mut output).unwrap();
        assert_eq!(options.root, Path::new("."));
        assert!(String::from_utf8(output).unwrap().contains("Add Tysel"));
    }

    #[test]
    fn package_manager_detection_uses_the_nearest_lockfile() {
        let root = std::env::temp_dir().join(format!(
            "tysel-init-package-manager-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(root.join("apps/service")).unwrap();
        fs::write(root.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();
        assert_eq!(PackageManager::detect(&root.join("apps/service")), PackageManager::Pnpm);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_tysel_tsconfig_must_cover_required_files() {
        let root = std::env::temp_dir().join(format!(
            "tysel-init-tsconfig-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let config = root.join("tsconfig.tysel.json");
        fs::write(&config, "{\n  // Tysel files\n  \"files\": [\"./src/tysel.ts\",],\n}\n")
            .unwrap();
        assert!(ensure_tsconfig_files(&config, &[Path::new("src/tysel.ts")]).is_ok());
        let error = ensure_tsconfig_files(
            &config,
            &[Path::new("src/tysel.ts"), Path::new("tests/tysel.test.ts")],
        )
        .unwrap_err();
        assert!(error.to_string().contains("tests/tysel.test.ts"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn new_project_destination_validation_rejects_nonempty_directories() {
        let root = std::env::temp_dir().join(format!(
            "tysel-init-destination-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("existing.txt"), "keep").unwrap();
        let error = super::project::validate_new_project_root(root.to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("not empty"));
        assert_eq!(
            super::project::validate_new_project_root("  trimmed-project  ").unwrap(),
            Path::new("trimmed-project")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn line_prompt_retries_an_invalid_entry_in_place() {
        let root = PathBuf::from("entry-validation");
        let mut input = Cursor::new(b"src/app.js\nsrc/app.ts\n");
        let mut output = Vec::new();
        let entry = super::prompt::prompt_text_validated(
            &mut input,
            &mut output,
            "Application entry",
            "src/index.ts",
            |value| super::project::validate_entry_input(&root, value),
        )
        .unwrap();
        assert_eq!(entry, Path::new("src/app.ts"));
        assert!(String::from_utf8(output).unwrap().contains("must use a TypeScript extension"));
    }

    #[test]
    fn gitignore_merge_adds_only_missing_tysel_entries() {
        let root = std::env::temp_dir().join(format!(
            "tysel-init-gitignore-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join(".gitignore");
        fs::write(&path, "node_modules/\ncustom/\n").unwrap();
        let (_, merged) = gitignore_with_entries(&path, templates::GITIGNORE).unwrap().unwrap();
        assert_eq!(merged.matches("node_modules/").count(), 1);
        assert!(merged.contains("custom/\n\n# Tysel\n"));
        assert!(merged.contains("data/\n"));
        assert!(merged.contains(".tysel/\n"));
        fs::write(&path, &merged).unwrap();
        assert!(gitignore_with_entries(&path, templates::GITIGNORE).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interactive_customize_maps_every_answer_to_options() {
        let root = PathBuf::from("custom-start");
        let mut input = Cursor::new(b"2\n3\n2\n2\nsrc/custom.ts\nno\nno\n");
        let mut output = Vec::new();
        let (options, _) = configure(request(root), &mut input, &mut output).unwrap();
        assert_eq!(options.template, Template::Mcp);
        assert_eq!(options.manifest_format, ManifestFormat::Json);
        assert_eq!(options.package_json, PackageJsonMode::None);
        assert_eq!(options.entry, Some(PathBuf::from("src/custom.ts")));
        assert!(!options.include_tests);
        assert!(!options.verify);
    }

    #[test]
    fn explicit_choices_seed_the_interactive_flow_instead_of_disabling_it() {
        let mut request = request(PathBuf::from("partial-start"));
        request.template = Some(Template::Mcp);
        let mut input = Cursor::new(b"2\n2\nsrc/tool.ts\nyes\nno\n");
        let mut output = Vec::new();
        let (options, confirm) = configure(request, &mut input, &mut output).unwrap();
        assert_eq!(options.template, Template::Mcp);
        assert_eq!(options.manifest_format, ManifestFormat::Json);
        assert_eq!(options.package_json, PackageJsonMode::None);
        assert_eq!(options.entry, Some(PathBuf::from("src/tool.ts")));
        assert!(options.include_tests);
        assert!(confirm);
        assert!(!String::from_utf8(output).unwrap().contains("How would you like to start?"));
    }

    #[test]
    fn mcp_template_uses_isolated_profile_and_ephemeral_listener() {
        let rendered = manifest(
            "mcp-app",
            Path::new("src/index.ts"),
            ManifestFormat::Json,
            Template::Mcp,
            true,
        )
        .unwrap();
        let parsed = Manifest::parse_with_format(&rendered, ManifestFormat::Json).unwrap();
        assert_eq!(parsed.app.profile, "isolated");
        assert_eq!(parsed.server.listen, "127.0.0.1:0");
        assert!(!rendered.contains("max_response_mb"));
        assert!(Template::Mcp.source(true, "../tysel-env.js").contains("lookup: {"));
    }

    #[test]
    fn generated_manifests_omit_default_response_limit() {
        for format in [ManifestFormat::Toml, ManifestFormat::Json] {
            let rendered =
                manifest("app", Path::new("src/index.ts"), format, Template::Http, true).unwrap();
            assert!(!rendered.contains("max_response_mb"), "{rendered}");
            let parsed = Manifest::parse_with_format(&rendered, format).unwrap();
            assert_eq!(parsed.limits.max_response_mb, 16);
        }
    }

    #[test]
    fn yes_no_prompt_retries_invalid_answers() {
        let mut input = Cursor::new(b"maybe\nno\n");
        let mut output = Vec::new();
        assert!(!prompt_yes_no(&mut input, &mut output, "Continue?", true).unwrap());
        assert!(String::from_utf8(output).unwrap().contains("Enter yes or no"));
    }

    #[test]
    fn closed_input_cancels_instead_of_accepting_defaults() {
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let error = configure(request(PathBuf::from("closed")), &mut input, &mut output)
            .err()
            .expect("EOF must cancel");
        assert!(error.to_string().contains("input closed"), "{error}");
    }

    #[test]
    fn dependency_free_projects_get_an_isolated_typecheck_config() {
        let config: serde_json::Value = serde_json::from_str(
            &generated_tsconfig(
                Path::new("src/tysel.ts"),
                false,
                Some(Path::new("tests/tysel.test.ts")),
                false,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(config["files"], serde_json::json!(["src/tysel.ts"]));
        assert_eq!(config["compilerOptions"]["types"], serde_json::json!([]));
    }

    #[test]
    fn an_existing_javascript_entry_gets_a_compatible_typecheck_config() {
        let config: serde_json::Value = serde_json::from_str(
            &generated_tsconfig(Path::new("src/server.js"), false, None, false).unwrap(),
        )
        .unwrap();
        assert_eq!(config["compilerOptions"]["allowJs"], true);
        assert_eq!(config["compilerOptions"]["checkJs"], false);
    }

    #[test]
    fn adopted_typed_projects_typecheck_the_generated_test() {
        let config: serde_json::Value = serde_json::from_str(
            &generated_tsconfig(
                Path::new("src/tysel.ts"),
                true,
                Some(Path::new("tests/tysel.test.ts")),
                true,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(config["files"], serde_json::json!(["src/tysel.ts", "tests/tysel.test.ts"]));
        assert_eq!(
            config["compilerOptions"]["types"],
            serde_json::json!(["@tysel/types", "@tysel/test"])
        );
    }

    #[test]
    fn no_tests_removes_test_dependencies_and_task_steps() {
        let package: serde_json::Value =
            serde_json::from_str(&generated_package_json("app", false, Template::Http).unwrap())
                .unwrap();
        assert!(package["scripts"].get("test").is_none());
        assert!(package["devDependencies"].get("@tysel/test").is_none());
        let rendered =
            manifest("app", Path::new("src/index.ts"), ManifestFormat::Toml, Template::Http, false)
                .unwrap();
        let parsed = Manifest::parse(&rendered).unwrap();
        assert_eq!(parsed.tasks["verify"].steps, [vec!["check"]]);
    }

    #[test]
    fn entry_paths_are_normalized_and_control_characters_are_rejected() {
        assert_eq!(
            normalize_entry(Path::new("./src/./index.ts")).unwrap(),
            Path::new("src/index.ts")
        );
        assert!(normalize_entry(Path::new("../outside.ts")).is_err());
        assert!(normalize_entry(Path::new("src/bad\nname.ts")).is_err());
        assert!(ensure_generated_entry_extension(Path::new("src/index.ts")).is_ok());
        assert!(ensure_generated_entry_extension(Path::new("src/index.mts")).is_ok());
        assert!(ensure_generated_entry_extension(Path::new("src/index.js")).is_err());

        let rendered = manifest(
            "app",
            Path::new("src/quoted\"entry.ts"),
            ManifestFormat::Toml,
            Template::Http,
            true,
        )
        .unwrap();
        assert_eq!(Manifest::parse(&rendered).unwrap().app.entry, "src/quoted\"entry.ts");

        let wasm =
            manifest("app", Path::new("app.wasm"), ManifestFormat::Toml, Template::Http, true)
                .unwrap_err();
        assert!(wasm.to_string().contains("Wasm Components require a manual manifest"));
    }

    #[test]
    fn next_step_paths_are_shell_safe() {
        assert_eq!(shell_arg(Path::new("my-app")), "my-app");
        assert_eq!(shell_arg(Path::new("parent dir/my-app")), "'parent dir/my-app'");
        assert_eq!(shell_arg(Path::new("it's/app")), "'it'\\''s/app'");
    }

    fn request_without_root() -> Request {
        Request {
            root: None,
            template: None,
            manifest_format: None,
            entry: None,
            package_json: None,
            add_scripts: false,
            package_manager: None,
            install: None,
            verify: None,
            include_tests: None,
            dry_run: false,
            json: false,
            diff: false,
            yes: false,
            no_interactive: false,
        }
    }
}
