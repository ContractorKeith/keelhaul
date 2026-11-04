use keelhaul::{
    format::snapshot::{Snapshot, Summary},
    ops::forget::{Options, plan},
};
use std::collections::HashSet;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};
fn snapshot(hour: &str, paths: Vec<String>) -> Snapshot {
    Snapshot {
        version: 1,
        time: format!("2024-11-03T{hour}:30:00.000000000Z"),
        host: "fixture".into(),
        username: "test".into(),
        paths,
        tags: vec![],
        excludes: vec![],
        tree: "ab".repeat(32),
        parent: None,
        program: "test".into(),
        summary: Summary::default(),
    }
}
#[test]
fn repeated_dst_hour_is_one_bucket_and_keeps_the_newest_instance() {
    let options = Options {
        keep_hourly: Some(3),
        ..Options::default()
    };
    let entries = ["04", "05", "06", "07"]
        .into_iter()
        .enumerate()
        .map(|(i, h)| (i.to_string(), snapshot(h, vec!["/source".into()])))
        .collect();
    let change = OffsetDateTime::parse("2024-11-03T06:00:00Z", &Rfc3339).unwrap();
    let report = plan(entries, &options, &HashSet::new(), |t| {
        Some(UtcOffset::from_hms(if t < change { -4 } else { -5 }, 0, 0).unwrap())
    })
    .unwrap();
    assert_eq!(
        report.groups[0]
            .keep
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        vec!["3", "2", "0"]
    );
    assert_eq!(report.groups[0].remove, vec!["1"]);
}
#[test]
fn grouping_normalizes_path_order_and_duplicates() {
    let entries = vec![
        (
            "old".into(),
            snapshot("04", vec!["/b".into(), "/a".into(), "/a".into()]),
        ),
        ("new".into(), snapshot("05", vec!["/a".into(), "/b".into()])),
    ];
    let report = plan(
        entries,
        &Options {
            keep_last: Some(1),
            ..Options::default()
        },
        &HashSet::new(),
        |_| Some(UtcOffset::UTC),
    )
    .unwrap();
    assert_eq!(report.groups.len(), 1);
    assert_eq!(
        report.groups[0].key.paths,
        Some(vec!["/a".into(), "/b".into()])
    );
    assert_eq!(report.groups[0].remove, vec!["old"]);
}
