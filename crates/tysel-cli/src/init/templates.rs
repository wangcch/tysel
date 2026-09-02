use std::path::Path;

use anyhow::{Context, Result};
use tysel_manifest::{Manifest, ManifestFormat};

use super::Template;

pub(super) const TSCONFIG: &str = r#"{
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

pub(super) const PACKAGE_JSON: &str = r#"{
  "name": "__NAME__",
  "version": "0.0.1",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "tysel dev",
    "check": "tysel types --check && tysel check",
    "test": "tysel test"
  },
  "devDependencies": {
    "@tysel/test": "__TYSEL_VERSION__",
    "@tysel/types": "__TYSEL_VERSION__",
    "@tysel/sdk": "__TYSEL_VERSION__",
    "typescript": "7.0.2"
  }
}
"#;

pub(super) const GITIGNORE: &str = "node_modules/\ndist/\ndata/\n.tysel/\n";

const HTTP_TYPED: &str = r#"import type { TyselApp } from "@tysel/types";
import type { TyselEnv } from "__TYSEL_ENV_IMPORT__";

export default {
  async fetch(request) {
    return Response.json({
      message: "Hello from Tysel",
      path: new URL(request.url).pathname,
    });
  },
} satisfies TyselApp<TyselEnv>;
"#;

const HTTP_STANDALONE: &str = r#"export default {
  async fetch(request: Request): Promise<Response> {
    return Response.json({
      message: "Hello from Tysel",
      path: new URL(request.url).pathname,
    });
  },
};
"#;

const WORKER_TYPED: &str = r#"import type { TyselApp } from "@tysel/types";
import type { TyselEnv } from "__TYSEL_ENV_IMPORT__";

export default {
  async fetch() {
    return Response.json({ status: "ready", worker: "jobs" });
  },
  tasks: {
    jobs: {
      kind: "queue",
      name: "jobs",
      async handler(input: unknown) {
        return { accepted: true, input };
      },
    },
  },
} satisfies TyselApp<TyselEnv>;
"#;

const WORKER_STANDALONE: &str = r#"export default {
  async fetch(): Promise<Response> {
    return Response.json({ status: "ready", worker: "jobs" });
  },
  tasks: {
    jobs: {
      kind: "queue",
      name: "jobs",
      async handler(input: unknown) {
        return { accepted: true, input };
      },
    },
  },
};
"#;

const MCP_TYPED: &str = r#"import type { TyselEnv } from "__TYSEL_ENV_IMPORT__";
import { defineApp } from "@tysel/sdk";

export default defineApp<TyselEnv>()({
  async fetch() {
    return Response.json({ status: "ready", transport: "mcp" });
  },
  tasks: {
    lookup: {
      kind: "mcp",
      description: "Look up a value",
      input: { value: "string" },
      async handler(input) {
        return { value: input.value };
      },
    },
  },
});
"#;

const MCP_STANDALONE: &str = r#"export default {
  async fetch(): Promise<Response> {
    return Response.json({ status: "ready", transport: "mcp" });
  },
  tasks: {
    lookup: {
      kind: "mcp",
      description: "Look up a value",
      input: { value: "string" },
      async handler(input: { value: string }) {
        return { value: input.value };
      },
    },
  },
};
"#;

const MINIMAL_TYPED: &str = r#"import type { TyselApp } from "@tysel/types";
import type { TyselEnv } from "__TYSEL_ENV_IMPORT__";

export default {
  async fetch() {
    return new Response("Hello from Tysel");
  },
} satisfies TyselApp<TyselEnv>;
"#;

const MINIMAL_STANDALONE: &str = r#"export default {
  async fetch(): Promise<Response> {
    return new Response("Hello from Tysel");
  },
};
"#;

const HTTP_TEST: &str = r#"import app from "__ENTRY_IMPORT__";

test("hello service", async () => {
  const response = await app.fetch(new Request("http://localhost/hello"));
  const body = await response.json() as { message: string; path: string };
  assert.equal(body.message, "Hello from Tysel");
  assert.equal(body.path, "/hello");
});
"#;

const FETCH_TEST: &str = r#"import app from "__ENTRY_IMPORT__";

test("application exports a fetch handler", () => {
  assert.equal(typeof app.fetch, "function");
});
"#;

struct TemplateSpec {
    profile: &'static str,
    listen: &'static str,
    typed_source: &'static str,
    standalone_source: &'static str,
    test_source: &'static str,
    needs_runtime_sdk: bool,
}

impl Template {
    fn spec(self) -> TemplateSpec {
        match self {
            Self::Http => TemplateSpec {
                profile: "service",
                listen: "127.0.0.1:3000",
                typed_source: HTTP_TYPED,
                standalone_source: HTTP_STANDALONE,
                test_source: HTTP_TEST,
                needs_runtime_sdk: false,
            },
            Self::Worker => TemplateSpec {
                profile: "service",
                listen: "127.0.0.1:3000",
                typed_source: WORKER_TYPED,
                standalone_source: WORKER_STANDALONE,
                test_source: FETCH_TEST,
                needs_runtime_sdk: false,
            },
            Self::Mcp => TemplateSpec {
                profile: "isolated",
                listen: "127.0.0.1:0",
                typed_source: MCP_TYPED,
                standalone_source: MCP_STANDALONE,
                test_source: FETCH_TEST,
                needs_runtime_sdk: true,
            },
            Self::Minimal => TemplateSpec {
                profile: "service",
                listen: "127.0.0.1:3000",
                typed_source: MINIMAL_TYPED,
                standalone_source: MINIMAL_STANDALONE,
                test_source: FETCH_TEST,
                needs_runtime_sdk: false,
            },
        }
    }

