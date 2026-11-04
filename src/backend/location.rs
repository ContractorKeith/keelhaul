//! Repository selection is independent of credentials and opening storage.
use super::s3::S3Location;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum RepoLocation {
    Local(PathBuf),
    S3(S3Location),
    Sftp(String),
}

impl RepoLocation {
    pub fn parse(repo: &str) -> Result<Self, &'static str> {
        if repo.starts_with("s3:") {
            S3Location::parse(repo)
                .map(Self::S3)
                .map_err(|_| "invalid S3 repository URL")
        } else if repo.starts_with("sftp:") {
            Ok(Self::Sftp(repo.to_owned()))
        } else if repo.contains(':') {
            Err("unknown repository scheme")
        } else if repo.is_empty() {
            Err("repository path must not be empty")
        } else {
            Ok(Self::Local(repo.into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn s3_repo_parse_table() {
        for (repo, valid) in [
            ("/scratch/repo", true),
            ("./repo", true),
            ("sftp:host:/repo", true),
            ("s3:https://example.com/bucket/prefix", true),
            ("s3:http://127.0.0.1:7070/bucket", true),
            ("s3:https://example.com", false),
            ("s3:ftp://host/bucket", false),
            ("s3:https://user:secret@host/bucket", false),
            ("s3:https://host/bucket?secret=x", false),
            ("s3:https://host/bucket/../outside", false),
            ("https://host/bucket", false),
            ("unknown:repo", false),
            ("", false),
        ] {
            assert_eq!(RepoLocation::parse(repo).is_ok(), valid, "{repo}");
        }
    }
}
