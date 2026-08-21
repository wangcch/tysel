use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

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
    "types": ["@tysel/test"]
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
    "@tysel/test": "0.0.1",
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

const TEST_TS: &str = r#"import app from "../src/index.ts";

test("hello service", async () => {
  const response = await app.fetch(new Request("http://localhost/hello"));
  const body = await response.json() as { message: string; path: string };
  assert.equal(body.message, "Hello from Tysel");
  assert.equal(body.path, "/hello");
});
"#;

const GITIGNORE: &str = "node_modules/\ndist/\ndata/\n.tysel/\n";

pub fn run(path: &Path) -> Result<()> {
    let root = if path.as_os_str().is_empty() { Path::new(".") } else { path };
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| *name != "." && *name != "..")
        .unwrap_or("tysel-app");
    if !is_package_name(name) {
        return Err(anyhow!(
            "application name '{name}' must start with a letter and contain only letters, digits, '-' or '_'"
        ));
    }

    let files = [
        (PathBuf::from("package.json"), PACKAGE_JSON.replace("__NAME__", name)),
        (PathBuf::from("tsconfig.json"), TSCONFIG.to_owned()),
        (PathBuf::from("src/index.ts"), INDEX_TS.to_owned()),
        (PathBuf::from("tests/app.test.ts"), TEST_TS.to_owned()),
        (PathBuf::from("tysel.toml"), manifest(name)),
        (PathBuf::from(".gitignore"), GITIGNORE.to_owned()),
    ];

    let conflicts: Vec<_> = files
        .iter()
        .map(|(relative, _)| root.join(relative))
        .filter(|path| path.exists())
        .collect();
    if !conflicts.is_empty() {
        let paths = conflicts.iter().map(|path| path.display().to_string()).collect::<Vec<_>>();
        return Err(anyhow!("refusing to overwrite existing files: {}", paths.join(", ")));
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
    transaction.commit();

    println!("created {name} in {}", root.display());
    println!("next: cd {} && tysel check && tysel test", root.display());
    Ok(())
}

#[derive(Default)]
struct ProjectTransaction {
    created_files: Vec<PathBuf>,
    created_dirs: Vec<PathBuf>,
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
        for path in self.created_dirs.iter().rev() {
            let _ = fs::remove_dir(path);
        }
    }
}

fn manifest(name: &str) -> String {
    format!(
        r#"[app]
name = "{name}"
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
max_response_mb = 4

[durable]
store = "sqlite"
path = "./data/tysel.db"

[observability]
logs = "json"
"#
    )
}

fn is_package_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('a'..='z' | 'A'..='Z'))
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
