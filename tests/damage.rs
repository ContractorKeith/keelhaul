//! Independent expected sets compare (object, kind, problem), not just object IDs.
//! See damage_support/byte_cases.rs and authenticated.rs for each injection's oracle.
#[path = "common/backup_fixture.rs"]
#[allow(dead_code, reason = "shared round-trip helpers")]
mod backup_fixture;
#[path = "damage_support/mod.rs"]
mod support;
#[test]
fn seeded_damage_exact_sets() {
    let scratch = tempfile::tempdir().unwrap();
    let source = std::fs::canonicalize(scratch.path())
        .unwrap()
        .join("source");
    backup_fixture::damage_fixture(&source);
    let f = support::Fixture::new(&source);
    support::run(&f, 0, "clean", Default::default(), Default::default());
    let count = if std::env::var_os("KEELHAUL_SOAK").is_some() {
        200
    } else {
        20
    };
    let seeds = match std::env::var("KEELHAUL_SEED") {
        Ok(s) => vec![s.parse::<u64>().unwrap()],
        Err(_) => (0..count).collect(),
    };
    let n = seeds.len();
    for seed in seeds {
        let mut h = blake3::Hasher::new();
        h.update(&seed.to_le_bytes());
        let mut random = [0; 32];
        h.finalize_xof().fill(&mut random);
        support::byte_cases::run_cases(&f, seed, random);
        support::authenticated::run_cases(&f, seed);
    }
    f.reset();
    support::run(
        &f,
        0,
        "clean after reset",
        Default::default(),
        Default::default(),
    );
    println!("damage: {n} seeds green");
}
