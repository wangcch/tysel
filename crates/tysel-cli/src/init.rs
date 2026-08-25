use std::fs;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use tysel_manifest::{Manifest, ManifestFormat};

const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ES2022",
    "moduleResolution": "bundler",
    "lib": ["ES2022", "DOM"],
    "strict": true,
    "noEmit": true,
    "allowImportingTsExtensions": true,
    "skipLibCheck": true,
    "types": ["@tysel/types", "@tysel/test"]
  },
  "include": ["src", "tests"]
}
"#;

const PACKAGE_JSON: &str = r#"{
  "name": "__NAME__",
  "version": "0.0.1",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "tysel dev",
    "check": "tysel check",
    "test": "tysel test"
  },
  "devDependencies": {
    "@tysel/test": "__TYSEL_VERSION__",
    "@tysel/types": "__TYSEL_VERSION__",
    "typescript": "7.0.2"
  }
}
"#;

const INDEX_TS: &str = r#"export default {
  async fetch(request: Request): Promise<Response> {
    return Response.json({
      message: "Hello from Tysel",
      path: new URL(request.url).pathname,
    });
  },
};
"#;

const WORKER_TS: &str = r#"export default {
  async fetch(): Promise<Response> {
    return Response.json({ status: "ready", worker: "jobs" });
  },
  tasks: {
    jobs: {
      kind: "queue" as const,
      name: "jobs",
      async handler(input: unknown) {
        return { accepted: true, input };
      },
    },
  },
};
"#;

const MCP_TS: &str = r#"export default {
  async fetch(): Promise<Response> {
    return Response.json({ status: "ready", transport: "mcp" });
  },
  tasks: {
    lookup: {
      kind: "mcp" as const,
      description: "Look up a value",
      input: { value: "string" },
      async handler(input: { value: string }) {
        return { value: input.value };
      },
    },
  },
};
"#;

const MINIMAL_TS: &str = r#"export default {
  async fetch(): Promise<Response> {
    return new Response("Hello from Tysel");
  },
};
"#;

const TEST_TS: &str = r#"import app from "../src/index.ts";

test("hello service", async () => {
  const response = await app.fetch(new Request("http://localhost/hello"));
  const body = await response.json() as { message: string; path: string };
  assert.equal(body.message, "Hello from Tysel");
  assert.equal(body.path, "/hello");
});
"#;

const GENERIC_TEST_TS: &str = r#"import app from "../src/index.ts";

test("application exports a fetch handler", () => {
  assert.equal(typeof app.fetch, "function");
});
"#;

const GITIGNORE: &str = "node_modules/\ndist/\ndata/\n.tysel/\n";

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

impl Template {
    fn source(self) -> &'static str {
        match self {
            Self::Http => INDEX_TS,
            Self::Worker => WORKER_TS,
            Self::Mcp => MCP_TS,
            Self::Minimal => MINIMAL_TS,
        }
    }

    fn profile(self) -> &'static str {
        match self {
            Self::Mcp => "isolated",
            Self::Http | Self::Worker | Self::Minimal => "service",
        }
    }

    fn listen(self) -> &'static str {
        match self {
            Self::Mcp => "127.0.0.1:0",
            Self::Http | Self::Worker | Self::Minimal => "127.0.0.1:3000",
        }
    }
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

