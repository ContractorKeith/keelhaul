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
    if path.starts_with("s3:") {
        let location = S3Location::parse(path)
            .map_err(|_| BackendError::InvalidConfig("invalid S3 repository URL"))?;
        return Ok(Box::new(S3Backend::new(S3Config::from_env(
            location, verbose,
        )?)?));
    }
    if path.starts_with("sftp:") {
        anyhow::bail!("this backend is not implemented yet");
    }
    if path.is_empty() || path.contains(':') {
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
}
