use std::process::{Command, Output};

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quick-share"))
        .args(arguments)
        .output()
        .expect("quick-share binary should run")
}

#[test]
fn help_identifies_the_rust_rewrite_cli() {
    // Arrange + Act
    let output = run(&["--help"]);
    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");

    // Assert
    assert!(output.status.success());
    assert!(stdout.contains("Quick Share"));
    assert!(stdout.contains("Usage:"));
}

#[test]
fn send_help_states_the_safe_web_fallback_boundary() {
    // Arrange + Act
    let output = run(&["send", "--help"]);
    let stdout = String::from_utf8(output.stdout).expect("help should be UTF-8");

    // Assert
    assert!(output.status.success());
    assert!(stdout.contains("zero compatible receivers"));
    assert!(stdout.contains("does not fall back to Web mode"));
}

#[test]
fn version_comes_from_the_workspace_package() {
    // Arrange + Act
    let output = run(&["--version"]);
    let stdout = String::from_utf8(output.stdout).expect("version should be UTF-8");

    // Assert
    assert!(output.status.success());
    assert_eq!(
        stdout.trim(),
        format!("quick-share {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn no_arguments_prints_help_and_returns_usage_error() {
    // Arrange + Act
    let output = run(&[]);
    let stderr = String::from_utf8(output.stderr).expect("error should be UTF-8");

    // Assert
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr.contains("Usage:"));
}

#[cfg(not(windows))]
#[test]
fn agent_reports_the_first_phase_windows_boundary() {
    let output = run(&["agent"]);
    let stderr = String::from_utf8(output.stderr).expect("error should be UTF-8");

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr.contains("Windows"));
    assert!(stderr.contains("agent"));
}

#[test]
fn workspace_contains_the_planned_crate_boundaries() {
    // Arrange
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("CLI crate should be two levels below workspace");
    let crates = [
        "quick-share-cli",
        "quick-share-core",
        "quick-share-protocol",
        "quick-share-discovery",
        "quick-share-transfer",
        "quick-share-web",
        "quick-share-platform",
    ];

    // Act + Assert
    for crate_name in crates {
        assert!(
            workspace
                .join("crates")
                .join(crate_name)
                .join("Cargo.toml")
                .is_file(),
            "missing planned crate {crate_name}"
        );
    }
}
