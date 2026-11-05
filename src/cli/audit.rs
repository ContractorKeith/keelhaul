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
        crate::output::damage(&damage);
    }
    super::backup::completed_damage()?;
    Ok(())
}

pub(super) fn restore<B: crate::backend::Backend>(
    repo: &crate::repo::Repository<B>,
    snap: &str,
    target: &std::path::Path,
    includes: &[std::path::PathBuf],
    options: crate::ops::restore::Options<'_>,
    json: bool,
) -> anyhow::Result<()> {
    let result = crate::ops::restore::restore_with_options(repo, snap, target, includes, &options);
    let report = match result {
        Ok(report) => report,
        Err(error) => return restore_failure(error, json),
    };
    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        println!(
            "restored {} entries from snapshot {}",
            report.restored, report.snapshot
        );
        for damage in &report.damaged {
            crate::output::damage(&damage);
        }
        for failure in &report.failed {
            crate::warning!("{}: {}", failure.path, failure.error);
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

pub(super) fn listing<B: crate::backend::Backend>(
    repo: &crate::repo::Repository<B>,
    selector: &str,
    filter: Option<&std::path::Path>,
    host: Option<&str>,
    json: bool,
    no_lock: bool,
) -> anyhow::Result<()> {
    let report = if no_lock {
        crate::ops::listing::ls_report_for_host_without_lock(repo, selector, filter, host)?
    } else {
        crate::ops::listing::ls_report_for_host(repo, selector, filter, host)?
    };
    if json {
        if report.damage.is_empty() {
            println!("{}", serde_json::to_string(&report.entries)?);
        } else {
            println!("{}", serde_json::to_string(&report)?);
        }
    } else {
        if selector == "latest" && !report.damage.is_empty() {
            crate::warning!("using snapshot {}", report.snapshot);
        }
        for entry in &report.entries {
            println!(
                "{}{}",
                entry.path,
                if entry.shared {
                    " [shared subtree]"
                } else {
                    ""
                }
            );
        }
    }
    for damage in &report.damage {
        crate::output::damage(&damage);
    }
    if !report.damage.is_empty() {
        super::backup::completed_damage()?;
    }
    Ok(())
}
