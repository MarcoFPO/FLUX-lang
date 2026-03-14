use std::process::Command;

fn flux_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flux-ftl"))
}

#[test]
fn check_file_json() {
    let output = flux_cmd()
        .args(["check", "testdata/hello_world.ftl"])
        .output()
        .expect("failed to execute");

    assert!(output.status.success(), "exit code was not 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout is not valid JSON");

    assert_eq!(json["status"], "OK");
}

#[test]
fn check_stdin_compat() {
    let source = std::fs::read_to_string("testdata/hello_world.ftl")
        .expect("failed to read testdata");

    let mut child = flux_cmd()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn");

    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("failed to open stdin");
        stdin.write_all(source.as_bytes()).expect("failed to write");
    }

    let output = child.wait_with_output().expect("failed to wait");
    assert!(output.status.success(), "exit code was not 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout is not valid JSON");

    assert_eq!(json["status"], "OK");
}

#[test]
fn compile_to_bin() {
    let out_path = std::env::temp_dir().join("flux_cli_test.flux.bin");
    // Clean up before test
    let _ = std::fs::remove_file(&out_path);

    let output = flux_cmd()
        .args([
            "compile",
            "testdata/hello_world.ftl",
            "-o",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute");

    assert!(
        output.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out_path.exists(), "binary file was not created");

    // Verify it starts with the FLUX magic bytes
    let bytes = std::fs::read(&out_path).expect("failed to read bin");
    assert!(bytes.len() >= 4);
    assert_eq!(&bytes[..4], b"FLUX");

    // Clean up
    let _ = std::fs::remove_file(&out_path);
}

#[test]
fn build_hello_world() {
    let out_path = std::env::temp_dir().join("flux_cli_test_hw");
    // Clean up before test
    let _ = std::fs::remove_file(&out_path);

    let output = flux_cmd()
        .args([
            "build",
            "testdata/hello_world.ftl",
            "-o",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute");

    assert!(
        output.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out_path.exists(), "executable was not created");

    // Run the built executable
    let run_output = Command::new(&out_path)
        .output()
        .expect("failed to run built binary");

    let stdout = String::from_utf8_lossy(&run_output.stdout);
    assert_eq!(stdout, "Hello World\n");

    // Clean up
    let _ = std::fs::remove_file(&out_path);
}

#[test]
fn ir_output() {
    let output = flux_cmd()
        .args(["ir", "testdata/hello_world.ftl"])
        .output()
        .expect("failed to execute");

    assert!(
        output.status.success(),
        "ir failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("define"), "IR should contain 'define'");
    assert!(stdout.contains("call"), "IR should contain 'call'");
}

#[test]
fn check_nonexistent_file() {
    let output = flux_cmd()
        .args(["check", "nofile.ftl"])
        .output()
        .expect("failed to execute");

    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit code 2 for missing file"
    );
}

#[test]
fn check_text_format() {
    let output = flux_cmd()
        .args(["check", "testdata/hello_world.ftl", "--format", "text"])
        .output()
        .expect("failed to execute");

    assert!(output.status.success(), "exit code was not 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Parse"),
        "text output should contain 'Parse'"
    );
    assert!(
        stdout.contains("Prove"),
        "text output should contain 'Prove'"
    );
}
