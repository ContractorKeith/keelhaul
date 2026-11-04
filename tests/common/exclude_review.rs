use super::*;
#[test]
fn gitignore_literal_braces_and_trailing_spaces() {
    // Cross-checked with `git check-ignore --no-index`: braces are literal;
    // unescaped trailing spaces disappear, while a backslash preserves one.
    let patterns =
        keelhaul::ops::backup::Patterns::new(&["{a,b}".into(), "trim   ".into(), "keep\\ ".into()])
            .unwrap();
    for name in ["{a,b}", "trim", "keep "] {
        assert!(patterns.excluded(Path::new(name), false), "{name:?}");
    }
    for name in ["a", "b", "trim ", "keep"] {
        assert!(!patterns.excluded(Path::new(name), false), "{name:?}");
    }
}
#[test]
fn roots_are_excluded_and_all_excluded_writes_no_snapshot() {
    let scratch = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([2; 32])).unwrap();
    let excluded = root.join("x.tmp");
    fs::write(&excluded, b"skip").unwrap();
    let included = root.join("keep");
    fs::write(&included, b"keep").unwrap();
    let mut policy = options(1);
    policy.excludes = vec!["*.tmp".into()];
    assert!(backup_with_options(&repo, std::slice::from_ref(&excluded), vec![], &policy).is_err());
    assert!(repo.snapshots().unwrap().is_empty());
    let report =
        backup_with_options(&repo, &[excluded.clone(), included], vec![], &policy).unwrap();
    assert_eq!(report.summary.files_read, 1);
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.path == excluded.to_string_lossy() && s.reason.contains("excluded"))
    );
}
#[test]
fn jobs_help_and_bounds() {
    let bin = env!("CARGO_BIN_EXE_keelhaul");
    for value in ["0", "257"] {
        assert_eq!(
            std::process::Command::new(bin)
                .args(["--jobs", value, "version"])
                .output()
                .unwrap()
                .status
                .code(),
            Some(2)
        );
    }
    assert!(
        std::process::Command::new(bin)
            .args(["--jobs", "256", "version"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let help = std::process::Command::new(bin)
        .arg("--help")
        .output()
        .unwrap();
    let text = String::from_utf8(help.stdout)
        .unwrap()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("8 MiB + 2 × pack_target"));
}
