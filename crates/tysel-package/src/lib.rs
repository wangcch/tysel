//! Versioned Tysel App Package (TAP) trailer used by single-file executables.
//!
//! The current stable envelope keeps a portable payload and explicit hashes.
//! Signatures and release evidence are layered on this versioned contract.

mod sourcemap;
mod tap;

pub use sourcemap::{OriginalPosition, SourceMap, SourceMapWriter, identity_source_map};
pub use tap::{
    MAX_AOT_ARTIFACTS_PER_COMPONENT, MAX_PACKAGED_COMPONENTS, MAX_TAP_PAYLOAD_BYTES,
    MIN_SUPPORTED_TAP_VERSION, PackageError, PackageManifest, PackagedAot, PackagedComponent,
    TAP_COMPATIBILITY_REPORT_VERSION, TAP_VERSION, Tap, TapCompatibilityReport,
    TapCompatibilityStatus, bundle_hash, compatibility_report, default_max_request_bytes,
};

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests;
