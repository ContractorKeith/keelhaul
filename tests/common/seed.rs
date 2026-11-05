//! A fresh run explores new cases; KEELHAUL_SEED replays a reported failure.
pub fn seed() -> u64 {
    use std::io::Write;
    let seed = std::env::var("KEELHAUL_SEED")
        .map(|s| s.parse().expect("numeric KEELHAUL_SEED"))
        .unwrap_or_else(|_| random_seed());
    writeln!(std::io::stderr(), "KEELHAUL_SEED={seed}").unwrap();
    seed
}
fn random_seed() -> u64 {
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}
#[test]
fn fresh_seed_is_not_the_previous_run() {
    assert_ne!(random_seed(), random_seed());
}
