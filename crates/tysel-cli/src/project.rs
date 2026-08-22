use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use tysel_manifest::{Manifest, ManifestFormat};

pub const MANIFEST_NAMES: &[&str] = &["tysel.toml", "tysel.json"];

#[derive(Debug)]
pub struct ProjectLocation {
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest_format: ManifestFormat,
    pub package_json: Option<PathBuf>,
}

impl ProjectLocation {
    pub fn discover(project_dir: Option<&Path>, manifest: Option<&Path>) -> Result<Self> {
        if project_dir.is_some() && manifest.is_some() {
            return Err(anyhow!("-C/--project cannot be combined with --manifest"));
        }

        let invocation_dir = env::current_dir().context("resolve current directory")?;
        let manifest_path = if let Some(path) = manifest {
            let path = absolute_from(&invocation_dir, path);
            if !path.is_file() {
                return Err(anyhow!("manifest file does not exist: {}", path.display()));
            }
            fs::canonicalize(&path)
                .with_context(|| format!("resolve manifest file {}", path.display()))?
        } else {
            let requested = project_dir
                .map(|path| absolute_from(&invocation_dir, path))
                .unwrap_or(invocation_dir);
            if !requested.is_dir() {
                return Err(anyhow!("project directory does not exist: {}", requested.display()));
            }
            let start = fs::canonicalize(&requested)
                .with_context(|| format!("resolve project directory {}", requested.display()))?;
            discover_manifest(&start)?.ok_or_else(|| {
                anyhow!(
                    "no Tysel manifest found from {}; expected {}",
                    start.display(),
                    MANIFEST_NAMES.join(" or ")
                )
            })?
        };

        let manifest_format = ManifestFormat::from_path(&manifest_path)?;
        let root = manifest_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let package = root.join("package.json");
        Ok(Self {
            root,
            manifest_path,
            manifest_format,
            package_json: package.is_file().then_some(package),
        })
    }
}

#[derive(Debug)]
pub struct ProjectContext {
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest_format: ManifestFormat,
    pub manifest: Manifest,
    pub package_json: Option<PathBuf>,
}

impl ProjectContext {
    pub fn discover(project_dir: Option<&Path>, manifest: Option<&Path>) -> Result<Self> {
        let location = ProjectLocation::discover(project_dir, manifest)?;
        let loaded = Manifest::from_path(&location.manifest_path)
            .with_context(|| format!("failed to read {}", location.manifest_path.display()))?;
        Ok(Self {
            root: location.root,
            manifest_path: location.manifest_path,
            manifest_format: location.manifest_format,
            manifest: loaded,
            package_json: location.package_json,
        })
    }
}

pub fn discover_manifest(start: &Path) -> Result<Option<PathBuf>> {
    for directory in start.ancestors() {
        let matches = MANIFEST_NAMES
            .iter()
            .map(|name| directory.join(name))
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => {}
            [path] => return Ok(Some(path.clone())),
            _ => {
                return Err(anyhow!(
                    "multiple Tysel manifests found in {}: {}; keep exactly one",
                    directory.display(),
                    matches
                        .iter()
                        .map(|path| path.file_name().unwrap_or_default().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }
    Ok(None)
}

fn absolute_from(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() { path.to_path_buf() } else { base.join(path) }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_project(name: &str) -> PathBuf {
        let root = env::temp_dir().join(format!(
            "tysel-project-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(root.join("src/nested")).unwrap();
        root
    }

    #[test]
    fn discovers_nearest_json_manifest_from_nested_directory() {
        let root = temp_project("json");
        fs::write(root.join("tysel.json"), "{}").unwrap();
        assert_eq!(
            discover_manifest(&root.join("src/nested")).unwrap(),
            Some(root.join("tysel.json"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_ambiguous_manifest_formats() {
        let root = temp_project("ambiguous");
        fs::write(root.join("tysel.toml"), "").unwrap();
        fs::write(root.join("tysel.json"), "{}").unwrap();
        let error = discover_manifest(&root).unwrap_err().to_string();
        assert!(error.contains("multiple Tysel manifests"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }
}