fn configure<R: BufRead, W: Write>(
    request: Request,
    input: &mut R,
    output: &mut W,
) -> Result<(Options, bool)> {
    writeln!(output, "Create a Tysel application\n")?;
    let customize = request.template.is_some()
        || request.manifest_format.is_some()
        || request.entry.is_some()
        || request.package_json.is_some()
        || request.add_scripts
        || request.include_tests.is_some()
        || prompt_select(
            input,
            output,
            "How would you like to start?",
            &["Quick start (recommended)", "Customize"],
            0,
        )? == 1;
    let template_explicit = request.template.is_some();
    let format_explicit = request.manifest_format.is_some();
    let entry_explicit = request.entry.is_some();
    let package_explicit = request.package_json.is_some() || request.add_scripts;
    let tests_explicit = request.include_tests.is_some();
    let mut options = options_from_request(request);
    if customize && !template_explicit {
        options.template = match prompt_select(
            input,
            output,
            "Application template",
            &["HTTP service", "Queue worker", "MCP tool", "Minimal"],
            0,
        )? {
            0 => Template::Http,
            1 => Template::Worker,
            2 => Template::Mcp,
            _ => Template::Minimal,
        };
    }
    if customize && !format_explicit {
        options.manifest_format = match prompt_select(
            input,
            output,
            "Manifest format",
            &["TOML (recommended)", "JSON"],
            0,
        )? {
            0 => ManifestFormat::Toml,
            _ => ManifestFormat::Json,
        };
    }
    if customize && !package_explicit {
        let package_exists = options.root.join("package.json").is_file();
        if package_exists {
            match prompt_select(
                input,
                output,
                "JavaScript ecosystem integration",
                &[
                    "Reuse package.json",
                    "Reuse package.json and add tysel:* scripts",
                    "Leave package.json untouched",
                ],
                0,
            )? {
                0 => options.package_json = PackageJsonMode::Reuse,
                1 => {
                    options.package_json = PackageJsonMode::Reuse;
                    options.add_scripts = true;
                }
                _ => options.package_json = PackageJsonMode::None,
            }
        } else {
            options.package_json = match prompt_select(
                input,
                output,
                "JavaScript ecosystem integration",
                &["Create package.json", "No package.json"],
                0,
            )? {
                0 => PackageJsonMode::Create,
                _ => PackageJsonMode::None,
            };
        }
    }
    if customize && !entry_explicit {
        let package_exists = options.root.join("package.json").is_file();
        let default_entry = if package_exists { "src/tysel.ts" } else { "src/index.ts" };
        options.entry =
            Some(PathBuf::from(prompt_text(input, output, "Application entry", default_entry)?));
    }
    if customize && !tests_explicit {
        options.include_tests = prompt_yes_no(input, output, "Include tests?", true)?;
    }
    Ok((options, true))
}

