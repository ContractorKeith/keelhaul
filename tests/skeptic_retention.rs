use keelhaul::ops::forget::Options;
#[test]
fn zero_only_policies_are_refused_but_tags_and_explicit_ids_work() {
    for options in [
        Options {
            keep_last: Some(0),
            ..Default::default()
        },
        Options {
            keep_hourly: Some(0),
            keep_daily: Some(0),
            keep_yearly: Some(0),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            options.validate(),
            Err(keelhaul::repo::Error::Usage(_))
        ));
    }
    assert!(
        Options {
            keep_last: Some(0),
            keep_tag: vec!["keep".into()],
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
    assert!(
        Options {
            snapshots: vec!["12345678".into()],
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
}
