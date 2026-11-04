//! Stable damage documents shared by audits and restore.
use crate::repo::Error;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Damage {
    pub object: String,
    pub kind: String,
    pub problem: String,
    pub detail: String,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Note {
    pub object: String,
    pub kind: String,
    pub problem: String,
}
#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub damage: Vec<Damage>,
    pub notes: Vec<Note>,
}
pub fn kind(object: &str) -> &'static str {
    match object.split('/').next() {
        Some("config") => "config",
        Some("keys") => "key",
        Some("data") => "pack",
        Some("index") => "index",
        Some("snapshots") => "snapshot",
        Some("locks") => "lock",
        _ => "blob",
    }
}
impl Damage {
    pub fn new(object: &str, kind: &str, problem: &str, detail: &str) -> Self {
        Self {
            object: object.into(),
            kind: kind.into(),
            problem: problem.into(),
            detail: detail.into(),
        }
    }
    /// Preserve raw POSIX paths in machine-readable file identifiers.
    pub fn file(snapshot: &str, path: &std::path::Path, problem: &str) -> Self {
        use base64::Engine;
        use std::os::unix::ffi::OsStrExt;
        let path = match path.to_str() {
            Some(path) => path.to_owned(),
            None => format!(
                "b64:{}",
                base64::prelude::BASE64_STANDARD.encode(path.as_os_str().as_bytes())
            ),
        };
        Self::new(
            &format!("{snapshot}:{path}"),
            "file",
            problem,
            &format!("snapshot {snapshot}, path {path}"),
        )
    }
    pub fn from_error(error: Error) -> Result<Self, Error> {
        match error {
            Error::Damage {
                object,
                problem,
                detail,
            } => Ok(Self::new(&object, kind(&object), problem, &detail)),
            other => Err(other),
        }
    }
}
impl std::fmt::Display for Damage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}: {}",
            self.object.escape_debug(),
            self.problem,
            self.detail.escape_debug()
        )
    }
}
impl Report {
    /// Collect repository damage, but propagate operational failures (exit 1).
    pub fn collect<T>(&mut self, result: Result<T, Error>) -> Result<Option<T>, Error> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) => {
                self.damage.push(Damage::from_error(error)?);
                Ok(None)
            }
        }
    }
    pub fn add(&mut self, object: &str, kind: &str, problem: &str, detail: &str) {
        self.damage.push(Damage::new(object, kind, problem, detail));
    }
    pub fn note(&mut self, object: &str, kind: &str, problem: &str) {
        self.notes.push(Note {
            object: object.into(),
            kind: kind.into(),
            problem: problem.into(),
        });
    }
    pub fn sort(&mut self) {
        self.damage.sort_by(|a, b| {
            (&a.object, &a.problem, &a.kind, &a.detail)
                .cmp(&(&b.object, &b.problem, &b.kind, &b.detail))
        });
        self.damage
            .dedup_by(|a, b| a.object == b.object && a.kind == b.kind && a.problem == b.problem);
        self.notes
            .sort_by(|a, b| (&a.object, &a.problem).cmp(&(&b.object, &b.problem)));
        self.notes.dedup();
    }
    pub fn text(&self) {
        for d in &self.damage {
            println!("{d}");
        }
        for n in &self.notes {
            println!("{}: {}", n.object.escape_debug(), n.problem);
        }
    }
}
