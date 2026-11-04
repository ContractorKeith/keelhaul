use super::{compose, connect};
use keelhaul::backend::{Backend, BackendError, Kind, ObjectName, sftp::SftpBackend};
use std::path::Path;
fn backend(config: &Path, root: &str) -> SftpBackend {
    SftpBackend::with_client(root.into(), connect(config, root)).unwrap()
}
fn remote(args: &[&str]) {
    let mut command = vec!["exec", "--user", "keelhaul", "-T", "sshd"];
    command.extend_from_slice(args);
    compose(&command);
}
fn link(target: &str, path: &str) {
    remote(&["ln", "-s", target, path]);
}
pub fn run(config: &Path, root: &str) {
    let object = ObjectName::new("config").unwrap();
    let storage_root = format!("{root}-link-storage");
    let storage = backend(config, &storage_root);
    storage.write(&object, b"config bytes").unwrap();
    let linked_root = format!("{root}-link-config");
    let linked = backend(config, &linked_root);
    link(
        &format!("{storage_root}/config"),
        &format!("{linked_root}/config"),
    );
    assert_eq!(linked.read(&object).unwrap(), b"config bytes");
    assert_eq!(linked.stat(&object).unwrap(), 12);
    linked.write(&object, b"config bytes").unwrap();
    assert_eq!(
        linked.list(Kind::Config).unwrap(),
        storage.list(Kind::Config).unwrap()
    );
    assert!(linked.list_foreign(Kind::Config).unwrap().is_empty());
    linked.delete(&object).unwrap();
    assert_eq!(storage.read(&object).unwrap(), b"config bytes");

    let wrong_root = format!("{root}-directory-is-file");
    let wrong = backend(config, &wrong_root);
    link(
        &format!("{storage_root}/config"),
        &format!("{wrong_root}/keys"),
    );
    assert!(wrong.list(Kind::Keys).is_err());

    let pack = ObjectName::new(&format!("data/ab/{}", "ab".repeat(32))).unwrap();
    for whole_data in [true, false] {
        let disk_root = format!("{root}-disk-{whole_data}");
        let disk = backend(config, &disk_root);
        disk.write(&pack, b"pack bytes").unwrap();
        let linked_root = format!("{root}-shard-{whole_data}");
        let linked = backend(config, &linked_root);
        if whole_data {
            link(&format!("{disk_root}/data"), &format!("{linked_root}/data"));
        } else {
            remote(&["mkdir", &format!("{linked_root}/data")]);
            link(
                &format!("{disk_root}/data/ab"),
                &format!("{linked_root}/data/ab"),
            );
        }
        assert_eq!(linked.read(&pack).unwrap(), b"pack bytes");
        assert_eq!(linked.stat(&pack).unwrap(), 10);
        assert_eq!(
            linked.list(Kind::Data).unwrap(),
            disk.list(Kind::Data).unwrap()
        );
        linked.delete(&pack).unwrap();
        linked.write(&pack, b"new pack bytes").unwrap();
        assert_eq!(disk.read(&pack).unwrap(), b"new pack bytes");
        let raw = "data/ab/junk.tmp-0123456789abcdef";
        linked.plant_raw(raw, b"partial").unwrap();
        let foreign = linked.list_foreign(Kind::Data).unwrap();
        assert_eq!(foreign.len(), 1);
        assert_eq!(foreign[0].raw, raw);
        assert!(foreign[0].deletable);
        linked.delete_foreign(raw).unwrap();
        assert!(linked.list_foreign(Kind::Data).unwrap().is_empty());
    }

    link(&storage_root, &format!("{linked_root}/parent"));
    let nested = backend(config, &format!("{linked_root}/parent/repo"));
    nested.write(&object, b"through symlinked parent").unwrap();
    assert_eq!(nested.read(&object).unwrap(), b"through symlinked parent");
    assert_eq!(nested.list(Kind::Config).unwrap().len(), 1);

    let broken_root = format!("{root}-broken");
    let broken = backend(config, &broken_root);
    link("absent", &format!("{broken_root}/config"));
    link("unknown", &format!("{broken_root}/unknown"));
    assert!(broken.list(Kind::Config).unwrap().is_empty());
    let entries = broken.list_foreign(Kind::Config).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|e| (e.raw.as_str(), e.deletable))
            .collect::<Vec<_>>(),
        vec![("config", false), ("unknown", true)]
    );
    assert!(matches!(
        broken.read(&object),
        Err(BackendError::NotFound(_))
    ));
    broken.delete_foreign("unknown").unwrap();
    assert!(matches!(
        broken.delete_foreign("config"),
        Err(BackendError::InvalidName(_))
    ));
    remote(&["mkdir", "-p", &format!("{broken_root}/data/ab")]);
    link(
        &"ab".repeat(32),
        &format!("{broken_root}/{}", pack.as_str()),
    );
    link("cd", &format!("{broken_root}/data/cd"));
    link("absent", &format!("{broken_root}/data/ef"));
    assert!(broken.list(Kind::Data).unwrap().is_empty());
    let entries = broken.list_foreign(Kind::Data).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|e| (e.raw.as_str(), e.deletable))
            .collect::<Vec<_>>(),
        vec![
            (pack.as_str(), false),
            ("data/cd", false),
            ("data/ef", false)
        ]
    );
    for entry in entries {
        assert!(matches!(
            broken.delete_foreign(&entry.raw),
            Err(BackendError::InvalidName(_))
        ));
    }
}
