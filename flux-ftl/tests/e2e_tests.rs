// ---------------------------------------------------------------------------
// Phase 18: End-to-End Pipeline Tests
//
// Full pipeline coverage: Parse -> Validate -> Type-Check -> Region-Check
// -> Z3 Prove -> Compile (BLAKE3) -> Codegen (LLVM IR) -> Binary
// ---------------------------------------------------------------------------

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Build a `Command` for the flux-ftl binary.
fn flux_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flux-ftl"))
}

/// Generate a unique temp path incorporating the test name to avoid collisions
/// when tests run in parallel.
fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("flux_e2e_{}", name))
}

/// Clean up a file if it exists. Ignores errors.
fn cleanup(path: &PathBuf) {
    let _ = fs::remove_file(path);
}

// ===========================================================================
// Full Pipeline Tests — hello_world.ftl
// ===========================================================================

#[test]
fn e2e_hello_world_check() {
    let output = flux_cmd()
        .args(["check", "testdata/hello_world.ftl"])
        .output()
        .expect("failed to execute flux-ftl check");

    assert!(
        output.status.success(),
        "check should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert_eq!(json["status"], "OK", "status should be OK");
    assert!(
        json["proof_results"].is_array(),
        "proof_results should be present as array"
    );
    assert!(
        !json["proof_results"]
            .as_array()
            .unwrap()
            .is_empty(),
        "proof_results should not be empty for hello_world"
    );
}

#[test]
fn e2e_hello_world_compile() {
    let out = temp_path("hw_compile.flux.bin");
    cleanup(&out);

    let output = flux_cmd()
        .args([
            "compile",
            "testdata/hello_world.ftl",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute flux-ftl compile");

    assert!(
        output.status.success(),
        "compile should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.exists(), "compiled binary file should exist");

    let bytes = fs::read(&out).expect("failed to read compiled binary");
    assert!(bytes.len() > 4, "compiled binary should have content");
    assert_eq!(&bytes[..4], b"FLUX", "compiled binary should start with FLUX magic");

    cleanup(&out);
}

#[test]
fn e2e_hello_world_build_and_run() {
    let out = temp_path("hw_build_run");
    cleanup(&out);

    let output = flux_cmd()
        .args([
            "build",
            "testdata/hello_world.ftl",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute flux-ftl build");

    assert!(
        output.status.success(),
        "build should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.exists(), "built executable should exist");

    // Run the built executable
    let run_output = Command::new(&out)
        .output()
        .expect("failed to run built binary");

    assert!(
        run_output.status.success(),
        "built binary should exit 0"
    );
    assert_eq!(
        String::from_utf8_lossy(&run_output.stdout),
        "Hello World\n",
        "built binary should print 'Hello World\\n'"
    );

    cleanup(&out);
}

#[test]
fn e2e_hello_world_ir() {
    let output = flux_cmd()
        .args(["ir", "testdata/hello_world.ftl"])
        .output()
        .expect("failed to execute flux-ftl ir");

    assert!(
        output.status.success(),
        "ir should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("define"),
        "IR output should contain 'define'"
    );
    assert!(
        stdout.contains("target triple"),
        "IR output should contain target triple directive"
    );
}

// ===========================================================================
// Error Handling Tests
// ===========================================================================

#[test]
fn e2e_invalid_ftl_returns_error() {
    let tmp = temp_path("invalid_syntax.ftl");
    fs::write(&tmp, "THIS IS NOT VALID FTL @@@ !!!").expect("failed to write temp file");

    let output = flux_cmd()
        .args(["check", tmp.to_str().unwrap()])
        .output()
        .expect("failed to execute flux-ftl check");

    assert!(
        !output.status.success(),
        "check on invalid FTL should return non-zero exit code"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain error indication — either JSON with error status or error text
    assert!(
        stdout.contains("PARSE_ERROR") || stdout.contains("parse_errors") || stdout.contains("error"),
        "output should indicate a parse error, got: {}",
        stdout
    );

    cleanup(&tmp);
}

#[test]
fn e2e_nonexistent_file_error() {
    let output = flux_cmd()
        .args(["check", "totally_nonexistent_file_12345.ftl"])
        .output()
        .expect("failed to execute flux-ftl check");

    assert!(
        !output.status.success(),
        "check on nonexistent file should return non-zero exit code"
    );

    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit code 2 for missing file"
    );
}

#[test]
fn e2e_compile_nonexistent_file_error() {
    let output = flux_cmd()
        .args(["compile", "totally_nonexistent_file_12345.ftl"])
        .output()
        .expect("failed to execute flux-ftl compile");

    assert!(
        !output.status.success(),
        "compile on nonexistent file should fail"
    );
}

#[test]
fn e2e_build_nonexistent_file_error() {
    let output = flux_cmd()
        .args(["build", "totally_nonexistent_file_12345.ftl"])
        .output()
        .expect("failed to execute flux-ftl build");

    assert!(
        !output.status.success(),
        "build on nonexistent file should fail"
    );
}

// ===========================================================================
// Determinism Tests
// ===========================================================================

#[test]
fn e2e_compile_deterministic() {
    let out1 = temp_path("determ_1.flux.bin");
    let out2 = temp_path("determ_2.flux.bin");
    cleanup(&out1);
    cleanup(&out2);

    // First compile
    let r1 = flux_cmd()
        .args([
            "compile",
            "testdata/hello_world.ftl",
            "-o",
            out1.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute first compile");
    assert!(r1.status.success(), "first compile should succeed");

    // Second compile
    let r2 = flux_cmd()
        .args([
            "compile",
            "testdata/hello_world.ftl",
            "-o",
            out2.to_str().unwrap(),
        ])
        .output()
        .expect("failed to execute second compile");
    assert!(r2.status.success(), "second compile should succeed");

    let bytes1 = fs::read(&out1).expect("read first binary");
    let bytes2 = fs::read(&out2).expect("read second binary");

    assert_eq!(
        bytes1, bytes2,
        "two compiles of the same source should produce identical binaries"
    );

    cleanup(&out1);
    cleanup(&out2);
}

#[test]
fn e2e_evolution_deterministic_with_seed() {
    // First run with seed 42
    let r1 = flux_cmd()
        .args([
            "evolve",
            "testdata/hello_world.ftl",
            "--generations",
            "2",
            "--population",
            "5",
            "--seed",
            "42",
        ])
        .output()
        .expect("failed to execute first evolve");

    assert!(
        r1.status.success(),
        "first evolve should succeed, stderr: {}",
        String::from_utf8_lossy(&r1.stderr)
    );

    // Second run with same seed
    let r2 = flux_cmd()
        .args([
            "evolve",
            "testdata/hello_world.ftl",
            "--generations",
            "2",
            "--population",
            "5",
            "--seed",
            "42",
        ])
        .output()
        .expect("failed to execute second evolve");

    assert!(
        r2.status.success(),
        "second evolve should succeed, stderr: {}",
        String::from_utf8_lossy(&r2.stderr)
    );

    let stdout1 = String::from_utf8_lossy(&r1.stdout);
    let stdout2 = String::from_utf8_lossy(&r2.stdout);

    // Both should produce valid JSON
    let json1: serde_json::Value =
        serde_json::from_str(&stdout1).expect("first evolve stdout should be valid JSON");
    let json2: serde_json::Value =
        serde_json::from_str(&stdout2).expect("second evolve stdout should be valid JSON");

    assert_eq!(
        json1, json2,
        "two evolve runs with the same seed should produce identical JSON output"
    );
}

// ===========================================================================
// Complex Programs — Validation
// ===========================================================================

#[test]
fn e2e_snake_game_validates() {
    let output = flux_cmd()
        .args(["check", "testdata/snake_game.ftl"])
        .output()
        .expect("failed to execute flux-ftl check on snake_game");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    // snake_game should parse and validate (may have proof warnings but should not crash)
    assert!(
        json["status"] == "OK"
            || json["status"] == "PROOF_FAIL"
            || json["status"] == "VALIDATION_FAIL",
        "snake_game check should produce a valid status, got: {}",
        json["status"]
    );

    // The AST should be present (parsing succeeded)
    assert!(
        json["ast"].is_object(),
        "snake_game should produce an AST"
    );
}

#[test]
fn e2e_ffi_validates_and_proves() {
    let output = flux_cmd()
        .args(["check", "testdata/ffi.ftl"])
        .output()
        .expect("failed to execute flux-ftl check on ffi");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert_eq!(json["status"], "OK", "ffi.ftl should check OK");

    // Verify proof_results exist and contain ASSUMED entries (from trust: EXTERN)
    let proofs = json["proof_results"].as_array().expect("proof_results should be array");
    let has_assumed = proofs.iter().any(|p| {
        let status = p["status"].as_str().unwrap_or("");
        status == "ASSUMED"
    });
    assert!(
        has_assumed,
        "ffi.ftl proofs should contain ASSUMED entries, got: {:?}",
        proofs
    );
}

#[test]
fn e2e_concurrency_validates() {
    let output = flux_cmd()
        .args(["check", "testdata/concurrency.ftl"])
        .output()
        .expect("failed to execute flux-ftl check on concurrency");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    // concurrency.ftl parses and validates, but some contracts may be disproven
    // (e.g. counter invariant is intentionally tight). Accept OK or PROOF_FAIL.
    assert!(
        json["status"] == "OK" || json["status"] == "PROOF_FAIL",
        "concurrency.ftl should parse and validate, got status: {}",
        json["status"]
    );

    // The AST should always be present (parsing succeeded)
    assert!(
        json["ast"].is_object(),
        "concurrency.ftl should produce an AST"
    );

    // Proof results should be present
    assert!(
        json["proof_results"].is_array(),
        "proof_results should be present"
    );
}

// ===========================================================================
// BMC Tests — Bounded Model Checking
// ===========================================================================

#[test]
fn e2e_check_with_bmc() {
    let output = flux_cmd()
        .args([
            "check",
            "testdata/hello_world.ftl",
            "--bmc",
            "--bmc-depth",
            "5",
        ])
        .output()
        .expect("failed to execute flux-ftl check --bmc");

    assert!(
        output.status.success(),
        "check --bmc should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert_eq!(json["status"], "OK", "check with BMC should produce OK status");

    // proof_results should be present
    assert!(
        json["proof_results"].is_array(),
        "proof_results should be present with BMC"
    );

    // Check for PROVEN or BMC_PROVEN status in proof results
    let proofs = json["proof_results"].as_array().unwrap();
    let has_proven = proofs.iter().any(|p| {
        let status = p["status"].as_str().unwrap_or("");
        status == "PROVEN" || status == "BMC_PROVEN"
    });
    assert!(
        has_proven,
        "BMC check should produce proven results, got: {:?}",
        proofs
    );
}

#[test]
fn e2e_check_with_bmc_ffi() {
    let output = flux_cmd()
        .args([
            "check",
            "testdata/ffi.ftl",
            "--bmc",
            "--bmc-depth",
            "3",
        ])
        .output()
        .expect("failed to execute flux-ftl check --bmc on ffi");

    assert!(
        output.status.success(),
        "check --bmc on ffi should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert_eq!(json["status"], "OK", "ffi with BMC should be OK");
}

// ===========================================================================
// Multi-Target IR Tests
// ===========================================================================

#[test]
fn e2e_ir_arm64() {
    let output = flux_cmd()
        .args(["ir", "testdata/hello_world.ftl", "--target", "arm64"])
        .output()
        .expect("failed to execute flux-ftl ir --target arm64");

    assert!(
        output.status.success(),
        "ir --target arm64 should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("aarch64"),
        "IR for arm64 should contain 'aarch64' in target triple, got first lines:\n{}",
        stdout.lines().take(5).collect::<Vec<_>>().join("\n")
    );
    assert!(
        stdout.contains("define"),
        "IR should contain function definitions"
    );
}

#[test]
fn e2e_ir_wasm() {
    let output = flux_cmd()
        .args(["ir", "testdata/hello_world.ftl", "--target", "wasm32"])
        .output()
        .expect("failed to execute flux-ftl ir --target wasm32");

    assert!(
        output.status.success(),
        "ir --target wasm32 should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("wasm"),
        "IR for wasm32 should contain 'wasm' in target triple, got first lines:\n{}",
        stdout.lines().take(5).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn e2e_ir_x86_64() {
    let output = flux_cmd()
        .args(["ir", "testdata/hello_world.ftl", "--target", "x86_64"])
        .output()
        .expect("failed to execute flux-ftl ir --target x86_64");

    assert!(
        output.status.success(),
        "ir --target x86_64 should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("x86_64"),
        "IR for x86_64 should contain 'x86_64' in target triple"
    );
}

// Note: riscv64 IR test is omitted because LLVM 14 does not support
// the generic-rv64 CPU on all platforms. The FluxTarget::Riscv64 parsing
// is tested in codegen_tests.rs (test_target_from_str).

// ===========================================================================
// Pipeline Completeness — Full round-trip with proof details
// ===========================================================================

#[test]
fn e2e_full_pipeline_proof_details() {
    // Verify that the full pipeline produces detailed proof information
    let output = flux_cmd()
        .args(["check", "testdata/hello_world.ftl"])
        .output()
        .expect("failed to execute");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify all pipeline stages produced results
    assert_eq!(json["status"], "OK");
    assert!(json["ast"].is_object(), "AST should be present");
    assert!(json["proof_results"].is_array(), "proof_results should be present");
    assert!(json["compiled"].is_object(), "compiled metadata should be present");

    // Verify compiled metadata has BLAKE3 entry_hash
    let compiled = &json["compiled"];
    assert!(
        compiled["entry_hash"].is_string(),
        "compiled should have an entry_hash field"
    );
    let hash = compiled["entry_hash"].as_str().unwrap();
    assert!(
        !hash.is_empty(),
        "BLAKE3 entry_hash should not be empty"
    );
    assert!(
        compiled["total_nodes"].is_number(),
        "compiled should have total_nodes"
    );
}

#[test]
fn e2e_full_pipeline_feedback() {
    // Verify that feedback is generated as part of the check pipeline
    let output = flux_cmd()
        .args(["check", "testdata/hello_world.ftl"])
        .output()
        .expect("failed to execute");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert!(
        json["feedback"].is_object(),
        "feedback should be present in check output"
    );
}

// ===========================================================================
// Build with optimization levels
// ===========================================================================

#[test]
fn e2e_build_opt_level_0() {
    let out = temp_path("hw_opt0");
    cleanup(&out);

    let output = flux_cmd()
        .args([
            "build",
            "testdata/hello_world.ftl",
            "-o",
            out.to_str().unwrap(),
            "--opt-level",
            "0",
        ])
        .output()
        .expect("failed to execute build --opt-level 0");

    assert!(
        output.status.success(),
        "build --opt-level 0 should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.exists(), "executable should exist");

    // Verify the binary still works correctly
    let run_output = Command::new(&out)
        .output()
        .expect("failed to run opt-0 binary");
    assert_eq!(
        String::from_utf8_lossy(&run_output.stdout),
        "Hello World\n"
    );

    cleanup(&out);
}

#[test]
fn e2e_build_opt_level_3() {
    let out = temp_path("hw_opt3");
    cleanup(&out);

    let output = flux_cmd()
        .args([
            "build",
            "testdata/hello_world.ftl",
            "-o",
            out.to_str().unwrap(),
            "--opt-level",
            "3",
        ])
        .output()
        .expect("failed to execute build --opt-level 3");

    assert!(
        output.status.success(),
        "build --opt-level 3 should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.exists(), "executable should exist");

    // Verify the binary still works correctly at higher opt
    let run_output = Command::new(&out)
        .output()
        .expect("failed to run opt-3 binary");
    assert_eq!(
        String::from_utf8_lossy(&run_output.stdout),
        "Hello World\n"
    );

    cleanup(&out);
}

// ===========================================================================
// Build with BMC verification
// ===========================================================================

#[test]
fn e2e_build_with_bmc() {
    let out = temp_path("hw_bmc_build");
    cleanup(&out);

    let output = flux_cmd()
        .args([
            "build",
            "testdata/hello_world.ftl",
            "-o",
            out.to_str().unwrap(),
            "--bmc",
            "--bmc-depth",
            "5",
        ])
        .output()
        .expect("failed to execute build --bmc");

    assert!(
        output.status.success(),
        "build --bmc should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.exists(), "executable should exist");

    let run_output = Command::new(&out)
        .output()
        .expect("failed to run bmc-verified binary");
    assert_eq!(
        String::from_utf8_lossy(&run_output.stdout),
        "Hello World\n"
    );

    cleanup(&out);
}

// ===========================================================================
// IR output for complex programs
// ===========================================================================

#[test]
fn e2e_ffi_ir_contains_extern_calls() {
    let output = flux_cmd()
        .args(["ir", "testdata/ffi.ftl"])
        .output()
        .expect("failed to execute flux-ftl ir on ffi");

    assert!(
        output.status.success(),
        "ir on ffi should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("@malloc"), "IR should declare malloc");
    assert!(stdout.contains("@free"), "IR should declare free");
    assert!(stdout.contains("@fopen"), "IR should declare fopen");
    assert!(stdout.contains("define"), "IR should contain function definitions");
}

#[test]
fn e2e_concurrency_ir() {
    // concurrency.ftl has PROOF_FAIL status, so IR generation will fail
    // because the `ir` subcommand runs the full check pipeline first and
    // aborts on non-OK status. Verify it exits with code 1 (check failure).
    let output = flux_cmd()
        .args(["ir", "testdata/concurrency.ftl"])
        .output()
        .expect("failed to execute flux-ftl ir on concurrency");

    // concurrency.ftl has a disproven contract, so ir will fail with exit 1
    assert!(
        !output.status.success(),
        "ir on concurrency.ftl should fail due to proof failures"
    );

    // stdout should contain the check JSON with the failure details
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("PROOF_FAIL") || stdout.contains("DISPROVEN"),
        "output should indicate proof failure"
    );
}

// ===========================================================================
// Stdin pipeline compatibility
// ===========================================================================

#[test]
fn e2e_stdin_full_pipeline() {
    let source =
        fs::read_to_string("testdata/hello_world.ftl").expect("failed to read hello_world.ftl");

    let mut child = flux_cmd()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn flux-ftl");

    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(source.as_bytes()).unwrap();
    }

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stdin pipeline should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdin output should be valid JSON");
    assert_eq!(json["status"], "OK");
}
