use std::fmt;
use std::path::Path;

use oxc::diagnostics::{OxcDiagnostic, Severity};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildDiagnostic {
    pub code: String,
    pub message: String,
    pub severity: DiagnosticSeverity,
    pub phase: String,
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<DiagnosticPosition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<DiagnosticPosition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Advice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticPosition {
    /// One-based line, suitable for terminal output.
    pub line: u32,
    /// One-based Unicode-scalar column, suitable for terminal output.
    pub column: u32,
    /// Zero-based UTF-8 byte offset, suitable for lossless editor conversion.
    pub byte_offset: u32,
}

#[derive(Debug, Clone)]
pub struct BuildDiagnostics {
    diagnostics: Vec<BuildDiagnostic>,
}

impl BuildDiagnostics {
    pub fn new(diagnostics: Vec<BuildDiagnostic>) -> Self {
        Self { diagnostics }
    }

    pub fn diagnostics(&self) -> &[BuildDiagnostic] {
        &self.diagnostics
    }
}

impl fmt::Display for BuildDiagnostics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                formatter.write_str("; ")?;
            }
            if let Some(start) = diagnostic.start {
                write!(
                    formatter,
                    "{} failed for {}:{}:{}: {}",
                    diagnostic.phase, diagnostic.file, start.line, start.column, diagnostic.message
                )?;
            } else {
                write!(
                    formatter,
                    "{} failed for {}: {}",
                    diagnostic.phase, diagnostic.file, diagnostic.message
                )?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for BuildDiagnostics {}

pub(crate) fn from_oxc(
    phase: &str,
    path: &Path,
    source: &str,
    errors: &[OxcDiagnostic],
) -> BuildDiagnostics {
    let line_index = LineIndex::new(source);
    let source_len = u32::try_from(source.len()).unwrap_or(u32::MAX);
    let diagnostics = errors
        .iter()
        .map(|error| {
            let label = error.labels.iter().find(|label| label.primary()).or(error.labels.first());
            let positions = label.map(|label| {
                let start_offset = label.offset().min(source_len);
                let end_offset = label.offset().saturating_add(label.len()).min(source_len);
                (line_index.position(source, start_offset), line_index.position(source, end_offset))
            });
            BuildDiagnostic {
                code: if error.code.is_some() {
                    error.code.to_string()
                } else {
                    format!("TYSEL_{}_ERROR", phase.to_ascii_uppercase())
                },
                message: error.message.to_string(),
                severity: match error.severity {
                    Severity::Error => DiagnosticSeverity::Error,
                    Severity::Warning => DiagnosticSeverity::Warning,
                    Severity::Advice => DiagnosticSeverity::Advice,
                },
                phase: phase.to_owned(),
                file: path.to_string_lossy().into_owned(),
                start: positions.map(|(start, _)| start),
                end: positions.map(|(_, end)| end),
            }
        })
        .collect();
    BuildDiagnostics::new(diagnostics)
}

struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(source: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(source.match_indices('\n').map(|(index, _)| index + 1));
        Self { starts }
    }

    fn position(&self, source: &str, byte_offset: u32) -> DiagnosticPosition {
        let mut offset = (byte_offset as usize).min(source.len());
        while !source.is_char_boundary(offset) {
            offset -= 1;
        }
        let line_index = self.starts.partition_point(|start| *start <= offset).saturating_sub(1);
        let line_start = self.starts[line_index];
        DiagnosticPosition {
            line: line_index as u32 + 1,
            column: source[line_start..offset].chars().count() as u32 + 1,
            byte_offset: offset as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use oxc::diagnostics::OxcDiagnostic;

    use super::{LineIndex, from_oxc};

    #[test]
    fn position_preserves_byte_offset_and_reports_character_column() {
        let source = "const name = '王';\nname";
        let byte_offset = source.rfind("name").unwrap() as u32;
        let position = LineIndex::new(source).position(source, byte_offset);
        assert_eq!(position.line, 2);
        assert_eq!(position.column, 1);
        assert_eq!(position.byte_offset, byte_offset);
    }

    #[test]
    fn diagnostic_without_a_label_does_not_invent_a_source_position() {
        let diagnostics = from_oxc(
            "parse",
            std::path::Path::new("src/index.ts"),
            "export default {};",
            &[OxcDiagnostic::error("missing location")],
        );
        let diagnostic = &diagnostics.diagnostics()[0];
        assert_eq!(diagnostic.start, None);
        assert_eq!(diagnostic.end, None);
        assert_eq!(diagnostics.to_string(), "parse failed for src/index.ts: missing location");
        let json = serde_json::to_value(diagnostic).unwrap();
        assert!(json.get("start").is_none());
        assert!(json.get("end").is_none());
    }
}
