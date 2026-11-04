use keelhaul::{
    backend::local::LocalBackend,
    crypto::envelope::Label,
    format::snapshot::{Snapshot, Summary},
    ops::forget::{self, Options},
    repo::Repository,
    secret::Secret,
};
use std::{collections::HashSet, process::Command};
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};
fn snapshot(time: &str) -> Snapshot {
    Snapshot {
        version: 1,
        time: time.into(),
        host: "fixture".into(),
        username: "test".into(),
        paths: vec!["/source".into()],
        tags: vec![],
        excludes: vec![],
        tree: "ab".repeat(32),
        parent: None,
        program: "test".into(),
        summary: Summary::default(),
    }
}
fn timestamp(t: OffsetDateTime) -> String {
    t.format(
        &time::format_description::parse_borrowed::<2>(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:9]Z",
        )
        .unwrap(),
    )
    .unwrap()
}
#[test]
fn synthetic_hourly_leap_year_retention_golden_in_new_york() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let repo = Repository::init(
        LocalBackend::new(root.join("repo")).unwrap(),
        &Secret::KeyFile([12; 32]),
    )
    .unwrap();
    std::fs::write(root.join("key"), [12; 32]).unwrap();
    let start = OffsetDateTime::parse("2024-01-01T05:00:00Z", &Rfc3339).unwrap();
    let mut by_id = std::collections::HashMap::new();
    // 366 local days; the spring missing hour and fall repeated hour cancel.
    for hour in 0..8784 {
        let time = timestamp(start + time::Duration::hours(hour));
        let id = repo
            .write_object(
                "snapshots",
                Label::Snapshot,
                &serde_json::to_vec(&snapshot(&time)).unwrap(),
            )
            .unwrap();
        by_id.insert(id, time);
    }
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_keelhaul"))
            .env("HOME", root)
            .env("TZ", "America/New_York")
            .arg("-r")
            .arg(root.join("repo"))
            .arg("--key-file")
            .arg(root.join("key"))
            .args(["--json", "forget", "--dry-run"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !String::from_utf8_lossy(&out.stderr).contains("uses UTC"),
            "must exercise local offsets"
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let report = run(&[
        "--keep-daily",
        "7",
        "--keep-weekly",
        "4",
        "--keep-monthly",
        "12",
    ]);
    let actual = report["groups"][0]["keep"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| by_id[s["id"].as_str().unwrap()].clone())
        .collect::<HashSet<_>>();
    // Monthly: last day of Jan–Dec at 23:00 NY (04:00 UTC before DST,
    // 03:00 UTC during DST, then 04:00 UTC). Daily: Dec 25–31 at 23:00.
    // ISO weeks: Dec 31 (2025-W01), Dec 29 (W52), Dec 22 (W51), Dec 15
    // (W50). Dec 31 is monthly+daily; Dec 29 is daily. Union: 20 snapshots.
    let expected = [
        "2024-02-01T04",
        "2024-03-01T04",
        "2024-04-01T03",
        "2024-05-01T03",
        "2024-06-01T03",
        "2024-07-01T03",
        "2024-08-01T03",
        "2024-09-01T03",
        "2024-10-01T03",
        "2024-11-01T03",
        "2024-12-01T04",
        "2025-01-01T04",
        "2024-12-26T04",
        "2024-12-27T04",
        "2024-12-28T04",
        "2024-12-29T04",
        "2024-12-30T04",
        "2024-12-31T04",
        "2024-12-23T04",
        "2024-12-16T04",
    ]
    .into_iter()
    .map(|s| format!("{s}:00:00.000000000Z"))
    .collect::<HashSet<_>>();
    assert_eq!(actual, expected);
    use keelhaul::backend::{Backend, Kind};
    assert_eq!(repo.backend.list(Kind::Snapshots).unwrap().len(), 8784);
}
#[test]
fn grouping_tags_union_and_usage_safety() {
    let mut a = snapshot("2024-01-01T00:00:00.000000000Z");
    a.tags = vec!["archive".into()];
    let mut b = snapshot("2024-01-02T00:00:00.000000000Z");
    b.host = "other".into();
    let mut c = snapshot("2024-01-03T00:00:00.000000000Z");
    c.paths = vec!["/different".into()];
    let options = Options {
        keep_last: Some(1),
        keep_tag: vec!["archive".into()],
        ..Options::default()
    };
    let report = forget::plan(
        vec![
            ("a".into(), a.clone()),
            ("b".into(), b.clone()),
            ("c".into(), c.clone()),
        ],
        &options,
        &HashSet::new(),
        |_| Some(UtcOffset::UTC),
    )
    .unwrap();
    assert_eq!(report.groups.len(), 3);
    let options = Options {
        group_by: Some(String::new()),
        ..options
    };
    let report = forget::plan(
        vec![("a".into(), a), ("b".into(), b), ("c".into(), c)],
        &options,
        &HashSet::new(),
        |_| Some(UtcOffset::UTC),
    )
    .unwrap();
    assert_eq!(report.groups.len(), 1);
    assert_eq!(report.groups[0].keep.len(), 2);
    assert_eq!(report.groups[0].remove, vec!["b"]);
    assert!(Options::default().validate().is_err());
    assert!(
        Options {
            snapshots: vec!["id".into()],
            keep_last: Some(1),
            ..Options::default()
        }
        .validate()
        .is_err()
    );
}
