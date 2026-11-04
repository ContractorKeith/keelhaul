#[test]
fn s3_backup_restore_roundtrip_metadata_dedup_and_include() {
    if !super::docker::enabled() {
        return;
    }
    let fixture = super::Fixture::new();
    let backend = fixture.backend("http://127.0.0.1:7070", "roundtrip");
    super::backup_fixture::run(backend);
}
