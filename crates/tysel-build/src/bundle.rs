use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use oxc::allocator::Allocator;
use oxc::ast::ast::{
    BindingPattern, Declaration, ExportDefaultDeclarationKind, ImportDeclaration,
    ImportDeclarationSpecifier, ImportOrExportKind, Statement,
};
use oxc::parser::Parser;
use oxc::span::{GetSpan, Span};
use oxc_resolver::{ResolveError, ResolveOptions, Resolver};
use tysel_package::{SourceMap, SourceMapWriter};

use crate::transpile;

const MAX_MODULES: usize = 2048;

const RUNTIME: &str = r#"var __tysel_require = (function () {
  var modules = Object.create(null);
  var cache = Object.create(null);
  function define(id, factory) {
    modules[id] = factory;
  }
  function require(id) {
    if (!Object.prototype.hasOwnProperty.call(modules, id)) {
      throw new Error("Cannot find module " + id);
    }
    if (Object.prototype.hasOwnProperty.call(cache, id)) {
      return cache[id].exports;
    }
    var module = { exports: {} };
    cache[id] = module;
    modules[id].call(module.exports, module, module.exports, require);
    return module.exports;
  }
"#;

pub fn has_runtime_imports(path: &Path, source: &str) -> Result<bool> {
    let analysis = analyze_source(path, source)?;
    Ok(!analysis.specifiers.is_empty())
}

pub fn bundle(entry: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    let resolver = Resolver::new(ResolveOptions {
        condition_names: vec![
            "tysel".into(),
            "import".into(),
            "module".into(),
            "browser".into(),
            "default".into(),
        ],
        main_fields: vec!["module".into(), "browser".into(), "main".into()],
        extensions: vec![
            ".ts".into(),
            ".mts".into(),
            ".tsx".into(),
            ".js".into(),
            ".mjs".into(),
            ".json".into(),
        ],
        extension_alias: vec![
            (".js".into(), vec![".ts".into(), ".tsx".into(), ".js".into()]),
            (".mjs".into(), vec![".mts".into(), ".mjs".into()]),
        ],
        builtin_modules: true,
        ..ResolveOptions::default()
    });

    let mut pending = VecDeque::from([entry.to_path_buf()]);
    let mut seen = HashSet::new();
    let mut modules = Vec::new();

    while let Some(path) = pending.pop_front() {
        let path = canonicalize_existing(&path)?;
        if !seen.insert(path.clone()) {
            continue;
        }
        if seen.len() > MAX_MODULES {
            anyhow::bail!("module graph exceeded {MAX_MODULES} files");
        }

        let source = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let source_path = display_path(&path);

        let id = module_id(&path);
        if is_json(&path) {
            let _: serde_json::Value = serde_json::from_str(&source)
                .with_context(|| format!("invalid JSON module {}", path.display()))?;
            let factory = format!("module.exports = JSON.parse({});", js_string(&source));
            modules.push(CompiledModule {
                id,
                factory,
                source_path,
                source_content: source,
                origins: vec![OrigPos { line: 0, column: 0 }],
                js_to_ts: None,
            });
            continue;
        }

        let (javascript, js_to_ts) = if needs_transpile(&path) {
            let (javascript, map) = transpile::to_javascript(&path, &source)?;
            (javascript, Some(SourceMap::parse(&map)?))
        } else {
            (source.clone(), None)
        };
        let analysis = analyze_source(&path, &javascript)?;
        if !analysis.dynamic_imports.is_empty() {
            anyhow::bail!("dynamic import is not supported in {}", path.display());
        }

        let mut resolved = HashMap::new();
        for specifier in &analysis.specifiers {
            let resolved_path = resolve_specifier(&resolver, &path, specifier)?;
            resolved.insert(specifier.clone(), module_id(&resolved_path));
            pending.push_back(resolved_path);
        }

        let (factory, origins) = if analysis.has_module_syntax {
            rewrite_esm(&javascript, &resolved)?
        } else {
            identity_origins(&javascript)
        };
        modules.push(CompiledModule {
            id,
            factory,
            source_path,
            source_content: source,
            origins,
            js_to_ts,
        });
    }

    let entry_id = module_id(&canonicalize_existing(entry)?);
    emit_bundle(&modules, &entry_id)
}

struct CompiledModule {
    id: String,
    factory: String,
    source_path: String,
    source_content: String,
    origins: Vec<OrigPos>,
    js_to_ts: Option<SourceMap>,
}

#[derive(Clone, Copy)]
struct OrigPos {
    line: u32,
    column: u32,
}

struct Analysis {
    has_module_syntax: bool,
    specifiers: Vec<String>,
    dynamic_imports: Vec<Span>,
}

