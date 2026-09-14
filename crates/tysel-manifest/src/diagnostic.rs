use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{Manifest, ManifestError, ManifestFormat};

/// A non-fatal configuration warning tied to the validated source snapshot.
#[derive(Debug, Clone)]
pub struct ManifestWarning {
    pub file: PathBuf,
    pub source_text: Arc<str>,
    pub code: &'static str,
    pub message: String,
    pub range: Option<Range<usize>>,
}

pub(crate) fn warnings(
    manifest: &Manifest,
    path: &Path,
    raw: &str,
    format: ManifestFormat,
) -> Vec<ManifestWarning> {
    let mut warnings = Vec::new();
    let mut source = None;
    // Never interpolate setting values: even ignored endpoints may contain credentials.
    let mut warn = |field: &[&str], code, message: String| {
        // Clean manifests need neither a source copy nor a location index.
        let (source_text, ranges) =
            source.get_or_insert_with(|| (Arc::<str>::from(raw), FieldRanges::new(raw, format)));
        warnings.push(ManifestWarning {
            file: path.to_path_buf(),
            source_text: Arc::clone(source_text),
            code,
            message,
            range: ranges.get(field),
        });
    };
    if manifest.durable.store != "sqlite" {
        warn(
            &["durable", "store"],
            "TYSEL_CONFIG_UNSUPPORTED_STORE",
            "durable.store only enables the application SQLite capability when exactly \"sqlite\"; this value disables that capability and its default durable event-log path. For durable handlers, host configuration takes precedence: TYSEL_DURABLE_POSTGRES_URL, then TYSEL_DURABLE_SQLITE_PATH, then durable-events.db beside the application database. This field does not select PostgreSQL.".into(),
        );
    }
    if !manifest.observability.logs.eq_ignore_ascii_case("json") {
        warn(
            &["observability", "logs"],
            "TYSEL_CONFIG_JSON_LOGS_DISABLED",
            "observability.logs only enables the runtime JSON logger when it is case-insensitive \"json\"; this value disables that logger in run/dev and packaged services. It does not select another log formatter.".into(),
        );
    }
    for (signal, value, code, endpoint) in [
        (
            "traces",
            &manifest.observability.traces,
            "TYSEL_CONFIG_IGNORED_TRACES",
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        ),
        (
            "metrics",
            &manifest.observability.metrics,
            "TYSEL_CONFIG_IGNORED_METRICS",
            "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
        ),
    ] {
        if value.is_some() {
            warn(
                &["observability", signal],
                code,
                format!(
                    "observability.{signal} is ignored by run/dev and packaged applications. Packaged services use {endpoint} before OTEL_EXPORTER_OTLP_ENDPOINT; OTEL_SDK_DISABLED=true disables export. Local run/dev and Component tasks do not initialize an OTLP exporter. Environment configuration is evaluated on the deployment host, not embedded at build time."
                ),
            );
        }
    }
    warnings
}

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
                ("TYSEL_MANIFEST_INVALID", FieldRanges::new(&raw, format).get(field))
            }
            _ => ("TYSEL_MANIFEST_INVALID", None),
        };
        Self { file: path.to_path_buf(), source_text: raw, code, range, error }
    }
}

type JsonObject<'a> = BTreeMap<String, &'a serde_json::value::RawValue>;

enum FieldRanges<'a> {
    Toml(Option<toml_edit::ImDocument<&'a str>>),
    Json { raw: &'a str, objects: BTreeMap<usize, Option<JsonObject<'a>>> },
}

impl<'a> FieldRanges<'a> {
    fn new(raw: &'a str, format: ManifestFormat) -> Self {
        match format {
            // ImDocument retains parser spans; DocumentMut would discard them.
            ManifestFormat::Toml => Self::Toml(toml_edit::ImDocument::parse(raw).ok()),
            ManifestFormat::Json => Self::Json { raw, objects: BTreeMap::new() },
        }
    }

    fn get(&mut self, field: &[impl AsRef<str>]) -> Option<Range<usize>> {
        match self {
            Self::Toml(document) => {
                let mut item = document.as_ref()?.as_item();
                for part in field {
                    item = item.get(part.as_ref())?;
                }
                item.span()
            }
            Self::Json { raw, objects } => {
                // Borrow original slices and parse each containing object only once.
                let mut value = *raw;
                for part in field {
                    let offset = value.as_ptr() as usize - raw.as_ptr() as usize;
                    let object =
                        objects.entry(offset).or_insert_with(|| serde_json::from_str(value).ok());
                    let next = *object.as_ref()?.get(part.as_ref())?;
                    value = next.get();
                }
                let offset = value.as_ptr() as usize - raw.as_ptr() as usize;
                Some(offset..offset + value.len())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Manifest;

    #[test]
    fn warnings_locate_ignored_values_without_echoing_them() {
        for (format, raw) in [
            (
                ManifestFormat::Toml,
                "# 王😀\napp = { name = 'test', entry = 'index.js' }\ndurable.store = 'SQLite'\nobservability = { logs = 'jsno', traces = 'https://user:secret@collector', metrics = '' }\n",
            ),
            (
                ManifestFormat::Json,
                r#"{"app":{"name":"test","entry":"王😀-index.js"},"durable":{"store":"SQLite"},"observability":{"logs":"jsno","traces":"https://user:secret@collector","metrics":""}}"#,
            ),
            (
                ManifestFormat::Json,
                r#"{"app":{"name":"test","entry":"王😀-index.js"},"durable":{"store":"SQLite"},"observabili\u0074y":{"lo\u0067s":"jsno","traces":"https://user:secret@collector","metrics":""}}"#,
            ),
        ] {
            let manifest = Manifest::parse_with_format(raw, format).unwrap();
            let warnings = warnings(&manifest, Path::new("manifest"), raw, format);
            assert_eq!(warnings.len(), 4);
            for (warning, value) in
                warnings.iter().zip(["SQLite", "jsno", "https://user:secret@collector", ""])
            {
                let literal = &raw[warning.range.clone().expect("explicit value span")];
                assert!(
                    literal == format!("\"{value}\"") || literal == format!("'{value}'"),
                    "{literal}"
                );
                assert!(!warning.message.contains("secret"));
                assert_eq!(warning.source_text.as_ref(), raw);
                assert!(Arc::ptr_eq(&warning.source_text, &warnings[0].source_text));
            }
        }
    }

    #[test]
    fn defaults_null_endpoints_and_mixed_case_json_do_not_warn() {
        for (format, raw) in [
            (ManifestFormat::Toml, "[app]\nname = 'test'\nentry = 'index.js'\n"),
            (
                ManifestFormat::Json,
                r#"{"app":{"name":"test","entry":"index.js"},"observability":{"logs":"JsOn","traces":null,"metrics":null}}"#,
            ),
        ] {
            let manifest = Manifest::parse_with_format(raw, format).unwrap();
            assert!(warnings(&manifest, Path::new("manifest"), raw, format).is_empty());
        }
    }

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
