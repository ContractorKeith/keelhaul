//! Root identity shared by backup, snapshot filters and future parent lookup.
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

/// Resolve the parent through the filesystem, preserving a final symlink.
/// Historical paths whose parents disappeared use a cleaned absolute spelling.
pub fn normalize_root(path: &Path) -> io::Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let resolved = match (joined.parent(), joined.file_name()) {
        (Some(parent), Some(name)) => fs::canonicalize(parent).map(|p| p.join(name)),
        _ => fs::canonicalize(&joined),
    };
    match resolved {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut clean = PathBuf::new();
            for part in joined.components() {
                match part {
                    Component::ParentDir => {
                        clean.pop();
                    }
                    Component::CurDir => {}
                    other => clean.push(other.as_os_str()),
                }
            }
            Ok(clean)
        }
        Err(error) => Err(error),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_parent_falls_back_to_cleaned_absolute_spelling() {
        let scratch = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(scratch.path()).unwrap();
        assert_eq!(
            normalize_root(&root.join("gone/../missing/file")).unwrap(),
            root.join("missing/file")
        );
        let relative = Path::new("missing-root/../missing-file");
        assert_eq!(
            normalize_root(relative).unwrap(),
            std::env::current_dir().unwrap().join("missing-file")
        );
    }
    #[test]
    fn parent_alias_resolves_but_final_alias_is_preserved() {
        let scratch = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(scratch.path()).unwrap();
        fs::create_dir(root.join("actual")).unwrap();
        std::os::unix::fs::symlink("actual", root.join("alias")).unwrap();
        assert_eq!(
            normalize_root(&root.join("alias/file")).unwrap(),
            root.join("actual/file")
        );
        assert_eq!(
            normalize_root(&root.join("alias")).unwrap(),
            root.join("alias")
        );
    }
}
