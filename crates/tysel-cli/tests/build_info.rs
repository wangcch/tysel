use std::process::Command;

use tysel_distribution::{BUILD_INFO_SCHEMA_VERSION, BuildInfo, Target};

#[test]
fn cli_reports_version_and_build_identity_without_a_subcommand() {
    let version = Command::new(env!("CARGO_BIN_EXE_tysel")).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        format!("tysel {}", env!("CARGO_PKG_VERSION"))
    );

    let output =
        Command::new(env!("CARGO_BIN_EXE_tysel")).arg("--build-info-json").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let info: BuildInfo = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(info.schema_version, BUILD_INFO_SCHEMA_VERSION);
    assert_eq!(info.binary, "tysel");
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.target, Target::current().canonical());
}
