use std::path::Path;

use anyhow::{Context, Result};

const NODE_PREFIX: &str = "node:";
const BUILTINS: &[&str] = &[
    "assert",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "crypto",
    "dgram",
    "dns",
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
    "querystring",
    "readline",
    "stream",
    "tls",
    "tty",
    "url",
    "util",
    "v8",
    "vm",
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
