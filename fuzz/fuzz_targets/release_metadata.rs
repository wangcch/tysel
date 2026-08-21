#![no_main]

use libfuzzer_sys::fuzz_target;
use tysel_build::{ReleaseEvidenceIndex, ReleaseSignature, TrustPolicy, validate_trust_policy};

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<ReleaseEvidenceIndex>(data);
    let _ = serde_json::from_slice::<ReleaseSignature>(data);
    if let Ok(policy) = serde_json::from_slice::<TrustPolicy>(data) {
        let _ = validate_trust_policy(&policy);
    }
});
