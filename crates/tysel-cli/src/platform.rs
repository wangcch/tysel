#[cfg(target_os = "linux")]
use std::ffi::CStr;
#[cfg(target_os = "macos")]
use std::process::Command;

use anyhow::{Context, Result};
use tysel_distribution::{PlatformRequirements, Target};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementCheck {
    pub name: &'static str,
    pub detected: String,
    pub required: String,
    pub compatible: bool,
}

pub fn evaluate(
    requirements: &PlatformRequirements,
    target: Target,
) -> Result<Vec<RequirementCheck>> {
    let mut checks = Vec::new();
    match target {
        Target::LinuxX64 | Target::LinuxArm64 => {
            if let Some(required) = &requirements.minimum_glibc {
                let detected = glibc_version()?;
                checks.push(check("glibc", detected, required)?);
            }
            if let Some(required) = &requirements.minimum_kernel {
                let detected = linux_kernel_version()?;
                checks.push(check("Linux kernel", detected, required)?);
            }
        }
        Target::DarwinX64 | Target::DarwinArm64 => {
            if let Some(required) = &requirements.minimum_macos {
                let detected = macos_version()?;
                checks.push(check("macOS", detected, required)?);
            }
        }
        Target::Unsupported => anyhow::bail!("unsupported platform"),
    }
    Ok(checks)
}

pub fn ensure_compatible(requirements: &PlatformRequirements, target: Target) -> Result<()> {
    for check in evaluate(requirements, target)? {
        anyhow::ensure!(
            check.compatible,
            "{} {} is required; detected {}",
            check.name,
            check.required,
            check.detected
        );
    }
    Ok(())
}

fn check(name: &'static str, detected: String, required: &str) -> Result<RequirementCheck> {
    let compatible = numeric_version(&detected)? >= numeric_version(required)?;
    Ok(RequirementCheck { name, detected, required: required.into(), compatible })
}

fn numeric_version(value: &str) -> Result<Vec<u64>> {
    let value = value.trim().trim_start_matches(|character: char| !character.is_ascii_digit());
    let numeric = value
        .split(|character: char| !character.is_ascii_digit() && character != '.')
        .next()
        .unwrap_or_default();
    let mut parts = numeric
        .split('.')
        .map(|part| part.parse::<u64>().context("invalid numeric platform version"))
        .collect::<Result<Vec<_>>>()?;
    anyhow::ensure!(!parts.is_empty() && parts.len() <= 4, "invalid platform version {value}");
    parts.resize(4, 0);
    Ok(parts)
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn glibc_version() -> Result<String> {
    // SAFETY: glibc returns a process-lifetime, NUL-terminated version string.
    let version = unsafe { CStr::from_ptr(libc::gnu_get_libc_version()) };
    Ok(version.to_str().context("glibc version is not UTF-8")?.into())
}

#[cfg(not(target_os = "linux"))]
fn glibc_version() -> Result<String> {
    anyhow::bail!("glibc version is unavailable on this platform")
}

#[cfg(target_os = "linux")]
fn linux_kernel_version() -> Result<String> {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|value| value.trim().to_owned())
        .context("read Linux kernel version")
}

#[cfg(not(target_os = "linux"))]
fn linux_kernel_version() -> Result<String> {
    anyhow::bail!("Linux kernel version is unavailable on this platform")
}

#[cfg(target_os = "macos")]
fn macos_version() -> Result<String> {
    let output = Command::new("/usr/bin/sw_vers").arg("-productVersion").output()?;
    anyhow::ensure!(output.status.success(), "sw_vers rejected product version query");
    Ok(String::from_utf8(output.stdout)?.trim().into())
}

#[cfg(not(target_os = "macos"))]
fn macos_version() -> Result<String> {
    anyhow::bail!("macOS version is unavailable on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_versions_compare_components_instead_of_text() {
        assert!(numeric_version("6.10.1-arch").unwrap() > numeric_version("6.8").unwrap());
        assert_eq!(numeric_version("glibc 2.39").unwrap(), numeric_version("2.39.0").unwrap());
        assert!(numeric_version("13.0").is_ok());
        assert!(numeric_version("unknown").is_err());
    }
}
