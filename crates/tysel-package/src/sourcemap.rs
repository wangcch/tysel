use serde::Deserialize;

use crate::tap::PackageError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalPosition {
    pub source: String,
    pub line: u32,
    pub column: u32,
    pub content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SourceMap {
    pub file: Option<String>,
    pub sources: Vec<String>,
    sources_content: Vec<Option<String>>,
    mappings: Vec<Mapping>,
}

#[derive(Debug, Clone, Copy)]
struct Mapping {
    generated_line: u32,
    generated_column: u32,
    source_index: u32,
    original_line: u32,
    original_column: u32,
}

#[derive(Deserialize)]
struct SourceMapFile {
    version: u32,
    file: Option<String>,
    sources: Vec<String>,
    #[serde(rename = "sourcesContent", default)]
    sources_content: Vec<Option<String>>,
    mappings: String,
}

impl SourceMap {
    pub fn parse(json: &[u8]) -> Result<Self, PackageError> {
        let file: SourceMapFile = serde_json::from_slice(json)?;
        if file.version != 3 {
            return Err(PackageError::Invalid(format!(
                "unsupported source map version {}",
                file.version
            )));
        }
        let mappings = parse_mappings(&file.mappings)?;
        Ok(Self {
            file: file.file,
            sources: file.sources,
            sources_content: file.sources_content,
            mappings,
        })
    }

    /// Look up a generated position. `line` and `column` are 1-based, matching JS stacks.
    pub fn original_position(&self, line: u32, column: u32) -> Option<OriginalPosition> {
        let gen_line = line.saturating_sub(1);
        let gen_column = column.saturating_sub(1);
        let mapping = self
            .mappings
            .iter()
            .filter(|mapping| {
                mapping.generated_line == gen_line && mapping.generated_column <= gen_column
            })
            .max_by_key(|mapping| mapping.generated_column)?;
        let source = self.sources.get(mapping.source_index as usize)?.clone();
        let content = self.sources_content.get(mapping.source_index as usize).cloned().flatten();
        Some(OriginalPosition {
            source,
            line: mapping.original_line + 1,
            column: mapping.original_column + 1,
            content,
        })
    }
}

pub fn identity_source_map(source_path: &str, source: &str) -> Result<Vec<u8>, PackageError> {
    let line_count = source.lines().count().max(1);
    let mut mappings = String::new();
    for index in 0..line_count {
        if index == 0 {
            mappings.push_str("AAAA");
        } else {
            mappings.push_str(";AACA");
        }
    }
    let json = serde_json::json!({
        "version": 3,
        "file": "app.js",
        "sources": [source_path],
        "sourcesContent": [source],
        "mappings": mappings,
    });
    Ok(serde_json::to_vec_pretty(&json)?)
}

fn parse_mappings(mappings: &str) -> Result<Vec<Mapping>, PackageError> {
    let mut out = Vec::new();
    let mut source_index = 0i64;
    let mut original_line = 0i64;
    let mut original_column = 0i64;
    for (generated_line, line) in mappings.split(';').enumerate() {
        let mut generated_column = 0i64;
        if line.is_empty() {
            continue;
        }
        let bytes = line.as_bytes();
        let mut offset = 0;
        while offset < bytes.len() {
            generated_column += decode_vlq(bytes, &mut offset)?;
            source_index += decode_vlq(bytes, &mut offset)?;
            original_line += decode_vlq(bytes, &mut offset)?;
            original_column += decode_vlq(bytes, &mut offset)?;
            if offset < bytes.len() && bytes[offset] == b',' {
                offset += 1;
            }
            out.push(Mapping {
                generated_line: generated_line as u32,
                generated_column: generated_column.max(0) as u32,
                source_index: source_index.max(0) as u32,
                original_line: original_line.max(0) as u32,
                original_column: original_column.max(0) as u32,
            });
        }
    }
    Ok(out)
}

fn decode_vlq(bytes: &[u8], offset: &mut usize) -> Result<i64, PackageError> {
    let mut result = 0i64;
    let mut shift = 0;
    loop {
        let digit =
            *bytes.get(*offset).ok_or_else(|| PackageError::Invalid("truncated vlq".into()))?;
        *offset += 1;
        let value = b64(digit).ok_or_else(|| PackageError::Invalid("invalid vlq digit".into()))?;
        result |= i64::from(value & 31) << shift;
        shift += 5;
        if value & 32 == 0 {
            break;
        }
    }
    let signed = if result & 1 != 0 { -(result >> 1) } else { result >> 1 };
    Ok(signed)
}

fn b64(digit: u8) -> Option<u8> {
    match digit {
        b'A'..=b'Z' => Some(digit - b'A'),
        b'a'..=b'z' => Some(digit - b'a' + 26),
        b'0'..=b'9' => Some(digit - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}
