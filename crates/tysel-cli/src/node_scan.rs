use std::path::Path;

use anyhow::{Context, Result};

const NODE_PREFIX: &str = "node:";
/// Node builtin roots understood by both source scanning and compatibility
/// classification. Keep this as the single catalog so the two paths cannot
/// silently disagree about a builtin import.
const BUILTINS: &[&str] = &[
    "_http_agent",
    "_http_client",
    "_http_common",
    "_http_incoming",
    "_http_outgoing",
    "_http_server",
    "_stream_duplex",
    "_stream_passthrough",
    "_stream_readable",
    "_stream_transform",
    "_stream_wrap",
    "_stream_writable",
    "_tls_common",
    "_tls_wrap",
    "assert",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "domain",
    "events",
    "fs",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "repl",
    "stream",
    "string_decoder",
    "sys",
    "timers",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

/// Parse a source file and return its Node builtin module specifiers.
pub fn scan_file(path: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    scan_source(path, &text)
}

pub fn scan_source(path: &Path, source: &str) -> Result<Vec<String>> {
    let mut found: Vec<_> = tysel_build::module_specifiers(path, source)?
        .into_iter()
        .filter(|specifier| is_node_builtin(specifier))
        .collect();
    found.sort();
    found.dedup();
    Ok(found)
}

pub fn is_node_builtin(specifier: &str) -> bool {
    let specifier = specifier.trim();
    if specifier.starts_with(NODE_PREFIX) {
        return builtin_root(specifier).is_some();
    }
    builtin_root(specifier).is_some_and(|root| BUILTINS.contains(&root))
}

pub fn builtin_root(specifier: &str) -> Option<&str> {
    let bare = specifier.trim().strip_prefix(NODE_PREFIX).unwrap_or(specifier.trim());
    let root = bare.split('/').next()?;
    (!root.is_empty()).then_some(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_node_prefix_and_bare_fs() {
        let found = scan_source(
            Path::new("example.ts"),
            r#"
            import fs from "fs";
            import "node:path/posix";
            const strict = require("node:assert/strict");
            export default {};
            "#,
        )
        .unwrap();
        assert_eq!(
            found,
            vec!["fs".to_string(), "node:assert/strict".to_string(), "node:path/posix".to_string()]
        );
    }

    #[test]
    fn detects_every_compatibility_only_builtin() {
        let found = scan_source(
            Path::new("example.ts"),
            r#"
            import "node:diagnostics_channel";
            import domain from "domain";
            import "punycode";
            import "repl";
            import "string_decoder";
            "#,
        )
        .unwrap();
        assert_eq!(
            found,
            vec![
                "domain".to_string(),
                "node:diagnostics_channel".to_string(),
                "punycode".to_string(),
                "repl".to_string(),
                "string_decoder".to_string(),
            ]
        );
    }

    #[test]
    fn detects_reserved_node_namespace_and_bare_builtin_roots() {
        for builtin in [
            "node:test",
            "node:timers/promises",
            "node:sqlite",
            "console",
            "timers/promises",
            "_http_agent",
        ] {
            assert!(is_node_builtin(builtin), "missed builtin {builtin}");
        }
        assert!(!is_node_builtin("node:"));
        assert!(!is_node_builtin("application-package"));

        let found = scan_source(
            Path::new("example.ts"),
            r#"
            import "node:test";
            import "node:sqlite";
            import "node:timers/promises";
            import "timers/promises";
            "#,
        )
        .unwrap();
        assert_eq!(
            found,
            vec![
                "node:sqlite".to_string(),
                "node:test".to_string(),
                "node:timers/promises".to_string(),
                "timers/promises".to_string(),
            ]
        );
    }

    #[test]
    fn ignores_application_imports() {
        assert!(
            scan_source(
                Path::new("example.js"),
                r#"// import fs from "fs";
                const text = "require('path')";
                import app from "./index.js"; import hono from "hono";"#,
            )
            .unwrap()
            .is_empty()
        );
    }
}
