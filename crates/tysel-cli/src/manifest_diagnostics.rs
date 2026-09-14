use std::path::Path;

use anyhow::{Context, Result};
use tysel_build::{BuildDiagnostic, DiagnosticSeverity};
use tysel_manifest::Manifest;

use crate::ErrorFormat;

pub fn load(path: &Path) -> Result<(Manifest, Vec<BuildDiagnostic>)> {
    let (manifest, warnings) = Manifest::from_path_with_warnings(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let diagnostics = warnings
        .into_iter()
        .map(|warning| {
            let mut diagnostic = BuildDiagnostic::at_source(
                warning.code,
                "manifest",
                warning.message,
                &warning.file,
                &warning.source_text,
                warning.range,
            );
            diagnostic.severity = DiagnosticSeverity::Warning;
            diagnostic
        })
        .collect();
    Ok((manifest, diagnostics))
}

/// Stderr is NDJSON in JSON mode; stdout remains the command's result/protocol.
/// Dev supplies a generation, including an empty snapshot to clear old diagnostics.
pub fn report(format: ErrorFormat, diagnostics: &[BuildDiagnostic], generation: Option<u64>) {
    match format {
        ErrorFormat::Human => {
            for diagnostic in diagnostics {
                let location = diagnostic
                    .start
                    .map_or_else(String::new, |start| format!(":{}:{}", start.line, start.column));
                eprintln!(
                    "warning[{}]: {}{}: {}",
                    diagnostic.code, diagnostic.file, location, diagnostic.message
                );
            }
        }
        ErrorFormat::Json if !diagnostics.is_empty() || generation.is_some() => {
            let mut event = serde_json::json!({
                "schemaVersion": 1,
                "event": "diagnostics",
                "diagnostics": diagnostics,
            });
            if let Some(generation) = generation {
                event["generation"] = generation.into();
            }
            eprintln!("{event}");
        }
        ErrorFormat::Json => {}
    }
}
