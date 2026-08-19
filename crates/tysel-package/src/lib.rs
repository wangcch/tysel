//! Tysel App Package (TAP) trailer used by the single-file spike.
//!
//! Spike C appends an uncompressed TAP blob and a 16-byte footer to a runtime
//! stub. CBOR, zstd, and signatures stay out of this spike.

mod sourcemap;
mod tap;

pub use sourcemap::{OriginalPosition, SourceMap, identity_source_map};
pub use tap::{default_max_request_bytes, PackageError, PackageManifest, TAP_VERSION, Tap};

pub fn crate_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

#[cfg(test)]
mod tests;
