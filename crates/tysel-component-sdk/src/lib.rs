//! Guest-side helpers for `tysel:component/task@0.4.0`.
//!
//! The generated WIT binding remains language/toolchain specific. This crate
//! standardizes typed JSON dispatch inside Rust components.

use serde::Serialize;
use serde::de::DeserializeOwned;

pub const ABI_VERSION: &str = "0.4.0";
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_ERROR_BYTES: usize = 4 * 1024;

pub trait Task {
    type Input: DeserializeOwned;
    type Output: Serialize;

    fn run(input: Self::Input) -> Result<Self::Output, String>;
}

/// Decode, invoke, and encode one task with the same byte limits as the host.
/// Guest errors are bounded before crossing the canonical ABI.
pub fn dispatch<T: Task>(input: &str) -> Result<String, String> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(format!("component input exceeds {MAX_INPUT_BYTES} bytes"));
    }
    let input = serde_json::from_str(input).map_err(bounded_error)?;
    let output = T::run(input).map_err(bounded_message)?;
    let output = serde_json::to_string(&output).map_err(bounded_error)?;
    if output.len() > MAX_OUTPUT_BYTES {
        return Err(format!("component output exceeds {MAX_OUTPUT_BYTES} bytes"));
    }
    Ok(output)
}

fn bounded_error(error: impl std::fmt::Display) -> String {
    bounded_message(error.to_string())
}

fn bounded_message(mut message: String) -> String {
    if message.len() > MAX_ERROR_BYTES {
        let mut end = MAX_ERROR_BYTES;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    message
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Deserialize)]
    struct Input {
        value: u32,
    }

    #[derive(Debug, PartialEq, Serialize)]
    struct Output {
        doubled: u32,
    }

    struct Double;

    impl Task for Double {
        type Input = Input;
        type Output = Output;

        fn run(input: Self::Input) -> Result<Self::Output, String> {
            Ok(Output { doubled: input.value * 2 })
        }
    }

    #[test]
    fn dispatches_typed_json() {
        assert_eq!(dispatch::<Double>(r#"{"value":21}"#).unwrap(), r#"{"doubled":42}"#);
    }

    #[test]
    fn rejects_invalid_and_oversized_input() {
        assert!(dispatch::<Double>("invalid").is_err());
        assert!(dispatch::<Double>(&" ".repeat(MAX_INPUT_BYTES + 1)).is_err());
    }

    #[test]
    fn bounds_guest_errors_on_utf8_boundaries() {
        struct Failure;
        impl Task for Failure {
            type Input = ();
            type Output = ();

            fn run(_: ()) -> Result<(), String> {
                Err("好".repeat(MAX_ERROR_BYTES))
            }
        }

        let error = dispatch::<Failure>("null").unwrap_err();
        assert!(error.len() <= MAX_ERROR_BYTES);
        assert!(std::str::from_utf8(error.as_bytes()).is_ok());
    }
}
