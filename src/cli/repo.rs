use crate::backend::{BackendError, local::LocalBackend};

pub(super) fn select(path: &str, initializing: bool) -> anyhow::Result<LocalBackend> {
    if path.starts_with("s3:") || path.starts_with("sftp:") {
        anyhow::bail!("this backend is not implemented yet");
    }
    if initializing {
        return Ok(LocalBackend::new(path)?);
    }
    LocalBackend::open(path).map_err(|error| match error {
        BackendError::NotFound(_) => anyhow::anyhow!("repository not found: {path}"),
        other => other.into(),
    })
}
