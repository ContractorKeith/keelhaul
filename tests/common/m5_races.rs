use super::*;
use keelhaul::backend::local::LocalBackend;
use std::{
    path::Path,
    process::{Command, Stdio},
    time::Instant,
};
fn wait(path: &Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timeout {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
#[ignore = "subprocess helper for exclusive race"]
fn exclusive_child() {
    let root = std::path::PathBuf::from(std::env::var_os("M5_RACE_ROOT").unwrap());
    let n = std::env::var("M5_RACE_NUMBER").unwrap();
    let repo = Repository::open(
        LocalBackend::open(root.join("repo")).unwrap(),
        &Secret::KeyFile([9; 32]),
    )
    .unwrap();
    fs::write(root.join(format!("ready-{n}")), []).unwrap();
    wait(&root.join("go"));
    match Guard::acquire(&repo.backend, keys(&repo), true, &SystemClock) {
        Ok(guard) => {
            let occupancy = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join("occupied"))
                .expect("two exclusive holders must never coexist");
            fs::write(root.join(format!("result-{n}")), b"held").unwrap();
            wait(&root.join("release"));
            drop(occupancy);
            fs::remove_file(root.join("occupied")).unwrap();
            drop(guard);
        }
        Err(Error::Locked(_)) => {
            fs::write(root.join(format!("result-{n}")), b"conflict").unwrap();
        }
        Err(e) => panic!("{e}"),
    }
}
#[test]
fn twenty_exclusive_process_races_never_both_proceed() {
    for iteration in 0..20 {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        Repository::init(
            LocalBackend::new(root.join("repo")).unwrap(),
            &Secret::KeyFile([9; 32]),
        )
        .unwrap();
        let mut children = Vec::new();
        for n in 0..2 {
            children.push(
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "races::exclusive_child", "--ignored"])
                    .env("M5_RACE_ROOT", root)
                    .env("M5_RACE_NUMBER", n.to_string())
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
        }
        for n in 0..2 {
            wait(&root.join(format!("ready-{n}")));
        }
        fs::write(root.join("go"), []).unwrap();
        for n in 0..2 {
            wait(&root.join(format!("result-{n}")));
        }
        let held = (0..2)
            .filter(|n| fs::read(root.join(format!("result-{n}"))).unwrap() == b"held")
            .count();
        assert!(
            held <= 1,
            "iteration {iteration}: both exclusives proceeded"
        );
        fs::write(root.join("release"), []).unwrap();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
    }
}
#[test]
fn racing_key_removals_leave_exactly_one_slot() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let repo = Repository::init(
        LocalBackend::new(root.join("repo")).unwrap(),
        &Secret::KeyFile([9; 32]),
    )
    .unwrap();
    let first = repo.list_keys().unwrap()[0].0.clone();
    let second = repo.add_key(&Secret::KeyFile([10; 32])).unwrap();
    fs::write(root.join("key"), [9; 32]).unwrap();
    let mut children = Vec::new();
    for id in [first, second] {
        children.push(
            Command::new(env!("CARGO_BIN_EXE_keelhaul"))
                .arg("-r")
                .arg(root.join("repo"))
                .arg("--key-file")
                .arg(root.join("key"))
                .args(["key", "remove", &id])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    let outcomes = children
        .iter_mut()
        .map(|c| c.wait().unwrap())
        .collect::<Vec<_>>();
    assert!(outcomes.iter().any(|s| !s.success()));
    assert_eq!(repo.backend.list(Kind::Keys).unwrap().len(), 1);
}
