use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value as JsonValue, json};
use tysel_engine::{IsolateConfig, Value};
use tysel_manifest::Manifest;
use tysel_package::SourceMap;

static RUNNER_ID: AtomicU64 = AtomicU64::new(0);

const HARNESS: &str = r#"
const __tysel_tests = [];
globalThis.test = function test(name, body) {
  if (typeof name !== "string" || !name) throw new TypeError("test name must be a non-empty string");
  if (typeof body !== "function") throw new TypeError("test body must be a function");
  __tysel_tests.push({ name, body });
};
globalThis.__tysel_test_register = globalThis.test;
function __tysel_format(value) {
  try { return JSON.stringify(value); } catch (_) { return String(value); }
}
function __tysel_assert(condition, message) {
  if (!condition) throw new Error(message || "assertion failed");
}
__tysel_assert.equal = function equal(actual, expected, message) {
  if (!Object.is(actual, expected)) {
    throw new Error(message || `expected ${__tysel_format(expected)}, received ${__tysel_format(actual)}`);
  }
};
__tysel_assert.deepEqual = function deepEqual(actual, expected, message) {
  const left = __tysel_format(actual);
  const right = __tysel_format(expected);
  if (left !== right) throw new Error(message || `expected ${right}, received ${left}`);
};
globalThis.assert = __tysel_assert;
globalThis.__tysel_assert = __tysel_assert;
"#;

const RUNNER: &str = r#"
export default {
  tasks: {
    __tysel_test_list__: {
      kind: "mcp",
      description: "Discover Tysel tests",
      input: {},
      handler() {
        return __tysel_tests.map((item) => item.name);
      },
    },
    __tysel_test_run__: {
      kind: "mcp",
      description: "Run one Tysel test in an isolated runtime",
      input: { index: "number" },
      async handler(input) {
        const index = Number(input && input.index);
        const item = __tysel_tests[index];
        if (!item) throw new RangeError(`test index ${index} is out of range`);
        try {
          await item.body();
          return { name: item.name, status: "passed" };
        } catch (error) {
          const message = String(error);
          const stack = error && error.stack ? String(error.stack) : "";
          return {
            name: item.name,
            status: "failed",
            error: stack.includes(message) ? stack : `${message}${stack ? `\n${stack}` : ""}`,
          };
        }
      },
    },
  },
};
"#;

pub fn run(
    manifest_path: &Path,
    requested: &[PathBuf],
    timeout_ms: u64,
    json_output: bool,
) -> Result<()> {
    if timeout_ms == 0 {
        return Err(anyhow!("--timeout-ms must be greater than zero"));
    }
    let manifest = Manifest::from_path(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let root = manifest_path.parent().unwrap_or(Path::new("."));
    configure_host(&manifest, root)?;
    let roots = if requested.is_empty() { vec![root.join("tests")] } else { requested.to_vec() };
    let mut files = Vec::new();
    for path in roots {
        let path = if path.is_absolute() { path } else { root.join(path) };
        discover(&path, &mut files)?;
    }
    files.sort();
    files.dedup();
    if files.is_empty() {
        return Err(anyhow!("no test files found (expected *.test.ts or *.test.js)"));
    }

    let mut reports = Vec::new();
    for file in &files {
        reports.push(run_file(file, &manifest, timeout_ms)?);
    }
    let passed = reports.iter().map(|report| count(&report["passed"])).sum::<u64>();
    let failed = reports.iter().map(|report| count(&report["failed"])).sum::<u64>();
    let report = json!({
        "schemaVersion": 1,
        "passed": passed,
        "failed": failed,
        "files": reports,
    });
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human(&report);
    }
    if failed > 0 {
        return Err(anyhow!("{failed} test(s) failed"));
    }
    Ok(())
}

