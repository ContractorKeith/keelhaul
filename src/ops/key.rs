use crate::{
    backend::Backend,
    repo::{Error, Repository},
};
use serde::Serialize;
#[derive(Serialize)]
pub struct SlotSummary {
    pub id: String,
    pub status: &'static str,
    pub kind: Option<crate::format::key_slot::SlotKind>,
    pub created: Option<String>,
    pub host: Option<String>,
    pub username: Option<String>,
}
pub fn list<B: Backend>(repo: &Repository<B>) -> Result<Vec<SlotSummary>, Error> {
    Ok(repo
        .scan_keys()?
        .into_iter()
        .map(|(id, slot)| {
            let status = match &slot {
                None => "unreadable",
                Some(slot)
                    if crate::crypto::slot::verify_check(slot, &repo.subkeys().key_slot_check)
                        .is_ok() =>
                {
                    "ok"
                }
                Some(_) => "damaged",
            };
            match slot {
                Some(slot) => SlotSummary {
                    id,
                    status,
                    kind: Some(slot.kind),
                    created: Some(slot.created),
                    host: Some(slot.host),
                    username: Some(slot.username),
                },
                None => SlotSummary {
                    id,
                    status,
                    kind: None,
                    created: None,
                    host: None,
                    username: None,
                },
            }
        })
        .collect())
}
pub fn render_list<B: Backend>(
    mut out: impl std::io::Write,
    repo: &Repository<B>,
    json: bool,
) -> Result<(), Error> {
    let slots = list(repo)?;
    if json {
        #[derive(Serialize)]
        struct Listing {
            slots: Vec<SlotSummary>,
        }
        serde_json::to_writer(&mut out, &Listing { slots })?;
        writeln!(out)?;
    } else {
        writeln!(out, "ID STATUS KIND CREATED HOST USERNAME")?;
        for slot in slots {
            let kind = match slot.kind {
                Some(crate::format::key_slot::SlotKind::Password) => "password",
                Some(crate::format::key_slot::SlotKind::Keyfile) => "keyfile",
                None => "-",
            };
            writeln!(
                out,
                "{} {} {} {} {} {}",
                slot.id,
                slot.status,
                kind,
                slot.created.as_deref().unwrap_or("-"),
                slot.host.as_deref().unwrap_or("-"),
                slot.username.as_deref().unwrap_or("-")
            )?;
        }
    }
    Ok(())
}
