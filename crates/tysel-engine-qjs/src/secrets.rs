//! Opaque secret handles for the trusted path.
//!
//! JavaScript receives `secret:name`. Raw values stay in the host process and
//! are never returned across the isolate boundary.

use std::collections::HashMap;
use std::sync::RwLock;

use tysel_engine::Value;

static SECRETS: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Replace the process-wide secret map. Tests that never call this keep the
/// open mode, which mints handles without an existence check.
pub fn configure(secrets: HashMap<String, String>) {
    *SECRETS.write().expect("secrets lock") = Some(secrets);
}

pub fn refer(name: &str) -> Result<Value, String> {
    let guard = SECRETS.read().expect("secrets lock");
    Ok(Value::String(handle_for(name, guard.as_ref())?))
}

pub fn resolve(handle: &str) -> Result<String, String> {
    let name = handle.strip_prefix("secret:").ok_or_else(|| "invalid secret handle".to_string())?;
    let guard = SECRETS.read().expect("secrets lock");
    let Some(map) = guard.as_ref() else {
        return Err(format!("unknown secret {name}"));
    };
    map.get(name).cloned().ok_or_else(|| format!("unknown secret {name}"))
}

pub fn handle_for(
    name: &str,
    configured: Option<&HashMap<String, String>>,
) -> Result<String, String> {
    if name.is_empty() || name.contains(':') || name.chars().any(char::is_whitespace) {
        return Err("invalid secret name".into());
    }
    if let Some(map) = configured {
        if !map.contains_key(name) {
            return Err(format!("unknown secret {name}"));
        }
    }
    Ok(format!("secret:{name}"))
}

pub fn load_declared(
    names: &[String],
    file_values: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for name in names {
        if let Ok(value) = std::env::var(name) {
            if !value.is_empty() {
                out.insert(name.clone(), value);
                continue;
            }
        }
        if let Some(value) = file_values.get(name) {
            if !value.is_empty() {
                out.insert(name.clone(), value.clone());
            }
        }
    }
    out
}

pub fn parse_dotenv(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = unquote(value.trim());
        out.insert(key.to_owned(), value);
    }
    out
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return value[1..value.len() - 1].to_owned();
        }
    }
    value.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_mode_mints_a_handle() {
        assert_eq!(handle_for("db", None).unwrap(), "secret:db");
    }

    #[test]
    fn configured_mode_rejects_unknown_names() {
        let map = HashMap::from([("db".into(), "super-secret".into())]);
        assert_eq!(handle_for("db", Some(&map)).unwrap(), "secret:db");
        let err = handle_for("missing", Some(&map)).unwrap_err();
        assert!(err.contains("unknown secret missing"), "{err}");
    }

    #[test]
    fn handle_does_not_embed_the_raw_value() {
        let map = HashMap::from([("db".into(), "super-secret".into())]);
        let handle = handle_for("db", Some(&map)).unwrap();
        assert!(!handle.contains("super-secret"));
    }

    #[test]
    fn rejects_invalid_names() {
        assert!(handle_for("", None).is_err());
        assert!(handle_for("a:b", None).is_err());
        assert!(handle_for("a b", None).is_err());
    }

    #[test]
    fn parse_dotenv_skips_comments_and_strips_quotes() {
        let parsed = parse_dotenv(
            r#"
# comment
OPENAI_API_KEY="sk-test"
export DB=postgres://local
EMPTY=
not-a-pair
"#,
        );
        assert_eq!(parsed.get("OPENAI_API_KEY").map(String::as_str), Some("sk-test"));
        assert_eq!(parsed.get("DB").map(String::as_str), Some("postgres://local"));
        assert_eq!(parsed.get("EMPTY").map(String::as_str), Some(""));
        assert!(!parsed.contains_key("not-a-pair"));
    }

    #[test]
    fn resolve_requires_a_configured_handle() {
        assert!(resolve("db").unwrap_err().contains("invalid secret handle"));
        assert!(resolve("secret:db").unwrap_err().contains("unknown secret db"));
    }

    #[test]
    fn load_declared_uses_file_when_env_is_missing() {
        let name = format!("TYSEL_MISSING_{}", std::process::id());
        let file = HashMap::from([(name.clone(), "from-file".into())]);
        let loaded = load_declared(std::slice::from_ref(&name), &file);
        assert_eq!(loaded.get(&name).map(String::as_str), Some("from-file"));
    }
}
