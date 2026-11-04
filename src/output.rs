use serde::Serialize;
use std::io::{self, Write};

#[derive(Serialize)]
struct Version<'a> {
    version: &'a str,
}

pub fn version(mut out: impl Write, json: bool) -> io::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    if json {
        serde_json::to_writer(&mut out, &Version { version })?;
        writeln!(out)
    } else {
        writeln!(out, "keelhaul {version}")
    }
}
