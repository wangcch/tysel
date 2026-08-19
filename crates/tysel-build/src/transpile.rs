use std::path::Path;

use anyhow::{Context, Result, anyhow};
use oxc::allocator::Allocator;
use oxc::codegen::{Codegen, CodegenOptions};
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use oxc::transformer::{TransformOptions, Transformer};
use serde_json::{Value, json};

pub fn transpile_typescript(path: &Path, source: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let (code, map_json) = emit_javascript(path, source, true)?;
    let map = map_json.ok_or_else(|| anyhow!("oxc codegen did not emit a source map"))?;
    let mut map_json: Value = serde_json::from_str(&map).context("parse oxc source map")?;
    if let Some(object) = map_json.as_object_mut() {
        object.insert("sourcesContent".into(), json!([source]));
        if !object.contains_key("sources") {
            object.insert("sources".into(), json!([path.to_string_lossy()]));
        }
    }
    Ok((code.into_bytes(), serde_json::to_vec_pretty(&map_json)?))
}

pub(crate) fn to_javascript(path: &Path, source: &str) -> Result<String> {
    Ok(emit_javascript(path, source, false)?.0)
}

pub(crate) fn source_type_for(path: &Path) -> Result<SourceType> {
    let source_type = SourceType::from_path(path)
        .map_err(|err| anyhow!("cannot infer source type for {}: {err}", path.display()))?;
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    Ok(match ext {
        "cjs" | "cts" => source_type.with_commonjs(true),
        _ => source_type.with_module(true),
    })
}

fn emit_javascript(
    path: &Path,
    source: &str,
    source_map: bool,
) -> Result<(String, Option<String>)> {
    let source_type = source_type_for(path)?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(anyhow!(format_diagnostics("parse", path, &parsed.diagnostics)));
    }
    let mut program = parsed.program;
    if source_type.is_typescript() {
        let semantic = SemanticBuilder::new().build(&program);
        if !semantic.diagnostics.is_empty() {
            return Err(anyhow!(format_diagnostics("semantic", path, &semantic.diagnostics)));
        }
        let scoping = semantic.semantic.into_scoping();
        let options = TransformOptions::default();
        let transformed =
            Transformer::new(&allocator, path, &options).build_with_scoping(scoping, &mut program);
        if !transformed.diagnostics.is_empty() {
            return Err(anyhow!(format_diagnostics("transform", path, &transformed.diagnostics)));
        }
    }

    let mut codegen = Codegen::new().with_source_text(source);
    if source_map {
        codegen = codegen.with_options(CodegenOptions {
            source_map_path: Some(path.to_path_buf()),
            ..CodegenOptions::default()
        });
    }
    let codegen = codegen.build(&program);
    let map = codegen.map.as_ref().map(|map| map.to_json_string());
    Ok((codegen.code, map))
}

pub(crate) fn format_diagnostics(
    stage: &str,
    path: &Path,
    errors: &[oxc::diagnostics::OxcDiagnostic],
) -> String {
    let details = errors.iter().map(|err| err.to_string()).collect::<Vec<_>>().join("; ");
    format!("{stage} failed for {}: {details}", path.display())
}
