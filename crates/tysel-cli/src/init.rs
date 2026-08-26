use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tysel_manifest::ManifestFormat;

mod plan;
mod project;
mod prompt;
mod templates;
mod transaction;

use prompt::{configure, prompt_yes_no};
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

pub struct Request {
    pub root: PathBuf,
    pub template: Option<Template>,
    pub manifest_format: Option<ManifestFormat>,
    pub entry: Option<PathBuf>,
    pub package_json: Option<PackageJsonMode>,
    pub add_scripts: bool,
    pub include_tests: Option<bool>,
    pub dry_run: bool,
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
    include_tests: bool,
    dry_run: bool,
}

pub fn run(request: Request) -> Result<()> {
    let interactive = !request.yes
        && !request.no_interactive
        && !request.dry_run
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal();
    let (options, confirm) = if interactive {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        configure(request, &mut stdin.lock(), &mut stdout.lock())?
    } else {
        (options_from_request(request), false)
    };
    run_options(options, confirm)
}

fn options_from_request(request: Request) -> Options {
    Options {
        root: request.root,
        template: request.template.unwrap_or(Template::Http),
        manifest_format: request.manifest_format.unwrap_or(ManifestFormat::Toml),
        entry: request.entry,
        package_json: request.package_json.unwrap_or(PackageJsonMode::Auto),
        add_scripts: request.add_scripts,
        include_tests: request.include_tests.unwrap_or(true),
        dry_run: request.dry_run,
    }
}

fn run_options(options: Options, confirm: bool) -> Result<()> {
    let plan = plan::build(options)?;
    if plan.dry_run || confirm {
        print_plan(
            &plan.root,
            &plan.files,
            plan.package_exists,
            plan.package_update.is_some(),
            plan.entry_existed,
            &plan.entry,
        );
    }
    if plan.dry_run {
        return Ok(());
    }
    if confirm && !confirm_plan()? {
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
    transaction.commit();

    println!("created {} in {}", plan.name, plan.root.display());
    if plan.include_tests {
        println!("next: cd {} && tysel check && tysel test", plan.root.display());
    } else {
        println!("next: cd {} && tysel check", plan.root.display());
    }
    Ok(())
}

fn print_plan(
    root: &Path,
    files: &[(PathBuf, String)],
    package_exists: bool,
    package_update: bool,
    entry_existed: bool,
    entry: &Path,
) {
    println!("\nTysel init plan for {}", root.display());
    for (relative, _) in files {
        println!("  create {}", relative.display());
    }
    if package_exists && !package_update {
        println!("  preserve package.json");
    }
    if package_update {
        println!("  update package.json scripts");
    }
    if entry_existed {
        println!("  reuse {}", entry.display());
    }
}

fn confirm_plan() -> Result<bool> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    prompt_yes_no(&mut stdin.lock(), &mut stdout.lock(), "Create this project?", true)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;

    use tysel_manifest::Manifest;

    use super::project::{is_application_name, normalize_entry};
    use super::templates::{
        generated_package_json, generated_tsconfig, manifest, tysel_env_import,
    };
    use super::*;

    fn request(root: PathBuf) -> Request {
        Request {
            root,
            template: None,
            manifest_format: None,
            entry: None,
            package_json: None,
            add_scripts: false,
            include_tests: None,
            dry_run: false,
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
    fn generated_project_pins_matching_public_type_packages() {
        let package: serde_json::Value =
            serde_json::from_str(&generated_package_json("app", true, Template::Http).unwrap())
                .unwrap();
        let expected = env!("CARGO_PKG_VERSION");
        assert_eq!(package["devDependencies"]["@tysel/types"], expected);
        assert_eq!(package["devDependencies"]["@tysel/test"], expected);
        assert!(package["devDependencies"].get("tysel").is_none());
        let mcp_package: serde_json::Value =
            serde_json::from_str(&generated_package_json("app", true, Template::Mcp).unwrap())
                .unwrap();
        assert_eq!(mcp_package["devDependencies"]["tysel"], expected);
        let tsconfig: serde_json::Value = serde_json::from_str(
            &generated_tsconfig(Path::new("src/index.ts"), false, true).unwrap(),
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
        assert!(!mcp.contains("from \"tysel\""));
        assert!(mcp.contains("kind: \"mcp\""));
        assert!(mcp.contains("async handler(input: { value: string })"));
    }

    #[test]
    fn typed_mcp_template_uses_schema_driven_inference() {
        let source = Template::Mcp.source(true, "../tysel-env.js");
        assert!(source.contains("import { defineApp } from \"tysel\""));
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
        let mut input = Cursor::new(b"\n");
        let mut output = Vec::new();
        let (options, confirm) = configure(request(root.clone()), &mut input, &mut output).unwrap();
        assert_eq!(options.root, root);
        assert_eq!(options.template, Template::Http);
        assert_eq!(options.manifest_format, ManifestFormat::Toml);
        assert_eq!(options.package_json, PackageJsonMode::Auto);
        assert!(options.entry.is_none());
        assert!(confirm);
        assert!(String::from_utf8(output).unwrap().contains("Quick start"));
    }

    #[test]
    fn interactive_customize_maps_every_answer_to_options() {
        let root = PathBuf::from("custom-start");
        let mut input = Cursor::new(b"2\n3\n2\n2\nsrc/custom.ts\nno\n");
        let mut output = Vec::new();
        let (options, _) = configure(request(root), &mut input, &mut output).unwrap();
        assert_eq!(options.template, Template::Mcp);
        assert_eq!(options.manifest_format, ManifestFormat::Json);
        assert_eq!(options.package_json, PackageJsonMode::None);
        assert_eq!(options.entry, Some(PathBuf::from("src/custom.ts")));
        assert!(!options.include_tests);
    }

    #[test]
    fn explicit_choices_seed_the_interactive_flow_instead_of_disabling_it() {
        let mut request = request(PathBuf::from("partial-start"));
        request.template = Some(Template::Mcp);
        let mut input = Cursor::new(b"2\n2\nsrc/tool.ts\nyes\n");
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
    fn existing_projects_get_an_isolated_typecheck_config() {
        let config: serde_json::Value = serde_json::from_str(
            &generated_tsconfig(Path::new("src/tysel.ts"), true, true).unwrap(),
        )
        .unwrap();
        assert_eq!(config["files"], serde_json::json!(["src/tysel.ts"]));
        assert_eq!(config["compilerOptions"]["types"], serde_json::json!([]));
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
}
