use crate::backend::{
    Backend, BackendError,
    local::LocalBackend,
    s3::{S3Backend, S3Config, S3Location},
};

// All commands resolve --repo here; version does not open storage.
pub(super) fn select(
    path: &str,
    initializing: bool,
    verbose: bool,
) -> anyhow::Result<Box<dyn Backend>> {
    if scheme(path) == Some("s3") {
        let location = S3Location::parse(path)
            .map_err(|_| BackendError::InvalidConfig("invalid S3 repository URL"))?;
        return Ok(Box::new(S3Backend::new(S3Config::from_env(
            location, verbose,
        )?)?));
    }
    if scheme(path) == Some("sftp") {
        anyhow::bail!("this backend is not implemented yet");
    }
    if path.is_empty() || scheme(path).is_some() {
        return Err(BackendError::InvalidConfig(
            "unknown repository scheme or empty repository path",
        )
        .into());
    }
    let backend = if initializing {
        LocalBackend::new(path)
    } else {
        LocalBackend::open(path)
    };
    backend
        .map(|b| Box::new(b) as Box<dyn Backend>)
        .map_err(|error| match error {
            BackendError::NotFound(_) => anyhow::anyhow!("repository not found: {path}"),
            other => other.into(),
        })
}

// Only a lowercase URI scheme before the first slash selects remote storage.
// Explicit filesystem paths keep colons as ordinary filename characters.
fn scheme(path: &str) -> Option<&str> {
    if path.starts_with(['/', '~']) || path.starts_with("./") || path.starts_with("../") {
        return None;
    }
    let (scheme, _) = path.split('/').next()?.split_once(':')?;
    let mut bytes = scheme.bytes();
    if !bytes.next()?.is_ascii_lowercase()
        || !bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-".contains(&b))
    {
        return None;
    }
    Some(scheme)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn s3_selector_rejects_invalid_urls_and_unknown_schemes_as_usage() {
        for repo in [
            "s3:https://host",
            "s3:ftp://host/bucket",
            "s3:https://host/bucket/a//b",
            "https://host/bucket",
            "unknown:repo",
            "",
        ] {
            let error = select(repo, false, false).err().unwrap();
            assert_eq!(
                crate::exit::failure(error.as_ref()),
                crate::exit::Failure::Usage,
                "{repo}"
            );
        }
    }
    #[test]
    fn s3_scheme_table_preserves_local_colons() {
        for (path, expected) in [
            ("/tmp/x:y", None),
            ("./a:b", None),
            ("../a:b", None),
            ("~/a:b", None),
            ("~user:a", None),
            ("a/b:c", None),
            ("1a:repo", None),
            ("Abc:repo", None),
            ("s3:https://host/bucket", Some("s3")),
            ("sftp:host:/repo", Some("sftp")),
            ("unknown:repo", Some("unknown")),
            ("foo+bar.baz-qux:repo", Some("foo+bar.baz-qux")),
        ] {
            assert_eq!(scheme(path), expected, "{path}");
        }
    }

    #[test]
    fn s3_selector_initializes_local_paths_with_colons() {
        let scratch = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let absolute = scratch.path().join("backup:2026");
        let relative = format!(
            "./{}/a:b",
            scratch.path().file_name().unwrap().to_str().unwrap()
        );
        for path in [absolute.to_str().unwrap(), &relative] {
            let backend = select(path, true, false).unwrap();
            let name = crate::backend::ObjectName::new("config").unwrap();
            backend.write(&name, b"local").unwrap();
            assert_eq!(
                select(path, false, false).unwrap().read(&name).unwrap(),
                b"local"
            );
        }
    }
}
