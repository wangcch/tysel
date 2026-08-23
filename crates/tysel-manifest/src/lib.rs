//! Application config loaded from `tysel.toml` or `tysel.json`.
//!
//! Public schema for application identity, server protocols, permissions,
//! limits, durable storage, and observability. Enforcement belongs to the
//! policy engine.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_FILESYSTEM_ROOTS_PER_OPERATION: usize = 64;
pub const JSON_SCHEMA: &str = include_str!("../schema/tysel-manifest-v1.schema.json");

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("failed to read manifest: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("failed to encode toml: {0}")]
    TomlEncode(#[from] toml::ser::Error),
    #[error("invalid json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported manifest format for {0}; expected .toml or .json")]
    UnsupportedFormat(PathBuf),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestFormat {
    Toml,
    Json,
}

impl ManifestFormat {
    pub fn from_path(path: &Path) -> Result<Self, ManifestError> {
        match path.extension().and_then(|value| value.to_str()) {
            Some("toml") => Ok(Self::Toml),
            Some("json") => Ok(Self::Json),
            _ => Err(ManifestError::UnsupportedFormat(path.to_path_buf())),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Json => "json",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub app: App,
    #[serde(default)]
    pub server: Server,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub durable: Durable,
    #[serde(default)]
    pub observability: Observability,
    #[serde(default)]
    pub tasks: BTreeMap<String, Task>,
}

fn default_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub steps: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub name: String,
    pub entry: String,
    #[serde(default = "default_profile")]
    pub profile: String,
}

fn default_profile() -> String {
    "service".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_true")]
    pub http1: bool,
    #[serde(default)]
    pub http2: bool,
    #[serde(default)]
    pub websocket: bool,
    #[serde(default = "default_workers")]
    pub workers: u32,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            http1: true,
            http2: false,
            websocket: false,
            workers: default_workers(),
        }
    }
}

fn default_listen() -> String {
    "127.0.0.1:3000".into()
}

fn default_true() -> bool {
    true
}

fn default_workers() -> u32 {
    1
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    #[serde(default)]
    pub fetch: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(default)]
    pub postgres: Vec<String>,
    #[serde(default)]
    pub fs_read: Vec<String>,
    #[serde(default)]
    pub fs_write: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(default = "default_memory_mb")]
    pub memory_mb: u32,
    #[serde(default = "default_cpu_ms")]
    pub cpu_ms_per_turn: u64,
    #[serde(default = "default_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_in_flight")]
    pub max_in_flight: u32,
    #[serde(default = "default_response_mb")]
    pub max_response_mb: u32,
    #[serde(default = "default_request_mb")]
    pub max_request_mb: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_mb: default_memory_mb(),
            cpu_ms_per_turn: default_cpu_ms(),
            request_timeout_ms: default_timeout_ms(),
            max_in_flight: default_in_flight(),
            max_response_mb: default_response_mb(),
            max_request_mb: default_request_mb(),
        }
    }
}

