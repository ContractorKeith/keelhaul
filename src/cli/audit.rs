//! Audit rendering, including restore failures before a report can be assembled.
use crate::{damage::Damage, repo::Error};
pub(super) fn restore_failure(error: Error, json: bool) -> anyhow::Result<()> {
    let damage = Damage::from_error(error)?;
    if json {
        #[derive(serde::Serialize)]
        struct Report {
            restored: u64,
            damage: Vec<Damage>,
        }
        println!(
            "{}",
            serde_json::to_string(&Report {
                restored: 0,
                damage: vec![damage]
            })?
        );
    } else {
        println!("{damage}");
    }
    super::backup::completed_damage()?;
    Ok(())
}
