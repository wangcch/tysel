use std::process::Command;

use tysel_distribution::{BuildInfo, Target};

#[test]
fn standalone_service_reports_metadata_before_reading_a_tap() {
    let version =
        Command::new(env!("CARGO_BIN_EXE_tysel-service")).arg("--version").output().unwrap();
    assert!(version.status.success(), "{}", String::from_utf8_lossy(&version.stderr));
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        format!("tysel-service {}", env!("CARGO_PKG_VERSION"))
    );

    let output = Command::new(env!("CARGO_BIN_EXE_tysel-service"))
        .arg("--build-info-json")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let info: BuildInfo = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(info.binary, "tysel-service");
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.target, Target::current().canonical());
}
