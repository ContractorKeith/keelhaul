use super::*;
#[test]
fn upload_errors_name_the_object_and_panics_exit_one() {
    for action in ["fail", "panic"] {
        let scratch = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(scratch.path()).unwrap();
        fixture(&root);
        backup(&root);
        fs::write(root.join("source/new"), b"new data").unwrap();
        let output = command(&root)
            .env("KEELHAUL_FAULT", format!("write:2:{action}"))
            .arg("backup")
            .arg(root.join("source"))
            .output()
            .unwrap();
        let message = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{message}");
        assert!(
            message.contains("data/"),
            "error must name the upload: {message}"
        );
        assert!(!message.contains("I/O error for None"), "{message}");
        if action == "fail" {
            assert!(
                message.contains(root.join("repo/data").to_str().unwrap()),
                "local error must carry its backend path: {message}"
            );
        }
        if action == "panic" {
            assert!(message.contains("upload worker panicked"), "{message}");
        }
        check(&root);
        backup(&root);
        restored(&root);
    }
}
