//! Outbound fetch policy for the trusted path.
//!
//! Unconfigured processes (unit tests) allow any host. After `configure`, an
//! empty list denies every outbound URL. Header values that are `secret:name`
//! or `Bearer secret:name` are expanded in the host and never returned to JS.

use std::sync::RwLock;

static FETCH_HOSTS: RwLock<Option<Vec<String>>> = RwLock::new(None);

/// Replace the process-wide fetch allowlist. Tests that never call this keep
/// the open mode used by engine unit tests.
pub fn configure(hosts: Vec<String>) {
    *FETCH_HOSTS.write().expect("fetch hosts lock") = Some(hosts);
}

pub fn host_permitted(host: &str) -> Result<(), String> {
    let guard = FETCH_HOSTS.read().expect("fetch hosts lock");
    check_host(host, guard.as_deref())
}

pub fn check_host(host: &str, configured: Option<&[String]>) -> Result<(), String> {
    let Some(allowed) = configured else {
        return Ok(());
    };
    let host = normalize_host(host);
    if host.is_empty() {
        return Err("missing host".into());
    }
    if allowed.iter().any(|pattern| normalize_host(pattern) == host) {
        return Ok(());
    }
    Err(format!("host {host} is not permitted"))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestHeaders {
    pub headers: Vec<(String, String)>,
    pub secret_names: Vec<String>,
}

pub fn expand_headers_json(headers_json: &str) -> Result<RequestHeaders, String> {
    expand_headers_json_with(headers_json, crate::secrets::resolve)
}

pub fn expand_headers_json_with(
    headers_json: &str,
    resolve: impl Fn(&str) -> Result<String, String>,
) -> Result<RequestHeaders, String> {
    if headers_json.is_empty() {
        return Ok(RequestHeaders::default());
    }
    let pairs: Vec<(String, String)> = serde_json::from_str(headers_json)
        .map_err(|err| format!("invalid fetch headers: {err}"))?;
    let mut headers = Vec::with_capacity(pairs.len());
    let mut secret_names = Vec::new();
    for (name, value) in pairs {
        let lower = name.to_ascii_lowercase();
        if skip_request_header(&lower) {
            continue;
        }
        if is_secret_handle_value(&value) && !secret_names.iter().any(|existing| existing == &lower)
        {
            secret_names.push(lower);
        }
        headers.push((name, expand_header_value_with(&value, &resolve)?));
    }
    Ok(RequestHeaders { headers, secret_names })
}

pub fn same_origin(current: &str, next: &str) -> Result<bool, String> {
    Ok(origin_of(current)? == origin_of(next)?)
}

pub fn strip_credentials_for_cross_origin(headers: &mut RequestHeaders) {
    let secret_names = headers.secret_names.clone();
    headers.headers.retain(|(name, _)| {
        let lower = name.to_ascii_lowercase();
        lower != "authorization" && !secret_names.iter().any(|marked| marked == &lower)
    });
}

fn origin_of(url: &str) -> Result<(String, String, u16), String> {
    let uri: hyper::Uri =
        url.parse().map_err(|err: hyper::http::uri::InvalidUri| err.to_string())?;
    let scheme = uri.scheme_str().ok_or("missing scheme")?.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err("outbound fetch only supports http and https".into());
    }
    let host = uri.host().ok_or("missing host")?.to_ascii_lowercase();
    let host = host.trim_end_matches('.').to_owned();
    let port = uri.port_u16().unwrap_or(if scheme == "https" { 443 } else { 80 });
    Ok((scheme, host, port))
}

fn is_secret_handle_value(value: &str) -> bool {
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix("Bearer ") {
        return rest.trim().starts_with("secret:");
    }
    trimmed.starts_with("secret:")
}

pub fn expand_header_value_with(
    value: &str,
    resolve: impl Fn(&str) -> Result<String, String>,
) -> Result<String, String> {
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix("Bearer ") {
        let rest = rest.trim();
        if rest.starts_with("secret:") {
            return Ok(format!("Bearer {}", resolve(rest)?));
        }
    }
    if trimmed.starts_with("secret:") {
        return resolve(trimmed);
    }
    Ok(value.to_owned())
}

pub fn skip_request_header(name: &str) -> bool {
    matches!(
        name,
        "host"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
    )
}