fn default_memory_mb() -> u32 {
    128
}
fn default_cpu_ms() -> u64 {
    50
}
fn default_timeout_ms() -> u64 {
    30_000
}
fn default_in_flight() -> u32 {
    1000
}
fn default_response_mb() -> u32 {
    16
}
fn default_request_mb() -> u32 {
    16
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Durable {
    #[serde(default = "default_store")]
    pub store: String,
    #[serde(default = "default_store_path")]
    pub path: String,
}

impl Default for Durable {
    fn default() -> Self {
        Self { store: default_store(), path: default_store_path() }
    }
}

fn default_store() -> String {
    "sqlite".into()
}
fn default_store_path() -> String {
    "./data/tysel.db".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observability {
    #[serde(default = "default_logs")]
    pub logs: String,
    #[serde(default)]
    pub traces: Option<String>,
    #[serde(default)]
    pub metrics: Option<String>,
}

impl Default for Observability {
    fn default() -> Self {
        Self { logs: default_logs(), traces: None, metrics: None }
    }
}

fn default_logs() -> String {
    "json".into()
}

impl Manifest {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let path = path.as_ref();
        let format = ManifestFormat::from_path(path)?;
        let raw = fs::read_to_string(path)?;
        Self::parse_with_format(&raw, format)
    }

    /// Parse a TOML manifest. Kept for callers embedding existing TOML fixtures.
    pub fn parse(raw: &str) -> Result<Self, ManifestError> {
        Self::parse_with_format(raw, ManifestFormat::Toml)
    }

    pub fn parse_with_format(raw: &str, format: ManifestFormat) -> Result<Self, ManifestError> {
        let manifest: Self = match format {
            ManifestFormat::Toml => toml::from_str(raw)?,
            ManifestFormat::Json => serde_json::from_str(raw)?,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn to_string_pretty(&self, format: ManifestFormat) -> Result<String, ManifestError> {
        match format {
            ManifestFormat::Toml => Ok(toml::to_string_pretty(self)?),
            ManifestFormat::Json => Ok(serde_json::to_string_pretty(self)?),
        }
    }

    pub fn inspect_report(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Application: {}\n", self.app.name));
        out.push_str(&format!("Profile: {}\n", self.app.profile));
        out.push_str(&format!("Entry: {}\n", self.app.entry));
        out.push_str(&format!("Listen: {}\n", self.server.listen));
        out.push_str(&format!("Logs: {}\n\n", self.observability.logs));
        out.push_str("Capabilities\n");
        if !self.permissions.fetch.is_empty() {
            out.push_str("  HTTP Client\n");
            for host in &self.permissions.fetch {
                out.push_str(&format!("    {host}\n"));
            }
        }
        if !self.permissions.postgres.is_empty() {
            out.push_str("  Postgres\n");
            for item in &self.permissions.postgres {
                out.push_str(&format!("    {item}\n"));
            }
        }
        if !self.permissions.secrets.is_empty() {
            out.push_str("  Secrets\n");
            for secret in &self.permissions.secrets {
                out.push_str(&format!("    {secret}\n"));
            }
        }
        if self.durable.store == "sqlite" {
            out.push_str("  SQLite\n");
            out.push_str(&format!("    {}\n", self.durable.path));
        }
        if !self.permissions.fs_read.is_empty() || !self.permissions.fs_write.is_empty() {
            out.push_str("\nFilesystem\n");
            if !self.permissions.fs_read.is_empty() {
                out.push_str("  Read\n");
                for path in &self.permissions.fs_read {
                    out.push_str(&format!("    {path}\n"));
                }
            }
            if !self.permissions.fs_write.is_empty() {
                out.push_str("  Write\n");
                for path in &self.permissions.fs_write {
                    out.push_str(&format!("    {path}\n"));
                }
            }
        }
        out.push_str(
            "\nDenied\n  Raw TCP\n  Child Process\n  FFI\n  Dynamic Library\n  Environment\n",
        );
        out
    }

    fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != 1 {
            return Err(ManifestError::Invalid(format!(
                "unsupported schema_version {}; expected 1",
                self.schema_version
            )));
        }
        if self.app.name.trim().is_empty() {
            return Err(ManifestError::Invalid("app.name must be non-empty".into()));
        }
        if !is_app_name(&self.app.name) {
            return Err(ManifestError::Invalid(
                "app.name must start with a letter or digit and contain only letters, digits, '-', '_' or '.'"
                    .into(),
            ));
        }
        if self.app.entry.trim().is_empty() {
            return Err(ManifestError::Invalid("app.entry must be non-empty".into()));
        }
        validate_entry(&self.app.entry)?;
        if !self.server.http1 && !self.server.http2 {
            return Err(ManifestError::Invalid(
                "server must enable at least one of http1 or http2".into(),
            ));
        }
        if self.server.websocket && !self.server.http1 {
            return Err(ManifestError::Invalid(
                "server.websocket requires server.http1 because WebSocket upgrades use HTTP/1.1"
                    .into(),
            ));
        }
        if !(1..=64).contains(&self.server.workers) {
            return Err(ManifestError::Invalid("server.workers must be between 1 and 64".into()));
        }
        if self.server.workers > 1 && self.app.profile != "service" {
            return Err(ManifestError::Invalid(
                "server.workers greater than 1 requires app.profile = \"service\"".into(),
            ));
        }
        if !matches!(self.app.profile.as_str(), "service" | "isolated" | "component") {
            return Err(ManifestError::Invalid(format!(
                "unsupported app profile {:?}; expected service, isolated, or component",
                self.app.profile
            )));
        }
        validate_string_set("permissions.fetch", &self.permissions.fetch)?;
        validate_string_set("permissions.secrets", &self.permissions.secrets)?;
        for (operation, roots) in
            [("fs_read", &self.permissions.fs_read), ("fs_write", &self.permissions.fs_write)]
        {
            let unique = roots
                .iter()
                .map(|root| root.trim())
                .filter(|root| !root.is_empty())
                .collect::<std::collections::BTreeSet<_>>();
            if unique.len() != roots.len() {
                return Err(ManifestError::Invalid(format!(
                    "{operation} roots must be non-empty and unique"
                )));
            }
            if unique.len() > MAX_FILESYSTEM_ROOTS_PER_OPERATION {
                return Err(ManifestError::Invalid(format!(
                    "{operation} declares more than {MAX_FILESYSTEM_ROOTS_PER_OPERATION} roots"
                )));
            }
        }
        if self.permissions.postgres.len() > 1 {
            return Err(ManifestError::Invalid(
                "this runtime supports exactly one Postgres connection; declare at most one grant"
                    .into(),
            ));
        }
        for item in &self.permissions.postgres {
            parse_postgres_grant(item).map_err(ManifestError::Invalid)?;
        }
        self.validate_tasks()?;
        Ok(())
    }

    fn validate_tasks(&self) -> Result<(), ManifestError> {
        for (name, task) in &self.tasks {
            if !is_task_name(name) {
                return Err(ManifestError::Invalid(format!(
                    "invalid task name {name:?}; start with a letter or digit, then use letters, digits, '-', '_' or ':'"
                )));
            }
            if task.depends.is_empty() && task.steps.is_empty() {
                return Err(ManifestError::Invalid(format!(
                    "task {name:?} must declare depends or steps"
                )));
            }
            let mut dependencies = BTreeSet::new();
            for dependency in &task.depends {
                if !self.tasks.contains_key(dependency) {
                    return Err(ManifestError::Invalid(format!(
                        "task {name:?} depends on unknown task {dependency:?}"
                    )));
                }
                if !dependencies.insert(dependency) {
                    return Err(ManifestError::Invalid(format!(
                        "task {name:?} contains duplicate dependency {dependency:?}"
                    )));
                }
            }
            for (index, step) in task.steps.iter().enumerate() {
                if step.is_empty() || step.iter().any(|argument| argument.is_empty()) {
                    return Err(ManifestError::Invalid(format!(
                        "task {name:?} step {} must contain non-empty arguments",
                        index + 1
                    )));
                }
                if !is_task_command(&step[0]) {
                    return Err(ManifestError::Invalid(format!(
                        "task {name:?} step {} uses unsupported Tysel command {:?}",
                        index + 1,
                        step[0]
                    )));
                }
                if step[1..].iter().any(|argument| {
                    matches!(
                        argument.as_str(),
                        "--" | "--manifest" | "-C" | "--project" | "--project-dir"
                    ) || argument.starts_with("-C")
                        || argument.starts_with("--manifest=")
                        || argument.starts_with("--project=")
                        || argument.starts_with("--project-dir=")
                }) {
                    return Err(ManifestError::Invalid(format!(
                        "task {name:?} step {} cannot override its project selection",
                        index + 1
                    )));
                }
            }
        }

        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        for name in self.tasks.keys() {
            visit_task(name, &self.tasks, &mut visiting, &mut visited)?;
        }
        Ok(())
    }
}

fn validate_string_set(name: &str, values: &[String]) -> Result<(), ManifestError> {
    let unique = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();
    if unique.len() != values.len() {
        return Err(ManifestError::Invalid(format!("{name} values must be non-empty and unique")));
    }
    Ok(())
}

fn is_app_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|ch| ch.is_ascii_alphanumeric())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn validate_entry(entry: &str) -> Result<(), ManifestError> {
    if entry.chars().any(char::is_control) || entry.contains('\\') {
        return Err(ManifestError::Invalid(
            "app.entry must use '/' separators and contain no control characters".into(),
        ));
    }
    let path = Path::new(entry);
    let bytes = entry.as_bytes();
    let windows_prefix = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let mut has_normal_component = false;
    if path.is_absolute()
        || windows_prefix
        || path.components().any(|component| match component {
            std::path::Component::Normal(_) => {
                has_normal_component = true;
                false
            }
            std::path::Component::CurDir => false,
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => true,
        })
        || !has_normal_component
    {
        return Err(ManifestError::Invalid(
            "app.entry must be a project-relative path without '..'".into(),
        ));
    }
    Ok(())
}

fn is_task_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|ch| ch.is_ascii_alphanumeric())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':'))
}

