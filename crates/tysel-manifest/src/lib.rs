//! Application config loaded from `tysel.toml`.
//!
//! Schema follows `roadmap.md` §14. Fields are parsed and reported now;
//! enforcement belongs to the policy engine.

use std::fs;
use std::path::Path;

use serde::Deserialize;

pub const MAX_FILESYSTEM_ROOTS_PER_OPERATION: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("failed to read manifest: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct App {
    pub name: String,
    pub entry: String,
    #[serde(default = "default_profile")]
    pub profile: String,
}

fn default_profile() -> String {
    "service".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct Server {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_true")]
    pub http1: bool,
    #[serde(default)]
    pub http2: bool,
    #[serde(default)]
    pub websocket: bool,
}

impl Default for Server {
    fn default() -> Self {
        Self { listen: default_listen(), http1: true, http2: false, websocket: false }
    }
}

fn default_listen() -> String {
    "127.0.0.1:3000".into()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
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
        let raw = fs::read_to_string(path)?;
        Self::parse(&raw)
    }

    pub fn parse(raw: &str) -> Result<Self, ManifestError> {
        let manifest: Self = toml::from_str(raw)?;
        manifest.validate()?;
        Ok(manifest)
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
        Ok(())
    }
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
        assert_eq!(manifest.server.listen, "127.0.0.1:3000");
        assert!(manifest.permissions.fetch.is_empty());
        assert!(manifest.inspect_report().contains("Logs: json"));
        assert!(manifest.inspect_report().contains("SQLite"));
        assert!(manifest.inspect_report().contains("./data/tysel.db"));
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
