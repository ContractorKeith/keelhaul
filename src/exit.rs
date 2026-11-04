//! Central command outcome to process exit-code mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Error,
    Usage,
    Damage,
    Locked,
}

pub const fn code(result: Result<(), Failure>) -> u8 {
    match result {
        Ok(()) => 0,
        Err(Failure::Error) => 1,
        Err(Failure::Usage) => 2,
        Err(Failure::Damage) => 3,
        Err(Failure::Locked) => 4,
    }
}

/// Classify typed causes even when callers add context or wrap an error.
pub fn failure(error: &(dyn std::error::Error + 'static)) -> Failure {
    let mut current = Some(error);
    while let Some(cause) = current {
        if matches!(
            cause.downcast_ref::<crate::secret::Error>(),
            Some(crate::secret::Error::ConflictingSources)
        ) {
            return Failure::Usage;
        }
        if matches!(
            cause.downcast_ref::<crate::repo::Error>(),
            Some(crate::repo::Error::Damage { .. })
        ) {
            return Failure::Damage;
        }
        current = cause.source();
    }
    Failure::Error
}

/// Keep stderr diagnostics and JSON failures consistent across command paths.
pub fn report(message: &str, failure: Failure, json: bool) {
    use std::io::Write;
    eprintln!("keelhaul: {message}");
    if json {
        #[derive(serde::Serialize)]
        struct Detail<'a> {
            message: &'a str,
            exit: u8,
        }
        #[derive(serde::Serialize)]
        struct Report<'a> {
            error: Detail<'a>,
        }
        let mut out = std::io::stdout().lock();
        let report = Report {
            error: Detail {
                message,
                exit: code(Err(failure)),
            },
        };
        if let Err(error) = serde_json::to_writer(&mut out, &report)
            .map_err(std::io::Error::other)
            .and_then(|()| writeln!(out))
        {
            eprintln!("keelhaul: cannot write JSON error: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_contextual_error_chains() {
        let usage =
            anyhow::Error::new(crate::secret::Error::ConflictingSources).context("loading secret");
        assert_eq!(failure(usage.as_ref()), Failure::Usage);
        let damaged = anyhow::Error::new(crate::repo::Error::Damage {
            object: "config".into(),
            problem: "bad-format",
            detail: "test".into(),
        })
        .context("opening repository");
        assert_eq!(failure(damaged.as_ref()), Failure::Damage);
        let wrong =
            anyhow::Error::new(crate::repo::Error::WrongSecret).context("opening repository");
        assert_eq!(failure(wrong.as_ref()), Failure::Error);
    }

    #[test]
    fn maps_all_outcomes() {
        assert_eq!(code(Ok(())), 0);
        for (failure, expected) in [
            (Failure::Error, 1),
            (Failure::Usage, 2),
            (Failure::Damage, 3),
            (Failure::Locked, 4),
        ] {
            assert_eq!(code(Err(failure)), expected);
        }
    }
}