pub fn skip_response_header(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn normalize_host(value: &str) -> String {
    let mut host = value.trim().to_ascii_lowercase();
    if let Some(rest) = host.strip_prefix("https://") {
        host = rest.to_owned();
    } else if let Some(rest) = host.strip_prefix("http://") {
        host = rest.to_owned();
    }
    if let Some((name, _)) = host.split_once('/') {
        host = name.to_owned();
    }
    if let Some(inner) = host.strip_prefix('[').and_then(|rest| rest.split_once(']')) {
        return inner.0.to_owned();
    }
    if let Some((name, _)) = host.rsplit_once(':') {
        if !name.is_empty() && host.chars().filter(|c| *c == ':').count() == 1 {
            host = name.to_owned();
        }
    }
    host.trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_mode_allows_any_host() {
        check_host("api.example.com", None).unwrap();
    }

    #[test]
    fn empty_allowlist_denies() {
        let err = check_host("api.example.com", Some(&[])).unwrap_err();
        assert!(err.contains("api.example.com"), "{err}");
        assert!(err.contains("not permitted"), "{err}");
    }

    #[test]
    fn allowlist_matches_host_only() {
        let allowed = ["api.openai.com".into()];
        check_host("api.openai.com", Some(&allowed)).unwrap();
        check_host("API.OpenAI.com", Some(&allowed)).unwrap();
        check_host("https://api.openai.com/v1", Some(&allowed)).unwrap();
        check_host("api.openai.com:443", Some(&allowed)).unwrap();
        let err = check_host("evil.example", Some(&allowed)).unwrap_err();
        assert!(err.contains("evil.example"), "{err}");
    }

    #[test]
    fn bearer_secret_handle_is_expanded() {
        let expanded = expand_header_value_with("Bearer secret:db", |handle| {
            assert_eq!(handle, "secret:db");
            Ok("raw-token".into())
        })
        .unwrap();
        assert_eq!(expanded, "Bearer raw-token");
        assert!(!expanded.contains("secret:"));
    }

    #[test]
    fn plain_secret_handle_is_expanded() {
        let expanded = expand_header_value_with("secret:db", |_| Ok("raw-token".into())).unwrap();
        assert_eq!(expanded, "raw-token");
    }

    #[test]
    fn ordinary_headers_are_unchanged() {
        let value = expand_header_value_with("application/json", |_| unreachable!()).unwrap();
        assert_eq!(value, "application/json");
    }

    #[test]
    fn substring_secret_is_not_scanned() {
        let value = expand_header_value_with("token secret:db extra", |_| unreachable!()).unwrap();
        assert_eq!(value, "token secret:db extra");
    }

    #[test]
    fn json_headers_skip_host_and_expand_bearer() {
        let out = expand_headers_json_with(
            r#"[["Host","evil"],["Authorization","Bearer secret:db"],["X-Id","1"]]"#,
            |handle| {
                assert_eq!(handle, "secret:db");
                Ok("raw".into())
            },
        )
        .unwrap();
        assert_eq!(
            out.headers,
            vec![("Authorization".into(), "Bearer raw".into()), ("X-Id".into(), "1".into())]
        );
        assert_eq!(out.secret_names, ["authorization"]);
    }

    #[test]
    fn origin_ignores_default_ports_and_path() {
        assert!(same_origin("https://API.Example.com/v1", "https://api.example.com:443/").unwrap());
        assert!(same_origin("http://127.0.0.1/a", "http://127.0.0.1:80/b").unwrap());
    }

    #[test]
    fn origin_differs_on_scheme_or_port() {
        assert!(!same_origin("https://api.example.com/", "http://api.example.com/").unwrap());
        assert!(!same_origin("http://127.0.0.1:1/", "http://127.0.0.1:2/").unwrap());
    }

    #[test]
    fn cross_origin_strips_authorization_and_expanded_secrets() {
        let mut headers = expand_headers_json_with(
            r#"[["Authorization","Bearer secret:db"],["X-Api-Key","secret:db"],["X-Id","1"]]"#,
            |_| Ok("raw".into()),
        )
        .unwrap();
        strip_credentials_for_cross_origin(&mut headers);
        assert_eq!(headers.headers, vec![("X-Id".into(), "1".into())]);
    }

    #[test]
    fn same_origin_keeps_credentials() {
        let headers = expand_headers_json_with(
            r#"[["Authorization","Bearer secret:db"],["X-Api-Key","secret:db"]]"#,
            |_| Ok("raw".into()),
        )
        .unwrap();
        assert_eq!(headers.headers.len(), 2);
    }

    #[test]
    fn hop_by_hop_response_headers_are_skipped() {
        assert!(skip_response_header("connection"));
        assert!(skip_response_header("transfer-encoding"));
        assert!(!skip_response_header("content-type"));
        assert!(!skip_response_header("x-request-id"));
    }
}
