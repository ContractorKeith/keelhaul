use super::*;
#[test]
fn confirmation_checks_both_prompts() {
    for (answers, confirm, succeeds, calls) in [
        (vec!["same", "same"], true, true, 2),
        (vec!["first", "different"], true, false, 2),
        (vec!["single"], false, true, 1),
    ] {
        let mut answers = answers.into_iter();
        let mut prompts = Vec::new();
        let result = prompt_with(confirm, |message| {
            prompts.push(message.to_owned());
            Ok(answers.next().unwrap().into())
        });
        assert_eq!(result.is_ok(), succeeds);
        if !succeeds {
            assert!(matches!(result, Err(Error::Mismatch)));
        }
        assert_eq!(prompts.len(), calls);
        assert_eq!(prompts[0], "Password: ");
        if confirm {
            assert_eq!(prompts[1], "Confirm password: ");
        }
    }
    assert!(matches!(
        prompt_with(true, |_| Err(io::Error::other("read failed"))),
        Err(Error::Io(_))
    ));
}

#[test]
fn strips_exactly_one_line_ending() {
    for (input, expected) in [
        ("pw\n", "pw"),
        ("pw\r\n", "pw"),
        ("pw\n\n", "pw\n"),
        ("pw\r", "pw\r"),
    ] {
        let Secret::Password(actual) = password(input.as_bytes().to_vec()).unwrap() else {
            panic!()
        };
        assert_eq!(actual, expected);
    }
}

#[test]
fn empty_password_reads_remain_allowed_but_writes_are_refused() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("password");
    std::fs::write(
        &path, b"
",
    )
    .unwrap();
    assert!(matches!(load(Some(&path), None, None), Ok(Secret::Password(pw)) if pw.is_empty()));
    assert!(matches!(
        load_for_init(Some(&path), None, None),
        Err(Error::EmptyPassword)
    ));
    assert!(matches!(
        new_password(Some(&path)),
        Err(Error::EmptyPassword)
    ));
}

#[test]
fn command_output_limit_is_inclusive() {
    let bytes = command_password("head -c 1048576 /dev/zero").unwrap();
    assert_eq!(bytes.len(), 1024 * 1024);
    assert!(matches!(
        command_password("head -c 1048577 /dev/zero"),
        Err(Error::CommandOutputTooLarge)
    ));
}
