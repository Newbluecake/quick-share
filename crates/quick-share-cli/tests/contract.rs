use quick_share_cli::{AppError, ExitStatus, IntentCommand, InteractionPolicy, parse_intent_from};

#[test]
fn complete_command_tree_parses_into_execution_independent_intents() {
    let cases = [
        vec!["quick-share", "send", "file.txt", "folder"],
        vec!["quick-share", "receive", "--output", "downloads", "--once"],
        vec!["quick-share", "serve", "file.txt", "--allow-http"],
        vec!["quick-share", "devices", "list"],
        vec![
            "quick-share",
            "devices",
            "rename",
            "qs_0123456789abcdef0123456789abcdef",
            "laptop",
        ],
        vec![
            "quick-share",
            "devices",
            "remove",
            "qs_0123456789abcdef0123456789abcdef",
            "--yes",
        ],
        vec!["quick-share", "config", "show"],
        vec!["quick-share", "config", "path"],
        vec!["quick-share", "config", "set", "device.name", "laptop"],
        vec!["quick-share", "update", "--check"],
    ];

    for arguments in cases {
        parse_intent_from(arguments).expect("documented command should parse");
    }
}

#[test]
fn argv_zero_shortcuts_inject_send_and_receive_commands() {
    let send =
        parse_intent_from(["sc", "--peer", "192.0.2.10:4242", "file.txt"]).expect("sc shortcut");
    let receive = parse_intent_from(["rc.exe", "--output", "received"]).expect("rc shortcut");

    assert!(matches!(send.command, IntentCommand::Send(_)));
    assert!(matches!(receive.command, IntentCommand::Receive(_)));
}

#[test]
fn explicit_resume_id_is_direct_only_and_preserved_in_the_intent() {
    let transfer_id = "018f47b4-5b3a-7c9d-8123-0123456789ab";
    let intent = parse_intent_from([
        "quick-share",
        "send",
        "--resume",
        transfer_id,
        "--peer",
        "127.0.0.1:4242",
        "file.txt",
    ])
    .expect("resume command");
    let IntentCommand::Send(send) = intent.command else {
        panic!("send intent");
    };
    assert_eq!(send.resume.as_deref(), Some(transfer_id));
    assert!(
        parse_intent_from([
            "quick-share",
            "send",
            "--resume",
            transfer_id,
            "--web",
            "file.txt",
        ])
        .is_err()
    );
}

#[test]
fn send_mode_and_content_conflicts_are_rejected_by_the_parser() {
    for arguments in [
        vec![
            "quick-share",
            "send",
            "--peer",
            "host:42",
            "--web",
            "file.txt",
        ],
        vec!["quick-share", "send", "--text", "hello", "file.txt"],
        vec!["quick-share", "send", "--text", "hello", "--clipboard"],
        vec!["quick-share", "send"],
    ] {
        let error = parse_intent_from(arguments).expect_err("conflicting command must fail");
        assert_eq!(error.exit_code(), 2);
    }
}

#[test]
fn config_set_rejects_secret_or_unknown_keys() {
    let error = parse_intent_from([
        "quick-share",
        "config",
        "set",
        "identity.private-key",
        "must-not-be-accepted",
    ])
    .expect_err("secret configuration key must not parse");
    assert_eq!(error.exit_code(), 2);
}

#[test]
fn upload_password_requires_upload_and_is_redacted_from_diagnostics() {
    let error = parse_intent_from([
        "quick-share",
        "serve",
        "file.txt",
        "--upload-password",
        "diagnostic-secret",
    ])
    .expect_err("upload password without uploads must fail");
    assert_eq!(error.exit_code(), 2);

    let intent = parse_intent_from([
        "quick-share",
        "serve",
        "--upload",
        "--upload-password",
        "diagnostic-secret",
    ])
    .expect("password-protected upload mode");
    let diagnostic = format!("{intent:?}");
    assert!(diagnostic.contains("[REDACTED]"));
    assert!(!diagnostic.contains("diagnostic-secret"));
}

#[test]
fn plaintext_and_custom_certificate_options_cannot_be_combined() {
    let error = parse_intent_from([
        "quick-share",
        "serve",
        "file.txt",
        "--allow-http",
        "--cert",
        "cert.pem",
        "--key",
        "key.pem",
    ])
    .expect_err("HTTP conflicts with TLS certificate options");
    assert_eq!(error.exit_code(), 2);
}

#[test]
fn non_interactive_confirmation_requires_an_explicit_yes() {
    assert!(matches!(
        InteractionPolicy::new(false, false).require_confirmation("remove trusted device"),
        Err(AppError::ConfirmationRequired(_))
    ));
    InteractionPolicy::new(false, true)
        .require_confirmation("remove trusted device")
        .expect("explicit yes is safe in non-interactive mode");
    InteractionPolicy::new(true, false)
        .require_confirmation("remove trusted device")
        .expect("interactive caller may prompt");
}

#[test]
fn application_errors_have_stable_documented_exit_statuses() {
    let cases = [
        (
            AppError::Usage("bad argument".to_owned()),
            ExitStatus::Usage,
        ),
        (AppError::Config("bad config".to_owned()), ExitStatus::Usage),
        (
            AppError::PeerUnavailable("offline".to_owned()),
            ExitStatus::PeerUnavailable,
        ),
        (
            AppError::Rejected("declined".to_owned()),
            ExitStatus::Rejected,
        ),
        (
            AppError::Identity("key mismatch".to_owned()),
            ExitStatus::Identity,
        ),
        (
            AppError::Filesystem("read failed".to_owned()),
            ExitStatus::Filesystem,
        ),
        (
            AppError::Network("scan failed".to_owned()),
            ExitStatus::Network,
        ),
        (
            AppError::Integrity("digest mismatch".to_owned()),
            ExitStatus::Integrity,
        ),
        (
            AppError::Update("replace failed".to_owned()),
            ExitStatus::Update,
        ),
        (AppError::Cancelled, ExitStatus::Success),
    ];

    for (error, expected) in cases {
        assert_eq!(error.status(), expected);
        assert_eq!(error.exit_code(), expected as u8);
    }
}
