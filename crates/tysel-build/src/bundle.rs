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
use oxc_resolver::{ResolveOptions, Resolver};
use serde_json::json;

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
    let mut originals: Vec<(String, String)> = Vec::new();

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
        originals.push((display_path(&path), source.clone()));

        let id = module_id(&path);
        if is_json(&path) {
            modules.push(CompiledModule {
                id,
                factory: format!("module.exports = {};", source.trim()),
            });
            continue;
        }

        let javascript =
            if needs_transpile(&path) { transpile::to_javascript(&path, &source)? } else { source };
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

        let factory = if analysis.has_module_syntax {
            rewrite_esm(&javascript, &resolved)?
        } else {
            javascript
        };
        modules.push(CompiledModule { id, factory });
    }

    let entry_id = module_id(&canonicalize_existing(entry)?);
    let mut out = String::from(RUNTIME);
    for module in &modules {
        out.push_str("  define(");
        out.push_str(&js_string(&module.id));
        out.push_str(", function (module, exports, require) {\n");
        out.push_str(&indent(&module.factory, 2));
        if !module.factory.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("  });\n");
    }
    out.push_str("  return require;\n})();\n");
    out.push_str("var __tysel_entry = __tysel_require(");
    out.push_str(&js_string(&entry_id));
    out.push_str(");\nexport default __tysel_entry.default !== void 0 ? __tysel_entry.default : __tysel_entry;\n");

    Ok((out.into_bytes(), bundle_source_map(&originals)?))
}

struct CompiledModule {
    id: String,
    factory: String,
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

fn rewrite_esm(javascript: &str, resolved: &HashMap<String, String>) -> Result<String> {
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
        Err(err) => {
            Err(anyhow!("cannot resolve '{}' from {}: {err}", specifier, importer.display()))
        }
    }
}

fn resolved_id(resolved: &HashMap<String, String>, specifier: &str) -> Result<String> {
    resolved.get(specifier).cloned().ok_or_else(|| anyhow!("missing resolved id for '{specifier}'"))
}

fn apply_replacements(source: &str, mut replacements: Vec<(Span, String)>) -> Result<String> {
    replacements.sort_by_key(|(span, _)| span.start);
    let mut out = String::with_capacity(source.len() + 64);
    let mut cursor = 0usize;
    for (span, text) in replacements {
        let start = span.start as usize;
        let end = span.end as usize;
        if start < cursor || end > source.len() || start > end {
            anyhow::bail!("overlapping or invalid module rewrite spans");
        }
        out.push_str(&source[cursor..start]);
        out.push_str(&text);
        if !text.is_empty() && !text.ends_with('\n') {
            out.push('\n');
        }
        cursor = end;
    }
    out.push_str(&source[cursor..]);
    Ok(out)
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

fn indent(source: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    let mut out = String::new();
    for (index, line) in source.lines().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if !line.is_empty() {
            out.push_str(&pad);
            out.push_str(line);
        }
    }
    if source.ends_with('\n') {
        out.push('\n');
    }
    out
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

fn bundle_source_map(originals: &[(String, String)]) -> Result<Vec<u8>> {
    let sources: Vec<&str> = originals.iter().map(|(path, _)| path.as_str()).collect();
    let contents: Vec<&str> = originals.iter().map(|(_, source)| source.as_str()).collect();
    let json = json!({
        "version": 3,
        "file": "bundle.js",
        "sources": sources,
        "sourcesContent": contents,
        "mappings": "AAAA",
    });
    Ok(serde_json::to_vec_pretty(&json)?)
}
