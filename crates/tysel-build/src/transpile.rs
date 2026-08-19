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
    let source_type = SourceType::from_path(path)
        .map_err(|err| anyhow!("cannot infer source type for {}: {err}", path.display()))?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(anyhow!(format_diagnostics("parse", path, &parsed.diagnostics)));
    }
    let mut program = parsed.program;
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

    let codegen = Codegen::new()
        .with_source_text(source)
        .with_options(CodegenOptions {
            source_map_path: Some(path.to_path_buf()),
            ..CodegenOptions::default()
        })
        .build(&program);
    let map = codegen.map.ok_or_else(|| anyhow!("oxc codegen did not emit a source map"))?;
    let mut map_json: Value =
        serde_json::from_str(&map.to_json_string()).context("parse oxc source map")?;
    if let Some(object) = map_json.as_object_mut() {
        object.insert("sourcesContent".into(), json!([source]));
        if !object.contains_key("sources") {
            object.insert("sources".into(), json!([path.to_string_lossy()]));
        }
    }
    Ok((codegen.code.into_bytes(), serde_json::to_vec_pretty(&map_json)?))
}

fn format_diagnostics(
    stage: &str,
    path: &Path,
    errors: &[oxc::diagnostics::OxcDiagnostic],
) -> String {
    let details = errors.iter().map(|err| err.to_string()).collect::<Vec<_>>().join("; ");
    format!("{stage} failed for {}: {details}", path.display())
}