fn configure_host(manifest: &Manifest, root: &Path) -> Result<()> {
    tysel_engine_qjs::configure_execution_profile(&manifest.app.profile);
    tysel_engine_qjs::configure_fetch_hosts(manifest.permissions.fetch.clone());
    tysel_engine_qjs::configure_sqlite_path(
        if manifest.durable.store == "sqlite" { &manifest.durable.path } else { "" },
        Some(root),
    );
    tysel_engine_qjs::configure_fs(
        manifest.permissions.fs_read.clone(),
        manifest.permissions.fs_write.clone(),
        Some(root),
    );
    let file_values = fs::read_to_string(root.join(".env"))
        .ok()
        .map(|text| tysel_engine_qjs::parse_dotenv(&text))
        .unwrap_or_default();
    let postgres = tysel_manifest::resolve_postgres(&manifest.permissions.postgres, &file_values);
    tysel_engine_qjs::configure_postgres(
        postgres.as_ref().map(|config| config.url.clone()),
        postgres.is_some_and(|config| config.read_only),
    );
    let redis = tysel_manifest::resolve_redis(&manifest.permissions.redis, &file_values);
    tysel_engine_qjs::configure_redis(
        redis.as_ref().map(|config| config.url.clone()),
        redis.is_some_and(|config| config.read_only),
    );
    tysel_engine_qjs::configure_secrets(tysel_engine_qjs::load_declared(
        &manifest.permissions.secrets,
        &file_values,
    ));
    tysel_runtime::configure_llm_from_env(manifest.limits.request_timeout_ms)?;
    Ok(())
}

fn discover(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if path.is_file() {
        if is_test_file(path) {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
        let entry = entry?;
        let child = entry.path();
        if child.is_dir() {
            if entry.file_name() != "node_modules" && entry.file_name() != ".git" {
                discover(&child, files)?;
            }
        } else if is_test_file(&child) {
            files.push(child);
        }
    }
    Ok(())
}

fn is_test_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else { return false };
    [".test.ts", ".test.mts", ".test.js", ".test.mjs"].iter().any(|suffix| name.ends_with(suffix))
}

fn run_file(path: &Path, manifest: &Manifest, timeout_ms: u64) -> Result<JsonValue> {
    let source = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let runner_path = temporary_runner_path(path);
    let temporary = TemporaryFile(runner_path.clone());
    let prefix = format!("{HARNESS}\n");
    let source_start_line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32 + 1;
    fs::write(&runner_path, format!("{prefix}{source}\n{RUNNER}"))
        .with_context(|| format!("write temporary runner for {}", path.display()))?;
    let (bundle, source_map) = tysel_build::read_bundle(&runner_path)
        .with_context(|| format!("bundle test {}", path.display()))?;
    let source_map = SourceMap::parse(&source_map).context("parse test source map")?;
    let bundle = String::from_utf8(bundle).context("test bundle was not UTF-8")?;
    let config = IsolateConfig {
        memory_limit_bytes: (manifest.limits.memory_mb as usize).saturating_mul(1024 * 1024),
        cpu_ms_per_turn: timeout_ms,
        request_timeout_ms: timeout_ms,
    };
    let list_deadline_ms = deadline_ms(timeout_ms)?;
    let names = tysel_engine_qjs::invoke_task_module(
        &bundle,
        "__tysel_test_list__",
        "{}",
        "tysel-test",
        list_deadline_ms,
        config,
    )
    .map_err(|error| {
        let message = symbolicate_stack(
            &error.to_string(),
            &source_map,
            &runner_path,
            path,
            source_start_line,
            source.lines().count() as u32,
        );
        anyhow!("discover tests in {}: {message}", path.display())
    })?;
    let names = engine_value_to_json(names)
        .as_array()
        .cloned()
        .ok_or_else(|| anyhow!("test discovery returned a non-array"))?;
    if names.len() > 1024 {
        return Err(anyhow!("test file registers more than 1024 tests"));
    }
    let mut tests = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let name = name.as_str().unwrap_or("unnamed test").to_owned();
        let input = serde_json::to_string(&json!({ "index": index }))?;
        let result = tysel_engine_qjs::invoke_task_module(
            &bundle,
            "__tysel_test_run__",
            &input,
            "tysel-test",
            deadline_ms(timeout_ms)?,
            config,
        );
        let mut result = match result {
            Ok(value) => engine_value_to_json(value),
            Err(error) => {
                let raw = error.to_string();
                let error = if raw.to_ascii_lowercase().contains("timeout") {
                    format!("test timed out after {timeout_ms}ms")
                } else {
                    symbolicate_stack(
                        &raw,
                        &source_map,
                        &runner_path,
                        path,
                        source_start_line,
                        source.lines().count() as u32,
                    )
                };
                json!({ "name": name, "status": "failed", "error": error })
            }
        };
        if let Some(stack) = result.get("error").and_then(JsonValue::as_str).map(str::to_owned) {
            result["error"] = JsonValue::String(symbolicate_stack(
                &stack,
                &source_map,
                &runner_path,
                path,
                source_start_line,
                source.lines().count() as u32,
            ));
        }
        tests.push(result);
    }
    let passed = tests.iter().filter(|test| test["status"] == "passed").count();
    let failed = tests.len().saturating_sub(passed);
    drop(temporary);
    Ok(json!({
        "path": path.display().to_string(),
        "passed": passed,
        "failed": failed,
        "tests": tests,
    }))
}