fn analyze_source(path: &Path, source: &str) -> Result<Analysis> {
    let source_type = transpile::source_type_for(path)?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(anyhow!(transpile::format_diagnostics("parse", path, &parsed.diagnostics)));
    }
    let mut specifiers = Vec::new();
    let mut seen = HashSet::new();
    for (specifier, requests) in parsed.module_record.requested_modules.iter() {
        if requests.iter().all(|request| request.is_type) {
            continue;
        }
        let specifier = specifier.as_str().to_string();
        if seen.insert(specifier.clone()) {
            specifiers.push(specifier);
        }
    }
    Ok(Analysis {
        has_module_syntax: parsed.module_record.has_module_syntax,
        specifiers,
        dynamic_imports: parsed
            .module_record
            .dynamic_imports
            .iter()
            .map(|item| item.span)
            .collect(),
    })
}

fn rewrite_esm(
    javascript: &str,
    resolved: &HashMap<String, String>,
) -> Result<(String, Vec<OrigPos>)> {
    let path = Path::new("module.js");
    let source_type = transpile::source_type_for(path)?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, javascript, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(anyhow!(transpile::format_diagnostics("parse", path, &parsed.diagnostics)));
    }
    if !parsed.module_record.dynamic_imports.is_empty() {
        anyhow::bail!("dynamic import is not supported");
    }

    let mut replacements = Vec::new();
    let mut require_n = 0u32;
    for stmt in &parsed.program.body {
        match stmt {
            Statement::ImportDeclaration(decl) => {
                if decl.import_kind == ImportOrExportKind::Type {
                    replacements.push((decl.span, String::new()));
                    continue;
                }
                replacements.push((decl.span, rewrite_import(decl, resolved, &mut require_n)?));
            }
            Statement::ExportAllDeclaration(decl) => {
                if decl.export_kind == ImportOrExportKind::Type {
                    replacements.push((decl.span, String::new()));
                    continue;
                }
                let id = resolved_id(resolved, decl.source.value.as_str())?;
                let req = format!("require({})", js_string(&id));
                let text = if let Some(exported) = &decl.exported {
                    exports_assign(exported.name().as_str(), &req)
                } else {
                    star_reexport(&req, require_n)
                };
                require_n += 1;
                replacements.push((decl.span, text));
            }
            Statement::ExportFromDeclaration(decl) => {
                if decl.export_kind == ImportOrExportKind::Type {
                    replacements.push((decl.span, String::new()));
                    continue;
                }
                let id = resolved_id(resolved, decl.source.value.as_str())?;
                let tmp = format!("__m{require_n}");
                require_n += 1;
                let mut text = format!("var {tmp} = require({});\n", js_string(&id));
                for spec in &decl.specifiers {
                    if spec.export_kind == ImportOrExportKind::Type {
                        continue;
                    }
                    let local = member_access(&tmp, spec.local.name().as_str());
                    text.push_str(&exports_assign(spec.exported.name().as_str(), &local));
                    text.push('\n');
                }
                replacements.push((decl.span, text));
            }
            Statement::ExportNamedDeclaration(decl) => {
                if decl.export_kind == ImportOrExportKind::Type {
                    replacements.push((decl.span, String::new()));
                    continue;
                }
                let mut text = String::new();
                for spec in &decl.specifiers {
                    if spec.export_kind == ImportOrExportKind::Type {
                        continue;
                    }
                    text.push_str(&exports_assign(
                        spec.exported.name().as_str(),
                        spec.local.name().as_str(),
                    ));
                    text.push('\n');
                }
                replacements.push((decl.span, text));
            }
            Statement::ExportDeclaration(decl) => {
                let inner = decl.declaration.span().source_text(javascript).to_string();
                let mut text = inner;
                text.push('\n');
                for name in declaration_names(&decl.declaration) {
                    text.push_str(&exports_assign(&name, &name));
                    text.push('\n');
                }
                replacements.push((decl.span, text));
            }
            Statement::ExportDefaultDeclaration(decl) => {
                replacements.push((decl.span, rewrite_export_default(decl, javascript)));
            }
            _ => {}
        }
    }
    apply_replacements(javascript, replacements)
}

fn identity_origins(source: &str) -> (String, Vec<OrigPos>) {
    let origins = source
        .lines()
        .enumerate()
        .map(|(line, _)| OrigPos { line: line as u32, column: 0 })
        .collect();
    (source.to_string(), origins)
}