    pub(super) fn source(self, typed: bool, env_import: &str) -> String {
        let spec = self.spec();
        if typed {
            spec.typed_source.replace("__TYSEL_ENV_IMPORT__", env_import)
        } else {
            spec.standalone_source.to_owned()
        }
    }

    pub(super) fn test_source(self, entry: &Path) -> String {
        let entry = entry.to_string_lossy().replace('\\', "/");
        self.spec().test_source.replace("__ENTRY_IMPORT__", &format!("../{entry}"))
    }

    pub(super) fn profile(self) -> &'static str {
        self.spec().profile
    }

    pub(super) fn listen(self) -> &'static str {
        self.spec().listen
    }

    pub(super) fn needs_runtime_sdk(self) -> bool {
        self.spec().needs_runtime_sdk
    }
}

pub(super) fn tysel_env_import(entry: &Path) -> String {
    let depth = entry.parent().map(|parent| parent.components().count()).unwrap_or(0);
    if depth == 0 {
        "./tysel-env.js".into()
    } else {
        format!("{}tysel-env.js", "../".repeat(depth))
    }
}

pub(super) fn generated_package_json(
    name: &str,
    include_tests: bool,
    template: Template,
) -> Result<String> {
    let package_name = name.to_ascii_lowercase().replace('_', "-");
    let package_template = PACKAGE_JSON
        .replace("__NAME__", &package_name)
        .replace("__TYSEL_VERSION__", env!("CARGO_PKG_VERSION"));
    let mut package: serde_json::Value = serde_json::from_str(&package_template)?;
    if !include_tests {
        package["scripts"].as_object_mut().expect("template scripts").remove("test");
        package["devDependencies"]
            .as_object_mut()
            .expect("template devDependencies")
            .remove("@tysel/test");
    }
    if !template.needs_runtime_sdk() {
        package["devDependencies"]
            .as_object_mut()
            .expect("template devDependencies")
            .remove("@tysel/sdk");
    }
    let mut rendered = serde_json::to_string_pretty(&package)?;
    rendered.push('\n');
    Ok(rendered)
}

pub(super) fn generated_tsconfig(
    entry: &Path,
    isolated: bool,
    include_tests: bool,
) -> Result<String> {
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

pub(super) fn manifest(
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;

    use super::*;

    #[test]
    fn every_typed_and_standalone_template_typechecks() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("resolve workspace");
        let tsc = workspace.join("node_modules/typescript/bin/tsc");
        assert!(tsc.is_file(), "missing TypeScript compiler at {}", tsc.display());

        let root = workspace.join("target").join(format!(
            "tysel-init-template-types-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        let source_dir = root.join("src");
        fs::create_dir_all(&source_dir).expect("create template typecheck directory");

        let mut files = Vec::new();
        for (name, template) in [
            ("http", Template::Http),
            ("worker", Template::Worker),
            ("mcp", Template::Mcp),
            ("minimal", Template::Minimal),
        ] {
            for (mode, typed) in [("typed", true), ("standalone", false)] {
                let relative = format!("src/{name}-{mode}.ts");
                fs::write(root.join(&relative), template.source(typed, "../tysel-env.js"))
                    .expect("write rendered template");
                files.push(relative);
            }
        }

        let rendered_manifest = manifest(
            "template-check",
            Path::new("src/http-typed.ts"),
            ManifestFormat::Toml,
            Template::Http,
            false,
        )
        .expect("render manifest");
        let parsed = Manifest::parse(&rendered_manifest).expect("parse rendered manifest");
        fs::write(
            root.join("tysel-env.d.ts"),
            crate::typegen::render(&parsed).expect("render environment types"),
        )
        .expect("write environment types");
        files.push("tysel-env.d.ts".into());

        fs::write(
            root.join("tsconfig.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "compilerOptions": {
                    "target": "ES2022",
                    "module": "ES2022",
                    "moduleResolution": "bundler",
                    "lib": ["ES2022", "DOM"],
                    "strict": true,
                    "noEmit": true,
                    "skipLibCheck": true,
                    "paths": {
                        "@tysel/types": [workspace.join("packages/tysel-types/src/index.ts")],
                        "@tysel/sdk": [workspace.join("packages/tysel/src/index.ts")]
                    }
                },
                "files": files,
            }))
            .expect("render typecheck config"),
        )
        .expect("write typecheck config");

        let output = Command::new("node")
            .arg(&tsc)
            .args(["--noEmit", "-p", "tsconfig.json"])
            .current_dir(&root)
            .output()
            .expect("run TypeScript compiler");
        assert!(
            output.status.success(),
            "generated template typecheck failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        fs::remove_dir_all(root).expect("remove template typecheck directory");
    }
}
