#![no_main]

use libfuzzer_sys::fuzz_target;
use tysel_package::{PackageManifest, Tap};

fuzz_target!(|data: &[u8]| {
    let _ = Tap::decode(data);

    let tap = Tap::new(
        PackageManifest {
            format_version: 0,
            runtime_version: "1.0.0".into(),
            application_id: "fuzz-app".into(),
            entrypoint: "src/index.js".into(),
            execution_profile: "service".into(),
            listen: "127.0.0.1:3000".into(),
            memory_limit_bytes: 128 * 1024 * 1024,
            cpu_ms_per_turn: 50,
            request_timeout_ms: 30_000,
            bundle_hash: String::new(),
            max_request_bytes: 16 * 1024 * 1024,
            websocket: false,
            sqlite_path: String::new(),
            secret_names: Vec::new(),
            fetch_hosts: Vec::new(),
            postgres: Vec::new(),
            fs_read: Vec::new(),
            fs_write: Vec::new(),
            json_logs: true,
        },
        data.iter().copied().take(64 * 1024).collect(),
        Vec::new(),
    );
    if let Ok(encoded) = tap.encode() {
        let decoded = Tap::decode(&encoded).expect("freshly encoded TAP must decode");
        assert_eq!(decoded.bundle, tap.bundle);
    }
});