fn run_options(options: Options, confirm: bool) -> Result<()> {
    let root =
        if options.root.as_os_str().is_empty() { Path::new(".") } else { options.root.as_path() };
    if root.exists() && !root.is_dir() {
        return Err(anyhow!("project root is not a directory: {}", root.display()));
    }
    let name = application_name(root)?;
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
    let package_update = if options.add_scripts && package_exists {
        if fs::symlink_metadata(&package_path)?.file_type().is_symlink() {
            return Err(anyhow!("refusing to modify symlinked {}", package_path.display()));
        }
        package_with_tysel_scripts(&package_path, options.include_tests)?
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

    let mut files = Vec::new();
    if create_package {
        files.push((
            PathBuf::from("package.json"),
            generated_package_json(&name, options.include_tests)?,
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
        files.push((entry.clone(), options.template.source().to_owned()));
    }
    let test_path = if existing_js_project {
        PathBuf::from("tests/tysel.test.ts")
    } else {
        PathBuf::from("tests/app.test.ts")
    };
    if options.include_tests && !root.join(&test_path).exists() {
        files.push((test_path, test_source(&entry, options.template)));
    }
    files.push((
        PathBuf::from(manifest_name),
        manifest(&name, &entry, options.manifest_format, options.template, options.include_tests)?,
    ));
    if !root.join(".gitignore").exists() {
        files.push((PathBuf::from(".gitignore"), GITIGNORE.to_owned()));
    }

    for (relative, _) in &files {
        ensure_safe_destination(root, &root.join(relative))?;
    }

    let conflicts: Vec<_> = files
        .iter()
        .map(|(relative, _)| root.join(relative))
        .filter(|path| path.exists())
        .collect();
    if !conflicts.is_empty() {
        let paths = conflicts.iter().map(|path| path.display().to_string()).collect::<Vec<_>>();
        return Err(anyhow!("refusing to overwrite existing files: {}", paths.join(", ")));
    }

    if options.dry_run || confirm {
        print_plan(root, &files, package_exists, package_update.is_some(), entry_existed, &entry);
    }
    if options.dry_run {
        return Ok(());
    }
    if confirm && !confirm_plan()? {
        println!("cancelled; no files were changed");
        return Ok(());
    }

    let mut transaction = ProjectTransaction::default();
    for (relative, contents) in files {
        let destination = root.join(relative);
        if let Some(parent) = destination.parent() {
            transaction
                .create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        transaction
            .write(&destination, contents.as_bytes())
            .with_context(|| format!("write {}", destination.display()))?;
    }
    if let Some((path, original, contents)) = package_update {
        transaction
            .replace(&path, &original, contents.as_bytes())
            .with_context(|| format!("update {}", path.display()))?;
    }
    transaction.commit();

    println!("created {name} in {}", root.display());
    if options.include_tests {
        println!("next: cd {} && tysel check && tysel test", root.display());
    } else {
        println!("next: cd {} && tysel check", root.display());
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

fn prompt_select<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    prompt: &str,
    choices: &[&str],
    default: usize,
) -> Result<usize> {
    loop {
        writeln!(output, "{prompt}")?;
        for (index, choice) in choices.iter().enumerate() {
            let marker = if index == default { "›" } else { " " };
            writeln!(output, "  {marker} {}. {choice}", index + 1)?;
        }
        write!(output, "Select [{}]: ", default + 1)?;
        output.flush()?;
        let value = read_answer(input)?;
        if value.is_empty() {
            return Ok(default);
        }
        if let Ok(index) = value.parse::<usize>()
            && (1..=choices.len()).contains(&index)
        {
            return Ok(index - 1);
        }
        writeln!(output, "Enter a number from 1 to {}.\n", choices.len())?;
    }
}

fn prompt_text<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    prompt: &str,
    default: &str,
) -> Result<String> {
    write!(output, "{prompt} [{default}]: ")?;
    output.flush()?;
    let value = read_answer(input)?;
    Ok(if value.is_empty() { default.to_owned() } else { value })
}

fn prompt_yes_no<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    prompt: &str,
    default: bool,
) -> Result<bool> {
    loop {
        let hint = if default { "Y/n" } else { "y/N" };
        write!(output, "{prompt} [{hint}]: ")?;
        output.flush()?;
        let value = read_answer(input)?.to_ascii_lowercase();
        match value.as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(output, "Enter yes or no.")?,
        }
    }
}

fn read_answer<R: BufRead>(input: &mut R) -> Result<String> {
    let mut value = String::new();
    if input.read_line(&mut value)? == 0 {
        return Err(anyhow!("input closed; no files were changed"));
    }
    Ok(value.trim().to_owned())
}

fn test_source(entry: &Path, template: Template) -> String {
    let entry = entry.to_string_lossy().replace('\\', "/");
    let source = if template == Template::Http { TEST_TS } else { GENERIC_TEST_TS };
    source.replace("../src/index.ts", &format!("../{entry}"))
}

fn application_name(root: &Path) -> Result<String> {
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

fn normalize_entry(entry: &Path) -> Result<PathBuf> {
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

fn ensure_safe_destination(root: &Path, destination: &Path) -> Result<()> {
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

fn generated_package_json(name: &str, include_tests: bool) -> Result<String> {
    let package_name = name.to_ascii_lowercase().replace('_', "-");
    let template = PACKAGE_JSON
        .replace("__NAME__", &package_name)
        .replace("__TYSEL_VERSION__", env!("CARGO_PKG_VERSION"));
    let mut package: serde_json::Value = serde_json::from_str(&template)?;
    if !include_tests {
        package["scripts"].as_object_mut().expect("template scripts").remove("test");
        package["devDependencies"]
            .as_object_mut()
            .expect("template devDependencies")
            .remove("@tysel/test");
    }
    let mut rendered = serde_json::to_string_pretty(&package)?;
    rendered.push('\n');
    Ok(rendered)
}

fn generated_tsconfig(entry: &Path, isolated: bool, include_tests: bool) -> Result<String> {
    let mut config: serde_json::Value = serde_json::from_str(TSCONFIG)?;
    let entry = entry.to_string_lossy().replace('\\', "/");
    let mut files = vec![serde_json::Value::String(entry)];
    if include_tests && !isolated {
        files.push(serde_json::Value::String("tests/app.test.ts".into()));
    }
    config.as_object_mut().expect("tsconfig template").remove("include");
    config["files"] = serde_json::Value::Array(files);
    if isolated {
        config["compilerOptions"]["types"] = serde_json::json!([]);
    } else if !include_tests {
        config["compilerOptions"]["types"] = serde_json::json!(["@tysel/types"]);
    }
    let mut rendered = serde_json::to_string_pretty(&config)?;
    rendered.push('\n');
    Ok(rendered)
}

fn package_with_tysel_scripts(
    path: &Path,
    include_tests: bool,
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
        ("tysel:check", "tysel check"),
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

#[derive(Default)]
struct ProjectTransaction {
    created_files: Vec<PathBuf>,
    created_dirs: Vec<PathBuf>,
    modified_files: Vec<(PathBuf, Vec<u8>)>,
    committed: bool,
}

impl ProjectTransaction {
    fn create_dir_all(&mut self, path: &Path) -> std::io::Result<()> {
        let mut missing = Vec::new();
        let mut cursor = path;
        while !cursor.exists() {
            missing.push(cursor.to_path_buf());
            let Some(parent) = cursor.parent() else { break };
            cursor = parent;
        }
        fs::create_dir_all(path)?;
        missing.reverse();
        self.created_dirs.extend(missing);
        Ok(())
    }

    fn write(&mut self, path: &Path, contents: &[u8]) -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        self.created_files.push(path.to_path_buf());
        file.write_all(contents)
    }

    fn replace(&mut self, path: &Path, expected: &[u8], contents: &[u8]) -> std::io::Result<()> {
        let original = fs::read(path)?;
        if original != expected {
            return Err(std::io::Error::other(format!(
                "{} changed while init was running",
                path.display()
            )));
        }
        self.modified_files.push((path.to_path_buf(), original));
        let mut file = fs::OpenOptions::new().write(true).truncate(true).open(path)?;
        file.write_all(contents)
    }

    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for ProjectTransaction {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for path in self.created_files.iter().rev() {
            let _ = fs::remove_file(path);
        }
        for (path, contents) in self.modified_files.iter().rev() {
            let _ = fs::write(path, contents);
        }
        for path in self.created_dirs.iter().rev() {
            let _ = fs::remove_dir(path);
        }
    }
}

fn manifest(
    name: &str,
    entry: &Path,
    format: ManifestFormat,
    template: Template,
    include_tests: bool,
) -> Result<String> {
    let entry = entry.to_string_lossy().replace('\\', "/");
    let mut manifest = Manifest::parse(
        r#"schema_version = 1

[app]
name = "placeholder"
entry = "src/index.ts"
profile = "service"

[server]
listen = "127.0.0.1:3000"
http1 = true
http2 = false
websocket = false

[permissions]

[limits]
memory_mb = 64
cpu_ms_per_turn = 50
request_timeout_ms = 30000
max_in_flight = 256

[durable]
store = "sqlite"
path = "./data/tysel.db"

[observability]
logs = "json"

[tasks.verify]
description = "Check and test"
steps = [["check"], ["test"]]

[tasks.release]
depends = ["verify"]
steps = [["build", "--release"]]
"#,
    )?;
    manifest.app.name = name.to_owned();
    manifest.app.entry = entry;
    manifest.app.profile = template.profile().to_owned();
    manifest.server.listen = template.listen().to_owned();
    manifest.validate_entry_profile(Path::new(&manifest.app.entry)).with_context(|| {
        "tysel init currently generates JavaScript applications; Wasm Components require a manual manifest"
    })?;
    if !include_tests {
        let verify = manifest.tasks.get_mut("verify").expect("template verify task");
        verify.description = Some("Check the application".into());
        verify.steps = vec![vec!["check".into()]];
    }
    let rendered = manifest.to_string_pretty(format)?;
    let rendered = rendered
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            !line.starts_with("max_response_mb =") && !line.starts_with("\"max_response_mb\":")
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!("{rendered}\n"))
}

fn is_application_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|ch| ch.is_ascii_alphanumeric())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

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
            serde_json::from_str(&generated_package_json("app", true).unwrap()).unwrap();
        let expected = env!("CARGO_PKG_VERSION");
        assert_eq!(package["devDependencies"]["@tysel/types"], expected);
        assert_eq!(package["devDependencies"]["@tysel/test"], expected);
        let tsconfig: serde_json::Value = serde_json::from_str(
            &generated_tsconfig(Path::new("src/index.ts"), false, true).unwrap(),
        )
        .unwrap();
        assert_eq!(
            tsconfig["compilerOptions"]["types"],
            serde_json::json!(["@tysel/types", "@tysel/test"])
        );
    }

    #[test]
    fn generated_package_name_is_valid_for_uppercase_or_underscored_directories() {
        let package: serde_json::Value =
            serde_json::from_str(&generated_package_json("My_App.v2", true).unwrap()).unwrap();
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
        assert!(Template::Mcp.source().contains("kind: \"mcp\""));
    }

    #[test]
    fn generated_manifests_omit_schema_only_response_limit() {
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
            serde_json::from_str(&generated_package_json("app", false).unwrap()).unwrap();
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
