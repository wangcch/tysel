//! Outbound fetch host allowlist for the trusted path.
//!
//! Unconfigured processes (unit tests) allow any host. After `configure`, an
//! empty list denies every outbound URL.

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
}