fn is_task_command(command: &str) -> bool {
    matches!(
        command,
        "check"
            | "test"
            | "build"
            | "inspect"
            | "compat"
            | "run"
            | "dev"
            | "mcp"
            | "queue"
            | "image"
    )
}

fn visit_task<'a>(
    name: &'a str,
    tasks: &'a BTreeMap<String, Task>,
    visiting: &mut BTreeSet<&'a str>,
    visited: &mut BTreeSet<&'a str>,
) -> Result<(), ManifestError> {
    if visited.contains(name) {
        return Ok(());
    }
    if !visiting.insert(name) {
        return Err(ManifestError::Invalid(format!("task dependency cycle includes {name:?}")));
    }
    for dependency in &tasks[name].depends {
        visit_task(dependency, tasks, visiting, visited)?;
    }
    visiting.remove(name);
    visited.insert(name);
    Ok(())
}

/// A named Postgres connection from `[permissions] postgres`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresGrant {
    pub name: String,
    pub mode: Option<String>,
}

/// Parse `main` or `main:read-write`. URLs are rejected so credentials cannot
/// enter the manifest or TAP trailer.
pub fn parse_postgres_grant(raw: &str) -> Result<PostgresGrant, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("postgres permission must not be empty".into());
    }
    if raw.contains("://") || raw.contains('/') || raw.contains('@') {
        return Err(format!(
            "postgres permission {raw:?} must be a connection name (e.g. main:read-write), not a URL"
        ));
    }
    let (name, mode) = match raw.split_once(':') {
        Some((name, mode)) => (name, Some(mode)),
        None => (raw, None),
    };
    if !is_postgres_alias(name) {
        return Err(format!(
            "postgres permission {raw:?} must be a connection name (e.g. main:read-write)"
        ));
    }
    if let Some(mode) = mode
        && mode != "read-write"
        && mode != "read-only"
    {
        return Err(format!("postgres permission {raw:?} mode must be read-write or read-only"));
    }
    Ok(PostgresGrant { name: name.to_owned(), mode: mode.map(str::to_owned) })
}