fn rewrite_import(
    decl: &ImportDeclaration<'_>,
    resolved: &HashMap<String, String>,
    require_n: &mut u32,
) -> Result<String> {
    let id = resolved_id(resolved, decl.source.value.as_str())?;
    let Some(specifiers) = &decl.specifiers else {
        return Ok(format!("require({});", js_string(&id)));
    };
    if specifiers.is_empty() {
        return Ok(format!("require({});", js_string(&id)));
    }
    let tmp = format!("__m{require_n}");
    *require_n += 1;
    let mut text = format!("var {tmp} = require({});\n", js_string(&id));
    for spec in specifiers {
        match spec {
            ImportDeclarationSpecifier::ImportDefaultSpecifier(local) => {
                let name = local.local.name.as_str();
                text.push_str(&format!(
                    "var {name} = {tmp}.default !== void 0 ? {tmp}.default : {tmp};\n"
                ));
            }
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(local) => {
                text.push_str(&format!("var {} = {tmp};\n", local.local.name.as_str()));
            }
            ImportDeclarationSpecifier::ImportSpecifier(named) => {
                if named.import_kind == ImportOrExportKind::Type {
                    continue;
                }
                let imported = named.imported.name();
                let local = named.local.name.as_str();
                text.push_str(&format!(
                    "var {local} = {};\n",
                    member_access(&tmp, imported.as_str())
                ));
            }
        }
    }
    Ok(text)
}

fn rewrite_export_default(
    decl: &oxc::ast::ast::ExportDefaultDeclaration<'_>,
    javascript: &str,
) -> String {
    match &decl.declaration {
        ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
            let inner = func.span.source_text(javascript);
            if let Some(id) = &func.id {
                format!("{inner}\nexports.default = {};", id.name.as_str())
            } else {
                format!("exports.default = {inner};")
            }
        }
        ExportDefaultDeclarationKind::ClassDeclaration(class) => {
            let inner = class.span.source_text(javascript);
            if let Some(id) = &class.id {
                format!("{inner}\nexports.default = {};", id.name.as_str())
            } else {
                format!("exports.default = {inner};")
            }
        }
        ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => String::new(),
        _ => {
            let inner = decl.declaration.span().source_text(javascript);
            format!("exports.default = {inner};")
        }
    }
}

fn declaration_names(decl: &Declaration<'_>) -> Vec<String> {
    match decl {
        Declaration::FunctionDeclaration(func) => {
            func.id.iter().map(|id| id.name.as_str().to_string()).collect()
        }
        Declaration::ClassDeclaration(class) => {
            class.id.iter().map(|id| id.name.as_str().to_string()).collect()
        }
        Declaration::VariableDeclaration(vars) => {
            let mut names = Vec::new();
            for declarator in &vars.declarations {
                collect_binding(&declarator.id, &mut names);
            }
            names
        }
        _ => Vec::new(),
    }
}

fn collect_binding(pattern: &BindingPattern<'_>, names: &mut Vec<String>) {
    match pattern {
        BindingPattern::BindingIdentifier(id) => names.push(id.name.as_str().to_string()),
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                collect_binding(&property.value, names);
            }
            if let Some(rest) = &object.rest {
                collect_binding(&rest.argument, names);
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for pattern in array.elements.iter().flatten() {
                collect_binding(pattern, names);
            }
            if let Some(rest) = &array.rest {
                collect_binding(&rest.argument, names);
            }
        }
        BindingPattern::AssignmentPattern(assign) => collect_binding(&assign.left, names),
    }
}

fn resolve_specifier(resolver: &Resolver, importer: &Path, specifier: &str) -> Result<PathBuf> {
    let directory = importer.parent().unwrap_or(importer);
    match resolver.resolve(directory, specifier) {
        Ok(resolution) => canonicalize_existing(&resolution.full_path()),
        Err(ResolveError::Builtin { resolved, .. }) => {
            Err(anyhow!("Node builtin '{resolved}' is not available in Tysel"))
        }
        Err(err) => {
            Err(anyhow!("cannot resolve '{}' from {}: {err}", specifier, importer.display()))
        }
    }
}

fn resolved_id(resolved: &HashMap<String, String>, specifier: &str) -> Result<String> {
    resolved.get(specifier).cloned().ok_or_else(|| anyhow!("missing resolved id for '{specifier}'"))
}

