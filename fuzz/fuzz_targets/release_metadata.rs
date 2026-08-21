#![no_main]

use libfuzzer_sys::fuzz_target;
use tysel_build::{
    ReleaseArtifactSignature, ReleaseEvidenceIndex, ReleaseSignature, ReproducibleBuildEvidence,
    TrustPolicy, validate_trust_policy,
};

fuzz_target!(|data: &[u8]| {
    let _ = serde_json::from_slice::<ReleaseEvidenceIndex>(data);
    let _ = serde_json::from_slice::<ReleaseSignature>(data);
    let _ = serde_json::from_slice::<ReleaseArtifactSignature>(data);
    let _ = serde_json::from_slice::<ReproducibleBuildEvidence>(data);
    if let Ok(policy) = serde_json::from_slice::<TrustPolicy>(data) {
        let _ = validate_trust_policy(&policy);
    }
});
