use std::{path::Path, process::Command};

fn run_validator(arguments: &[&str]) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("python3")
        .current_dir(root)
        .arg("scripts/verify_runtime_smoke_evidence.py")
        .args(arguments)
        .output()
        .expect("python3 must be available for repository contract tests");

    assert!(
        output.status.success(),
        "runtime smoke evidence validator failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn validator_self_test_passes() {
    run_validator(&["--self-test"]);
}

#[test]
fn sanitized_fixture_passes() {
    run_validator(&["tests/fixtures/runtime-smoke-evidence.valid.json"]);
}
