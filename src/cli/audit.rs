//! Audit rendering, including restore failures before a report can be assembled.
use crate::{damage::Damage, repo::Error};
pub(super) fn restore_failure(error: Error, json: bool) -> anyhow::Result<()> {
    let damage = Damage::from_error(error)?;
    if json {
        #[derive(serde::Serialize)]
        struct Report {
            restored: u64,
            damage: Vec<Damage>,
            failed: Vec<crate::ops::restore::Failure>,
        }
        println!(
            "{}",
            serde_json::to_string(&Report {
                restored: 0,
                damage: vec![damage],
                failed: Vec::new(),
            })?
        );
    } else {
        eprintln!("{damage}");
    }
    super::backup::completed_damage()?;
    Ok(())
}

pub(super) fn restore<B: crate::backend::Backend>(
    repo: &crate::repo::Repository<B>,
    snap: &str,
    target: &std::path::Path,
    includes: &[std::path::PathBuf],
    host: Option<&str>,
    json: bool,
) -> anyhow::Result<()> {
    let report = match crate::ops::restore::restore_for_host(repo, snap, target, includes, host) {
        Ok(report) => report,
        Err(error) => return restore_failure(error, json),
    };
    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        println!("restored {} entries", report.restored);
        for damage in &report.damaged {
            eprintln!("{damage}");
        }
        for failure in &report.failed {
            eprintln!("{}: {}", failure.path, failure.error);
        }
    }
    if !report.damaged.is_empty() {
        super::backup::completed_damage()?;
    }
    if !report.failed.is_empty() {
        // Report already rendered; avoid emitting a second JSON document.
        std::process::exit(1);
    }
    Ok(())
}
