use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::{ManifestError, ManifestFormat};

/// A manifest failure tied to the exact source snapshot that was validated.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct ManifestSourceError {
    pub file: PathBuf,
    pub source_text: String,
    pub code: &'static str,
    pub range: Option<Range<usize>>,
    #[source]
    pub error: ManifestError,
}

impl ManifestSourceError {
    pub(crate) fn new(
        path: &Path,
        raw: String,
        format: ManifestFormat,
        error: ManifestError,
    ) -> Self {
        let (code, range) = match &error {
            ManifestError::Toml(error) => ("TYSEL_MANIFEST_PARSE_ERROR", error.span()),
            ManifestError::Json(error) => {
                // serde_json reports a one-based byte column; retain an insertion
                // point rather than claiming a whole token is invalid.
                let line_start = raw
                    .split_inclusive('\n')
                    .take(error.line().saturating_sub(1))
                    .map(str::len)
                    .sum::<usize>();
                let offset = (line_start + error.column().saturating_sub(1)).min(raw.len());
                ("TYSEL_MANIFEST_PARSE_ERROR", (error.line() > 0).then_some(offset..offset))
            }
            ManifestError::Field { field, .. } => {
                ("TYSEL_MANIFEST_INVALID", field_range(&raw, format, field))
            }
            _ => ("TYSEL_MANIFEST_INVALID", None),
        };
        Self { file: path.to_path_buf(), source_text: raw, code, range, error }
    }
}

fn field_range(raw: &str, format: ManifestFormat, field: &[String]) -> Option<Range<usize>> {
    match format {
        ManifestFormat::Toml => {
            // ImDocument retains parser spans; DocumentMut would discard them.
            let document = toml_edit::ImDocument::parse(raw).ok()?;
            let mut item = document.as_item();
            for part in field {
                item = item.get(part.as_str())?;
            }
            item.span()
        }
        ManifestFormat::Json => {
            // Borrow RawValue slices to locate nested values without searching
            // for key text (which may also occur in strings or other objects).
            let mut value = raw;
            for part in field {
                let object: BTreeMap<String, &serde_json::value::RawValue> =
                    serde_json::from_str(value).ok()?;
                value = object.get(part)?.get();
            }
            let offset = value.as_ptr() as usize - raw.as_ptr() as usize;
            Some(offset..offset + value.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Manifest;

    #[test]
    fn semantic_ranges_identify_nested_values_in_both_formats() {
        let cases = [
            (
                ManifestFormat::Toml,
                "# 王😀 workers = 0\n[app]\nname = 'test'\nentry = 'index.ts'\n[server]\nworkers = 0\n",
            ),
            (
                ManifestFormat::Toml,
                "app = { name = 'test', entry = 'index.ts' }\nserver.workers = 0\n",
            ),
            (
                ManifestFormat::Json,
                r#"{"app":{"name":"test","entry":"workers-王😀.ts"},"server":{"workers":0}}"#,
            ),
        ];
        for (format, raw) in cases {
            let error = Manifest::parse_with_format(raw, format).unwrap_err();
            let diagnostic =
                ManifestSourceError::new(Path::new("manifest"), raw.to_owned(), format, error);
            assert_eq!(diagnostic.code, "TYSEL_MANIFEST_INVALID");
            assert_eq!(&raw[diagnostic.range.unwrap()], "0");
        }
    }

    #[test]
    fn absent_default_field_has_no_invented_location() {
        for (format, raw) in [
            (ManifestFormat::Toml, "[app]\nname = 'test'\nentry = 'main.wasm'\n"),
            (ManifestFormat::Json, r#"{"app":{"name":"test","entry":"main.wasm"}}"#),
        ] {
            // Explicit field identities also work when the field is absent.
            let error =
                ManifestError::invalid_field(&["app", "profile"], "profile required".into());
            let diagnostic =
                ManifestSourceError::new(Path::new("manifest"), raw.to_owned(), format, error);
            assert_eq!(diagnostic.range, None);
        }
    }

    #[test]
    fn syntax_failures_have_a_stable_parse_code() {
        for (format, raw) in [(ManifestFormat::Toml, "[app"), (ManifestFormat::Json, "{\n  ")] {
            let error = Manifest::parse_with_format(raw, format).unwrap_err();
            let diagnostic =
                ManifestSourceError::new(Path::new("manifest"), raw.to_owned(), format, error);
            assert_eq!(diagnostic.code, "TYSEL_MANIFEST_PARSE_ERROR");
            let range = diagnostic.range.unwrap();
            assert!(range.start <= range.end && range.end <= raw.len());
        }
    }
}
