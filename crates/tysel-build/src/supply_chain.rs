use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use tysel_package::{Tap, bundle_hash};

const RUNTIME_INVENTORY: &[u8] = include_bytes!("runtime-components.json");
pub const SUPPLY_CHAIN_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeInventory {
    pub inventory_version: u32,
    pub cargo_lock_sha256: String,
    pub roots: Vec<String>,
    pub components: Vec<SupplyChainComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyChainComponent {
    pub name: String,
    pub version: String,
    pub license: String,
    pub purl: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CycloneDxBom {
    pub bom_format: String,
    pub spec_version: String,
    pub version: u32,
    pub metadata: BomMetadata,
    pub components: Vec<BomComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BomMetadata {
    pub component: BomComponent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BomComponent {
    #[serde(rename = "type")]
    pub component_type: String,
    pub bom_ref: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hashes: Vec<BomHash>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub licenses: Vec<BomLicenseChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purl: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BomHash {
    pub alg: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BomLicenseChoice {
    pub expression: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LicenseInventory {
    pub inventory_version: u32,
    pub cargo_lock_sha256: String,
    pub components: Vec<LicensedComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LicensedComponent {
    pub name: String,
    pub version: String,
    pub license: String,
    pub purl: String,
}

pub fn embedded_runtime_inventory() -> Result<RuntimeInventory> {
    let inventory: RuntimeInventory = serde_json::from_slice(RUNTIME_INVENTORY)
        .context("embedded runtime supply-chain inventory is invalid")?;
    validate_inventory(&inventory)?;
    Ok(inventory)
}

pub(crate) fn release_supply_chain(
    tap: &Tap,
    artifact_sha256: &str,
) -> Result<(CycloneDxBom, LicenseInventory)> {
    let inventory = embedded_runtime_inventory()?;
    let application = BomComponent {
        component_type: "application".into(),
        bom_ref: format!("tysel-application:{artifact_sha256}"),
        name: tap.manifest.application_id.clone(),
        version: None,
        hashes: vec![BomHash { alg: "SHA-256".into(), content: artifact_sha256.into() }],
        licenses: Vec::new(),
        purl: None,
    };
    let components = inventory
        .components
        .iter()
        .map(|component| BomComponent {
            component_type: "library".into(),
            bom_ref: component.purl.clone(),
            name: component.name.clone(),
            version: Some(component.version.clone()),
            hashes: component
                .checksum
                .iter()
                .map(|checksum| BomHash { alg: "SHA-256".into(), content: checksum.clone() })
                .collect(),
            licenses: vec![BomLicenseChoice { expression: component.license.clone() }],
            purl: Some(component.purl.clone()),
        })
        .collect();
    let licenses = LicenseInventory {
        inventory_version: SUPPLY_CHAIN_VERSION,
        cargo_lock_sha256: inventory.cargo_lock_sha256,
        components: inventory
            .components
            .into_iter()
            .map(|component| LicensedComponent {
                name: component.name,
                version: component.version,
                license: component.license,
                purl: component.purl,
            })
            .collect(),
    };
    Ok((
        CycloneDxBom {
            bom_format: "CycloneDX".into(),
            spec_version: "1.5".into(),
            version: 1,
            metadata: BomMetadata { component: application },
            components,
        },
        licenses,
    ))
}

fn validate_inventory(inventory: &RuntimeInventory) -> Result<()> {
    ensure!(inventory.inventory_version == SUPPLY_CHAIN_VERSION, "unsupported inventory version");
    ensure!(inventory.cargo_lock_sha256.len() == 64, "invalid Cargo.lock digest");
    ensure!(!inventory.roots.is_empty(), "runtime inventory has no roots");
    ensure!(!inventory.components.is_empty(), "runtime inventory has no components");
    let mut previous = None;
    for component in &inventory.components {
        ensure!(!component.license.trim().is_empty(), "{} has no license", component.purl);
        ensure!(component.purl.starts_with("pkg:cargo/"), "invalid component purl");
        if let Some(previous) = previous {
            ensure!(previous < component.purl.as_str(), "components are duplicated or unsorted");
        }
        previous = Some(component.purl.as_str());
    }
    Ok(())
}

pub(crate) fn inventory_digest() -> String {
    bundle_hash(RUNTIME_INVENTORY)
}

pub(crate) fn embedded_runtime_inventory_bytes() -> &'static [u8] {
    RUNTIME_INVENTORY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_inventory_is_complete_and_sorted() {
        let inventory = embedded_runtime_inventory().unwrap();
        assert!(inventory.components.iter().any(|component| component.name == "tysel-runtime"));
        assert!(inventory.components.iter().any(|component| component.name == "tokio"));
        assert!(inventory.components.iter().all(|component| !component.license.is_empty()));
    }

    #[test]
    fn inventory_validation_fails_closed_on_missing_licenses_and_duplicates() {
        let mut inventory = embedded_runtime_inventory().unwrap();
        inventory.components[0].license.clear();
        assert!(validate_inventory(&inventory).unwrap_err().to_string().contains("has no license"));

        let mut inventory = embedded_runtime_inventory().unwrap();
        inventory.components[1].purl = inventory.components[0].purl.clone();
        assert!(
            validate_inventory(&inventory)
                .unwrap_err()
                .to_string()
                .contains("duplicated or unsorted")
        );
    }
}
