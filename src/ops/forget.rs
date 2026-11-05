//! Retention selects the newest member of occupied calendar buckets in each group.
use crate::{
    backend::{Backend, ObjectName},
    format::snapshot::Snapshot,
    repo::{
        Error, Repository,
        lock::{self, SystemClock},
    },
};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use time::{OffsetDateTime, UtcOffset};
#[derive(Debug, Default, clap::Args)]
pub struct Options {
    pub snapshots: Vec<String>,
    #[arg(long)]
    pub keep_last: Option<usize>,
    #[arg(
        long,
        help = "Keep the newest snapshot per local hour; the repeated DST hour is one bucket"
    )]
    pub keep_hourly: Option<usize>,
    #[arg(long)]
    pub keep_daily: Option<usize>,
    #[arg(long)]
    pub keep_weekly: Option<usize>,
    #[arg(long)]
    pub keep_monthly: Option<usize>,
    #[arg(long)]
    pub keep_yearly: Option<usize>,
    #[arg(long)]
    pub keep_tag: Vec<String>,
    #[arg(long, default_value = "host,paths")]
    pub group_by: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub prune: bool,
}
#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct GroupKey {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<String>>,
}
#[derive(Debug, Serialize)]
pub struct Kept {
    pub id: String,
    pub reasons: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Group {
    pub key: GroupKey,
    pub keep: Vec<Kept>,
    pub remove: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub groups: Vec<Group>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prune: Option<super::prune::Summary>,
}
impl Options {
    fn counts(&self) -> [Option<usize>; 6] {
        [
            self.keep_last,
            self.keep_hourly,
            self.keep_daily,
            self.keep_weekly,
            self.keep_monthly,
            self.keep_yearly,
        ]
    }
    pub fn validate(&self) -> Result<(), Error> {
        let policy = self.counts().iter().any(Option::is_some) || !self.keep_tag.is_empty();
        if self.snapshots.is_empty() && !policy {
            return Err(Error::Usage(
                "forget requires snapshot IDs or a retention policy".into(),
            ));
        }
        if !self.snapshots.is_empty() && policy {
            return Err(Error::Usage(
                "snapshot IDs cannot be combined with a retention policy".into(),
            ));
        }
        if policy && self.keep_tag.is_empty() && !self.counts().iter().flatten().any(|n| *n > 0) {
            return Err(Error::Usage("retention policy would keep nothing".into()));
        }
        let mut seen = HashSet::new();
        for part in self
            .group_by
            .as_deref()
            .unwrap_or("host,paths")
            .split(',')
            .filter(|s| !s.is_empty())
        {
            if !["host", "paths"].contains(&part) || !seen.insert(part) {
                return Err(Error::Usage(
                    "group-by must contain host, paths, both, or be empty".into(),
                ));
            }
        }
        Ok(())
    }
}
pub fn forget<B: Backend>(repo: &Repository<B>, options: &Options) -> Result<Report, Error> {
    options.validate()?;
    let fallback = local_offset();
    forget_with_offsets(repo, options, |time| {
        // time 0.3.55 separates the guarded TZ refresh from thread-safe
        // localtime lookups. Preserve historical DST after startup refresh.
        fallback.and_then(|offset| UtcOffset::local_offset_at(time).ok().or(Some(offset)))
    })
}
static LOCAL_OFFSET: std::sync::OnceLock<Option<UtcOffset>> = std::sync::OnceLock::new();
/// Call at process startup, before backends or workers spawn any threads.
pub fn local_offset() -> Option<UtcOffset> {
    *LOCAL_OFFSET.get_or_init(|| {
        time::util::refresh_tz()?;
        UtcOffset::current_local_offset().ok()
    })
}
pub fn forget_with_offset<B: Backend>(
    repo: &Repository<B>,
    options: &Options,
    offset: Option<UtcOffset>,
) -> Result<Report, Error> {
    forget_with_offsets(repo, options, |_| offset)
}
fn forget_with_offsets<B: Backend>(
    repo: &Repository<B>,
    options: &Options,
    mut offset: impl FnMut(OffsetDateTime) -> Option<UtcOffset>,
) -> Result<Report, Error> {
    options.validate()?;
    lock::with(&repo.backend, repo.subkeys(), true, |guard| {
        let scanned = repo.scan_snapshots()?;
        if scanned.damaged {
            return Err(Error::Damage {
                object: "snapshots".into(),
                problem: "bad-format",
                detail: "unreadable snapshot; retention aborted".into(),
            });
        }
        let mut explicit = HashSet::new();
        for selector in &options.snapshots {
            explicit.insert(repo.select_snapshot(selector)?.0);
        }
        let mut report = plan(scanned.entries, options, &explicit, &mut offset)?;
        if !options.dry_run {
            guard.self_check()?;
            for group in &report.groups {
                for id in &group.remove {
                    repo.backend
                        .delete(&ObjectName::new(&format!("snapshots/{id}"))?)?;
                }
            }
        }
        if options.prune {
            report.prune = Some(super::prune::run(
                repo,
                20,
                options.dry_run,
                &SystemClock,
                guard,
            )?);
        }
        Ok(report)
    })
}
pub fn plan(
    snapshots: Vec<(String, Snapshot)>,
    options: &Options,
    explicit: &HashSet<String>,
    mut offset: impl FnMut(OffsetDateTime) -> Option<UtcOffset>,
) -> Result<Report, Error> {
    options.validate()?;
    let grouping = options
        .group_by
        .as_deref()
        .unwrap_or("host,paths")
        .split(',')
        .collect::<HashSet<_>>();
    let mut groups: BTreeMap<GroupKey, Vec<(String, Snapshot)>> = BTreeMap::new();
    for (id, snapshot) in snapshots {
        let key = GroupKey {
            host: grouping.contains("host").then(|| snapshot.host.clone()),
            paths: grouping.contains("paths").then(|| {
                let mut paths = snapshot.paths.clone();
                paths.sort();
                paths.dedup();
                paths
            }),
        };
        groups.entry(key).or_default().push((id, snapshot));
    }
    let mut report = Report {
        groups: Vec::new(),
        prune: None,
    };
    let mut warned = false;
    for (key, mut snapshots) in groups {
        snapshots.sort_by(|a, b| b.1.time.cmp(&a.1.time).then(b.0.cmp(&a.0)));
        let mut group = Group {
            key,
            keep: Vec::new(),
            remove: Vec::new(),
        };
        let mut buckets: [HashSet<String>; 5] = std::array::from_fn(|_| HashSet::new());
        for (position, (id, snapshot)) in snapshots.into_iter().enumerate() {
            let mut reasons = Vec::new();
            if !options.snapshots.is_empty() {
                if !explicit.contains(&id) {
                    reasons.push("not-selected".into());
                }
            } else {
                if options.keep_last.is_some_and(|n| position < n) {
                    reasons.push("last".into());
                }
                let utc = OffsetDateTime::parse(
                    &snapshot.time,
                    &time::format_description::well_known::Rfc3339,
                )
                .map_err(|e| Error::Damage {
                    object: format!("snapshots/{id}"),
                    problem: "bad-format",
                    detail: e.to_string(),
                })?;
                let local = utc.to_offset(offset(utc).unwrap_or_else(|| {
                    if !warned {
                        crate::warning!(
                            "warning: local time zone offset unavailable; retention uses UTC"
                        );
                        warned = true;
                    }
                    UtcOffset::UTC
                }));
                let (week_year, week, _) = local.to_iso_week_date();
                let keys = [
                    format!(
                        "{}-{}-{}-{}",
                        local.year(),
                        local.month() as u8,
                        local.day(),
                        local.hour()
                    ),
                    format!("{}-{}-{}", local.year(), local.month() as u8, local.day()),
                    format!("{week_year}-{week}"),
                    format!("{}-{}", local.year(), local.month() as u8),
                    format!("{}", local.year()),
                ];
                for (i, (count, name)) in [
                    (options.keep_hourly, "hourly"),
                    (options.keep_daily, "daily"),
                    (options.keep_weekly, "weekly"),
                    (options.keep_monthly, "monthly"),
                    (options.keep_yearly, "yearly"),
                ]
                .into_iter()
                .enumerate()
                {
                    if count.is_some_and(|n| buckets[i].len() < n)
                        && buckets[i].insert(keys[i].clone())
                    {
                        reasons.push(name.into());
                    }
                }
                for tag in &options.keep_tag {
                    if snapshot.tags.contains(tag) {
                        reasons.push(format!("tag:{tag}"));
                    }
                }
            }
            if reasons.is_empty() {
                group.remove.push(id);
            } else {
                group.keep.push(Kept { id, reasons });
            }
        }
        report.groups.push(group);
    }
    Ok(report)
}
