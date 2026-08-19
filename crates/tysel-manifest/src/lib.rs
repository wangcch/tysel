//! Application config loaded from `tysel.toml`.
//!
//! Schema follows `roadmap.md` §14. Fields are parsed and reported now;
//! enforcement belongs to the policy engine.

use std::fs;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("failed to read manifest: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid toml: {0}")]
    Toml(#[from] toml::de::Error),
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
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_mb: default_memory_mb(),
            cpu_ms_per_turn: default_cpu_ms(),
            request_timeout_ms: default_timeout_ms(),
            max_in_flight: default_in_flight(),
            max_response_mb: default_response_mb(),
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
        Ok(toml::from_str(raw)?)
    }

    pub fn inspect_report(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Application: {}\n", self.app.name));
        out.push_str(&format!("Profile: {}\n", self.app.profile));
        out.push_str(&format!("Entry: {}\n", self.app.entry));
        out.push_str(&format!("Listen: {}\n\n", self.server.listen));
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
        if !self.permissions.fs_read.is_empty() {
            out.push_str("\nFilesystem\n  Read\n");
            for path in &self.permissions.fs_read {
                out.push_str(&format!("    {path}\n"));
            }
        }
        out.push_str(
            "\nDenied\n  Raw TCP\n  Child Process\n  FFI\n  Dynamic Library\n  Environment\n",
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
