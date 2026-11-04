use keelhaul::chunker::{Chunker, MAX, MIN};
use proptest::prelude::*;
fn config() -> ProptestConfig {
    let seed = std::env::var("KEELHAUL_SEED")
        .map(|s| s.parse::<u64>().expect("KEELHAUL_SEED is a u64"))
        .unwrap_or(1801807212);
    eprintln!("KEELHAUL_SEED={seed}");
    ProptestConfig {
        cases: 12,
        rng_seed: proptest::test_runner::RngSeed::Fixed(seed),
        ..ProptestConfig::default()
    }
}
proptest! {
    #![proptest_config(config())]
    #[test]
    fn streaming_concatenates_bounded_deterministic(seed in any::<[u8;32]>(), key in any::<[u8;32]>(), len in 0usize..20*1024*1024) {
        let mut bytes=vec![0;len]; let mut h=blake3::Hasher::new(); h.update(&seed); h.finalize_xof().fill(&mut bytes);
        let mut reader=Chunker::new(bytes.as_slice(),&key); let mut output=Vec::new(); let mut lengths=Vec::new();
        while let Some(chunk)=reader.next_chunk().unwrap() { prop_assert!(chunk.len()<=MAX); lengths.push(chunk.len()); output.extend(chunk); }
        prop_assert_eq!(&output,&bytes);
        for length in lengths.iter().take(lengths.len().saturating_sub(1)) { prop_assert!(*length>=MIN); }
        let mut again=Chunker::new(ShortReads(bytes.as_slice()),&key); let mut other=Vec::new();
        while let Some(chunk)=again.next_chunk().unwrap() { other.push(chunk.len()); }
        prop_assert_eq!(lengths,other);
    }
}
struct ShortReads<'a>(&'a [u8]);
impl std::io::Read for ShortReads<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let len = buf.len().min(32749);
        std::io::Read::read(&mut self.0, &mut buf[..len])
    }
}
#[test]
fn empty_and_limits() {
    for len in [0, MIN - 1, MIN, MAX, MAX + 1] {
        let bytes = vec![0; len];
        let mut c = Chunker::new(bytes.as_slice(), &[1; 32]);
        let mut n = 0;
        while let Some(bytes) = c.next_chunk().unwrap() {
            assert!(bytes.len() <= MAX);
            n += bytes.len();
        }
        assert_eq!(n, len);
    }
}