fn deadline_ms(timeout_ms: u64) -> Result<u64> {
    SystemTime::now()
        .checked_add(Duration::from_millis(timeout_ms))
        .and_then(|deadline| deadline.duration_since(SystemTime::UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .ok_or_else(|| anyhow!("test deadline is outside the supported range"))
}

fn symbolicate_stack(
    stack: &str,
    map: &SourceMap,
    runner_path: &Path,
    test_path: &Path,
    source_start_line: u32,
    source_line_count: u32,
) -> String {
    stack
        .lines()
        .map(|line| {
            let Some(marker) = line.find("app.js:") else { return line.to_owned() };
            let coordinates = &line[marker + "app.js:".len()..];
            let Some((generated_line, rest)) = parse_number(coordinates) else {
                return line.to_owned();
            };
            let Some(rest) = rest.strip_prefix(':') else { return line.to_owned() };
            let Some((generated_column, _)) = parse_number(rest) else { return line.to_owned() };
            let Some(position) = map.original_position(generated_line, generated_column) else {
                return line.to_owned();
            };
            let runner_matches = Path::new(&position.source) == runner_path
                || Path::new(&position.source).file_name() == runner_path.file_name();
            let (source, original_line) = if runner_matches
                && position.line >= source_start_line
                && position.line < source_start_line.saturating_add(source_line_count)
            {
                (test_path.display().to_string(), position.line - source_start_line + 1)
            } else {
                (position.source, position.line)
            };
            let original = format!("{source}:{original_line}:{}", position.column);
            let token_len = "app.js:".len()
                + generated_line.to_string().len()
                + 1
                + generated_column.to_string().len();
            format!("{}{}{}", &line[..marker], original, &line[marker + token_len..])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_number(value: &str) -> Option<(u32, &str)> {
    let digits = value.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    Some((value[..digits].parse().ok()?, &value[digits..]))
}

fn count(value: &JsonValue) -> u64 {
    value.as_u64().or_else(|| value.as_f64().map(|value| value as u64)).unwrap_or(0)
}

fn temporary_runner_path(path: &Path) -> PathBuf {
    let id = RUNNER_ID.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("test.ts");
    path.with_file_name(format!(".tysel-{name}-{}-{id}.ts", std::process::id()))
}

struct TemporaryFile(PathBuf);

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn engine_value_to_json(value: Value) -> JsonValue {
    match value {
        Value::Null => JsonValue::Null,
        Value::Bool(value) => json!(value),
        Value::Number(value) => json!(value),
        Value::String(value) => json!(value),
        Value::Bytes(value) => json!(value),
        Value::Array(values) => {
            JsonValue::Array(values.into_iter().map(engine_value_to_json).collect())
        }
        Value::Record(fields) => JsonValue::Object(
            fields.into_iter().map(|(key, value)| (key, engine_value_to_json(value))).collect(),
        ),
    }
}

fn print_human(report: &JsonValue) {
    for file in report["files"].as_array().into_iter().flatten() {
        println!("{}", file["path"].as_str().unwrap_or("test"));
        for test in file["tests"].as_array().into_iter().flatten() {
            let status = if test["status"] == "passed" { "ok" } else { "fail" };
            println!("  {status:<4} {}", test["name"].as_str().unwrap_or("unnamed test"));
            if let Some(error) = test["error"].as_str() {
                for line in error.lines() {
                    println!("       {line}");
                }
            }
        }
    }
    println!("\n{} passed, {} failed", report["passed"], report["failed"]);
}