fn apply_replacements(
    source: &str,
    mut replacements: Vec<(Span, String)>,
) -> Result<(String, Vec<OrigPos>)> {
    replacements.sort_by_key(|(span, _)| span.start);
    let mut out = String::with_capacity(source.len() + 64);
    let mut origins = Vec::new();
    let mut at_line_start = true;
    let mut cursor = 0usize;

    let mut append = |text: &str, origin_at: &dyn Fn(usize) -> OrigPos| {
        for (index, ch) in text.char_indices() {
            if at_line_start {
                origins.push(origin_at(index));
                at_line_start = false;
            }
            out.push(ch);
            if ch == '\n' {
                at_line_start = true;
            }
        }
    };

    for (span, text) in replacements {
        let start = span.start as usize;
        let end = span.end as usize;
        if start < cursor || end > source.len() || start > end {
            anyhow::bail!("overlapping or invalid module rewrite spans");
        }
        let copy = &source[cursor..start];
        append(copy, &|index| pos_at(source, cursor + index));
        let span_origin = pos_at(source, start);
        append(&text, &|_| span_origin);
        if !text.is_empty() && !text.ends_with('\n') {
            append("\n", &|_| span_origin);
        }
        cursor = end;
    }
    append(&source[cursor..], &|index| pos_at(source, cursor + index));
    Ok((out, origins))
}

fn pos_at(source: &str, byte: usize) -> OrigPos {
    let byte = byte.min(source.len());
    let mut line = 0u32;
    let mut column = 0u32;
    for ch in source[..byte].chars() {
        if ch == '\n' {
            line += 1;
            column = 0;
        } else {
            column += ch.len_utf16() as u32;
        }
    }
    OrigPos { line, column }
}

fn star_reexport(req: &str, n: u32) -> String {
    format!(
        "var __star{n} = {req};\nfor (var __k{n} in __star{n}) {{\n  if (__k{n} !== \"default\" && Object.prototype.hasOwnProperty.call(__star{n}, __k{n})) exports[__k{n}] = __star{n}[__k{n}];\n}}"
    )
}

fn exports_assign(name: &str, value: &str) -> String {
    if is_identifier(name) {
        format!("exports.{name} = {value};")
    } else {
        format!("exports[{}] = {value};", js_string(name))
    }
}

fn member_access(object: &str, name: &str) -> String {
    if is_identifier(name) {
        format!("{object}.{name}")
    } else {
        format!("{object}[{}]", js_string(name))
    }
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
}

fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
}

fn module_id(path: &Path) -> String {
    display_path(path)
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn canonicalize_existing(path: &Path) -> Result<PathBuf> {
    path.canonicalize().with_context(|| format!("cannot canonicalize {}", path.display()))
}

fn is_json(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("json")
}

fn needs_transpile(path: &Path) -> bool {
    matches!(path.extension().and_then(|ext| ext.to_str()), Some("ts" | "mts" | "cts" | "tsx"))
}

fn emit_bundle(modules: &[CompiledModule], entry_id: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut out = String::new();
    let mut map = SourceMapWriter::new();
    push_unmapped(&mut out, &mut map, RUNTIME);
    for (source_index, module) in modules.iter().enumerate() {
        push_unmapped(
            &mut out,
            &mut map,
            &format!(
                "  define({}, function (module, exports, require) {{\n",
                js_string(&module.id)
            ),
        );
        for (line_index, line) in module.factory.lines().enumerate() {
            out.push_str("  ");
            out.push_str(line);
            if let Some((orig_line, orig_column)) = original_for_factory_line(module, line_index) {
                let generated_column = if line.is_empty() { 0 } else { 2 };
                map.add(generated_column, source_index as u32, orig_line, orig_column);
            }
            out.push('\n');
            map.end_line();
        }
        if module.factory.lines().next().is_none() {
            out.push('\n');
            map.end_line();
        }
        push_unmapped(&mut out, &mut map, "  });\n");
    }
    push_unmapped(&mut out, &mut map, "  return require;\n})();\n");
    push_unmapped(&mut out, &mut map, "var __tysel_entry = __tysel_require(");
    push_unmapped(&mut out, &mut map, &js_string(entry_id));
    push_unmapped(
        &mut out,
        &mut map,
        ");\nexport default __tysel_entry.default !== void 0 ? __tysel_entry.default : __tysel_entry;\n",
    );

    let sources = modules.iter().map(|module| module.source_path.clone()).collect();
    let contents = modules.iter().map(|module| module.source_content.clone()).collect();
    Ok((out.into_bytes(), map.into_json("bundle.js", sources, contents)?))
}

fn original_for_factory_line(module: &CompiledModule, line_index: usize) -> Option<(u32, u32)> {
    let pos = *module.origins.get(line_index)?;
    if let Some(map) = &module.js_to_ts {
        if let Some(found) = map.original_position(pos.line + 1, pos.column + 1) {
            return Some((found.line.saturating_sub(1), found.column.saturating_sub(1)));
        }
    }
    Some((pos.line, pos.column))
}

fn push_unmapped(out: &mut String, map: &mut SourceMapWriter, text: &str) {
    for ch in text.chars() {
        out.push(ch);
        if ch == '\n' {
            map.end_line();
        }
    }
}
