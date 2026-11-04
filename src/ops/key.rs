use crate::{
    backend::Backend,
    repo::{Error, Repository},
};
use serde::Serialize;

#[derive(Serialize)]
pub struct SlotSummary {
    pub id: String,
    pub kind: crate::format::key_slot::SlotKind,
    pub created: String,
    pub host: String,
    pub username: String,
}

pub fn list<B: Backend>(repo: &Repository<B>) -> Result<Vec<SlotSummary>, Error> {
    Ok(repo
        .list_keys()?
        .into_iter()
        .map(|(id, slot)| SlotSummary {
            id,
            kind: slot.kind,
            created: slot.created,
            host: slot.host,
            username: slot.username,
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
        for slot in slots {
            let kind = match slot.kind {
                crate::format::key_slot::SlotKind::Password => "password",
                crate::format::key_slot::SlotKind::Keyfile => "keyfile",
            };
            writeln!(
                out,
                "{} {} {} {} {}",
                slot.id, kind, slot.created, slot.host, slot.username
            )?;
        }
    }
    Ok(())
}
