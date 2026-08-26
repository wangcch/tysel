use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(super) struct ProjectTransaction {
    created_files: Vec<PathBuf>,
    created_dirs: Vec<PathBuf>,
    modified_files: Vec<(PathBuf, Vec<u8>)>,
    committed: bool,
}

impl ProjectTransaction {
    pub(super) fn create_dir_all(&mut self, path: &Path) -> std::io::Result<()> {
        let mut missing = Vec::new();
        let mut cursor = path;
        while !cursor.exists() {
            missing.push(cursor.to_path_buf());
            let Some(parent) = cursor.parent() else { break };
            cursor = parent;
        }
        fs::create_dir_all(path)?;
        missing.reverse();
        self.created_dirs.extend(missing);
        Ok(())
    }

    pub(super) fn write(&mut self, path: &Path, contents: &[u8]) -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        self.created_files.push(path.to_path_buf());
        file.write_all(contents)
    }

    pub(super) fn replace(
        &mut self,
        path: &Path,
        expected: &[u8],
        contents: &[u8],
    ) -> std::io::Result<()> {
        let original = fs::read(path)?;
        if original != expected {
            return Err(std::io::Error::other(format!(
                "{} changed while init was running",
                path.display()
            )));
        }
        self.modified_files.push((path.to_path_buf(), original));
        let mut file = fs::OpenOptions::new().write(true).truncate(true).open(path)?;
        file.write_all(contents)
    }

    pub(super) fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for ProjectTransaction {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for path in self.created_files.iter().rev() {
            let _ = fs::remove_file(path);
        }
        for (path, contents) in self.modified_files.iter().rev() {
            let _ = fs::write(path, contents);
        }
        for path in self.created_dirs.iter().rev() {
            let _ = fs::remove_dir(path);
        }
    }
}