pub fn postgres_url_env_key(name: &str) -> String {
    format!("TYSEL_POSTGRES_{}", name.replace('-', "_").to_ascii_uppercase())
}

/// Resolve the declared connection's URL and access mode from
/// `TYSEL_POSTGRES_<NAME>`.
/// Invalid grants (including URLs) are ignored so they are never used as
/// connection strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPostgres {
    pub url: String,
    pub read_only: bool,
}

pub fn resolve_postgres(
    grants: &[String],
    file_values: &std::collections::HashMap<String, String>,
) -> Option<ResolvedPostgres> {
    let raw = grants.iter().map(|item| item.trim()).find(|item| !item.is_empty())?;
    let grant = parse_postgres_grant(raw).ok()?;
    let key = postgres_url_env_key(&grant.name);
    let url = std::env::var(&key)
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| file_values.get(&key).filter(|value| !value.is_empty()).cloned())?;
    Some(ResolvedPostgres { url, read_only: grant.mode.as_deref() == Some("read-only") })
}

fn is_postgres_alias(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_profiles_and_fields() {
        let profile = Manifest::parse(
            r#"
[app]
name = "future"
entry = "src/index.ts"
profile = "future-profile"
"#,
        )
        .unwrap_err();
        assert!(profile.to_string().contains("unsupported app profile"));

        let field = Manifest::parse(
            r#"
[app]
name = "future"
entry = "src/index.ts"
compatibility_flag = true
"#,
        )
        .unwrap_err();
        assert!(field.to_string().contains("unknown field"));
    }

    #[test]
    fn rejects_duplicate_and_excessive_filesystem_roots() {
        let mut manifest = Manifest::parse(
            r#"
[app]
name = "fs-policy"
entry = "app.wasm"
"#,
        )
        .unwrap();
        manifest.permissions.fs_read = vec!["./data".into(), "./data".into()];
        assert!(manifest.validate().unwrap_err().to_string().contains("unique"));

        manifest.permissions.fs_read = (0..=MAX_FILESYSTEM_ROOTS_PER_OPERATION)
            .map(|index| format!("./data-{index}"))
            .collect();
        assert!(manifest.validate().unwrap_err().to_string().contains("more than"));
    }

    #[test]
    fn rejects_server_with_no_http_protocol() {
        let error = Manifest::parse(
            r#"
[app]
name = "no-http"
entry = "src/index.ts"

[server]
http1 = false
http2 = false
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("at least one"));
    }

    #[test]
    fn rejects_websocket_without_http1() {
        let error = Manifest::parse(
            r#"
[app]
name = "h2-websocket"
entry = "src/index.ts"

[server]
http1 = false
http2 = true
websocket = true
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("requires server.http1"));
    }

    #[test]
    fn service_workers_are_explicit_and_bounded() {
        let manifest = Manifest::parse(
            r#"
[app]
name = "parallel-service"
entry = "src/index.ts"
profile = "service"

[server]
workers = 2
"#,
        )
        .unwrap();
        assert_eq!(manifest.server.workers, 2);

        let invalid = Manifest::parse(
            r#"
[app]
name = "invalid-workers"
entry = "src/index.ts"

[server]
workers = 0
"#,
        )
        .unwrap_err();
        assert!(invalid.to_string().contains("between 1 and 64"));

        let isolated = Manifest::parse(
            r#"
[app]
name = "isolated-workers"
entry = "src/index.ts"
profile = "isolated"

[server]
workers = 2
"#,
        )
        .unwrap_err();
        assert!(isolated.to_string().contains("requires app.profile"));
    }

    #[test]
    fn parses_hello_manifest() {
        let manifest = Manifest::parse(
            r#"
[app]
name = "hello-service"
entry = "src/index.ts"
profile = "service"

[server]
listen = "127.0.0.1:3000"
"#,
        )
        .unwrap();
        assert_eq!(manifest.app.name, "hello-service");
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.server.listen, "127.0.0.1:3000");
        assert!(manifest.permissions.fetch.is_empty());
        assert!(manifest.inspect_report().contains("Logs: json"));
        assert!(manifest.inspect_report().contains("SQLite"));
        assert!(manifest.inspect_report().contains("./data/tysel.db"));
    }

    #[test]
    fn parses_equivalent_json_manifest() {
        let manifest = Manifest::parse_with_format(
            r#"{
  "schema_version": 1,
  "app": {
    "name": "hello-json",
    "entry": "src/index.ts",
    "profile": "service"
  },
  "server": {
    "listen": "127.0.0.1:4000"
  }
}"#,
            ManifestFormat::Json,
        )
        .unwrap();
        assert_eq!(manifest.app.name, "hello-json");
        assert_eq!(manifest.server.listen, "127.0.0.1:4000");
        assert!(manifest.server.http1);
    }

    #[test]
    fn rejects_unsupported_schema_version_in_both_formats() {
        let toml = Manifest::parse(
            r#"
schema_version = 2

[app]
name = "future"
entry = "src/index.ts"
"#,
        )
        .unwrap_err();
        assert!(toml.to_string().contains("schema_version 2"));

        let json = Manifest::parse_with_format(
            r#"{"schema_version":2,"app":{"name":"future","entry":"src/index.ts"}}"#,
            ManifestFormat::Json,
        )
        .unwrap_err();
        assert!(json.to_string().contains("schema_version 2"));
    }

    #[test]
    fn rejects_empty_identity_and_non_set_permissions() {
        let empty_name = Manifest::parse(
            r#"
[app]
name = " "
entry = "src/index.ts"
"#,
        )
        .unwrap_err();
        assert!(empty_name.to_string().contains("app.name"), "{empty_name}");

        let duplicate_fetch = Manifest::parse(
            r#"
[app]
name = "app"
entry = "src/index.ts"

[permissions]
fetch = ["api.example.com", " api.example.com "]
"#,
        )
        .unwrap_err();
        assert!(duplicate_fetch.to_string().contains("permissions.fetch"), "{duplicate_fetch}");

        let unsafe_name = Manifest::parse(
            r#"
[app]
name = "../../outside"
entry = "src/index.ts"
"#,
        )
        .unwrap_err();
        assert!(unsafe_name.to_string().contains("app.name"), "{unsafe_name}");

        for entry in ["../outside.ts", "/tmp/outside.ts", "C:/outside.ts", ".", "./", "././."] {
            let error = Manifest::parse(&format!(
                r#"
[app]
name = "app"
entry = "{entry}"
"#
            ))
            .unwrap_err();
            assert!(error.to_string().contains("app.entry"), "{entry}: {error}");
        }
    }

    #[test]
    fn parses_and_validates_native_tasks() {
        let manifest = Manifest::parse(
            r#"
[app]
name = "tasks"
entry = "src/index.ts"

[tasks.verify]
description = "Check and test"
steps = [["check"], ["test"]]

[tasks.release]
depends = ["verify"]
steps = [["build", "--release"]]
"#,
        )
        .unwrap();
        assert_eq!(manifest.tasks["verify"].steps.len(), 2);
        assert_eq!(manifest.tasks["release"].depends, ["verify"]);
    }

    #[test]
    fn rejects_invalid_task_graphs_and_commands() {
        let cycle = Manifest::parse(
            r#"
[app]
name = "tasks"
entry = "src/index.ts"

[tasks.a]
depends = ["b"]

[tasks.b]
depends = ["a"]
"#,
        )
        .unwrap_err();
        assert!(cycle.to_string().contains("cycle"), "{cycle}");

        let command = Manifest::parse(
            r#"
[app]
name = "tasks"
entry = "src/index.ts"

[tasks.unsafe]
steps = [["upgrade"]]
"#,
        )
        .unwrap_err();
        assert!(command.to_string().contains("unsupported Tysel command"), "{command}");

        let option_like_name = Manifest::parse(
            r#"
[app]
name = "tasks"
entry = "src/index.ts"

[tasks."-unsafe"]
steps = [["check"]]
"#,
        )
        .unwrap_err();
        assert!(option_like_name.to_string().contains("invalid task name"));

        let project_override = Manifest::parse(
            r#"
[app]
name = "tasks"
entry = "src/index.ts"

[tasks.unsafe]
steps = [["check", "-C/tmp/other-project"]]
"#,
        )
        .unwrap_err();
        assert!(project_override.to_string().contains("project selection"), "{project_override}");
    }

    #[test]
    fn bundled_json_schema_is_valid_and_versioned() {
        let schema: serde_json::Value = serde_json::from_str(JSON_SCHEMA).unwrap();
        assert_eq!(schema["$id"], "https://tysel.dev/schemas/manifest-v1.json");
        assert_eq!(schema["properties"]["schema_version"]["const"], 1);
        assert!(schema["properties"]["tasks"].is_object());
        assert_eq!(schema["properties"]["server"]["properties"]["workers"]["default"], 1);
        assert_eq!(schema["properties"]["server"]["properties"]["workers"]["maximum"], 64);

        let entry_pattern = schema["properties"]["app"]["properties"]["entry"]["pattern"]
            .as_str()
            .expect("app.entry pattern");
        assert!(
            entry_pattern.contains(r"(?!\.(?:/\.)*/?$)"),
            "schema must reject paths made only from current-directory components"
        );

        for (field, maximum) in [
            ("memory_mb", u64::from(u32::MAX)),
            ("cpu_ms_per_turn", u64::MAX),
            ("request_timeout_ms", u64::MAX),
            ("max_in_flight", u64::from(u32::MAX)),
            ("max_response_mb", u64::from(u32::MAX)),
            ("max_request_mb", u64::from(u32::MAX)),
        ] {
            assert_eq!(
                schema["properties"]["limits"]["properties"][field]["maximum"], maximum,
                "limits.{field} must match its runtime integer representation"
            );
        }
    }

    #[test]
    fn json_limits_reject_values_above_their_schema_maximum() {
        for (field, overflow) in [
            ("memory_mb", "4294967296"),
            ("cpu_ms_per_turn", "18446744073709551616"),
            ("request_timeout_ms", "18446744073709551616"),
            ("max_in_flight", "4294967296"),
            ("max_response_mb", "4294967296"),
            ("max_request_mb", "4294967296"),
        ] {
            let raw = format!(
                r#"{{
  "app": {{ "name": "limits", "entry": "src/index.ts" }},
  "limits": {{ "{field}": {overflow} }}
}}"#
            );
            let error = Manifest::parse_with_format(&raw, ManifestFormat::Json).unwrap_err();
            assert!(error.to_string().contains("invalid json"), "{field}: {error}");
        }
    }

    #[test]
    fn postgres_grant_parses_named_connection() {
        let grant = parse_postgres_grant("main:read-write").unwrap();
        assert_eq!(grant.name, "main");
        assert_eq!(grant.mode.as_deref(), Some("read-write"));
        assert_eq!(postgres_url_env_key("main"), "TYSEL_POSTGRES_MAIN");
        let bare = parse_postgres_grant("main").unwrap();
        assert_eq!(bare.mode, None);
    }

    #[test]
    fn postgres_url_is_rejected() {
        let err = parse_postgres_grant("postgres://user:pass@localhost/db").unwrap_err();
        assert!(err.contains("not a URL"), "{err}");
        let err = Manifest::parse(
            r#"
[app]
name = "hello-service"
entry = "src/index.ts"
profile = "service"

[permissions]
postgres = ["postgres://user:pass@localhost/db"]
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not a URL"), "{err}");
    }

    #[test]
    fn resolve_postgres_ignores_embedded_urls() {
        let url = resolve_postgres(
            &["postgres://user:pass@localhost/db".into()],
            &std::collections::HashMap::new(),
        );
        assert_eq!(url, None);
    }

    #[test]
    fn resolve_postgres_preserves_read_only_mode() {
        let mut file_values = std::collections::HashMap::new();
        file_values.insert("TYSEL_POSTGRES_REVIEW_RO".into(), "postgres://localhost/app".into());
        let resolved = resolve_postgres(&["review_ro:read-only".into()], &file_values).unwrap();
        assert_eq!(resolved.url, "postgres://localhost/app");
        assert!(resolved.read_only);
    }

    #[test]
    fn rejects_multiple_postgres_connections_until_named_api_exists() {
        let err = Manifest::parse(
            r#"
[app]
name = "hello-service"
entry = "src/index.ts"
profile = "service"

[permissions]
postgres = ["main:read-write", "audit:read-only"]
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("at most one grant"), "{err}");
    }
}
