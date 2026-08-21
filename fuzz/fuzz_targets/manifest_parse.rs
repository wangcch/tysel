#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        let _ = tysel_manifest::Manifest::parse(input);
        let _ = tysel_manifest::parse_postgres_grant(input);
    }
});
