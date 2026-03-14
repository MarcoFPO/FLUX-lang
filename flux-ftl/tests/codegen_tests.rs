// ---------------------------------------------------------------------------
// Integration tests for the LLVM codegen module
// ---------------------------------------------------------------------------

use std::io::Write;
use std::process::Command;

use flux_ftl::codegen::{codegen, CodegenConfig, OutputFormat};
use flux_ftl::parser::parse_ftl;

fn parse_file(path: &str) -> flux_ftl::ast::Program {
    let source = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let result = parse_ftl(&source);
    result
        .ast
        .unwrap_or_else(|| panic!("parse {path} failed: {:?}", result.errors))
}

// ---------------------------------------------------------------------------
// hello_world.ftl tests
// ---------------------------------------------------------------------------

#[test]
fn hello_world_generates_ir() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen failed");
    assert!(!result.llvm_ir.is_empty(), "IR should not be empty");
    assert!(
        result.llvm_ir.contains("define i32 @main"),
        "IR must define main"
    );
    assert!(
        result.llvm_ir.contains("flux_module"),
        "module should be named flux_module"
    );
}

#[test]
fn hello_world_ir_contains_write_syscall() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen failed");
    assert!(
        result.llvm_ir.contains("@write"),
        "IR must contain call to write"
    );
}

#[test]
fn hello_world_ir_contains_exit_syscall() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen failed");
    assert!(
        result.llvm_ir.contains("@_exit"),
        "IR must contain call to _exit"
    );
}

#[test]
fn hello_world_compiles_to_object() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::ObjectFile,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen failed");
    // ELF object files start with 0x7f ELF
    assert!(result.output_bytes.len() > 4, "object file too small");
    assert_eq!(
        &result.output_bytes[..4],
        b"\x7fELF",
        "output must be a valid ELF object file"
    );
}

#[test]
fn ffi_generates_extern_declarations() {
    let program = parse_file("testdata/ffi.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen failed");
    assert!(
        result.llvm_ir.contains("@malloc"),
        "IR must declare malloc"
    );
    assert!(result.llvm_ir.contains("@free"), "IR must declare free");
    assert!(
        result.llvm_ir.contains("@memcpy"),
        "IR must declare memcpy"
    );
    assert!(
        result.llvm_ir.contains("@fopen"),
        "IR must declare fopen"
    );
    assert!(
        result.llvm_ir.contains("@fwrite"),
        "IR must declare fwrite"
    );
    assert!(
        result.llvm_ir.contains("@fclose"),
        "IR must declare fclose"
    );
}

#[test]
fn hello_world_links_and_runs() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::ObjectFile,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen failed");

    // Write object file to temp location
    let obj_path = "/tmp/flux_hello_world_test.o";
    let bin_path = "/tmp/flux_hello_world_test";
    {
        let mut f = std::fs::File::create(obj_path).expect("create obj file");
        f.write_all(&result.output_bytes).expect("write obj file");
    }

    // Link with cc (gcc/clang)
    let link = Command::new("cc")
        .args([obj_path, "-o", bin_path, "-lc"])
        .output()
        .expect("failed to run linker");
    assert!(
        link.status.success(),
        "linking failed: {}",
        String::from_utf8_lossy(&link.stderr)
    );

    // Run the binary and check output
    let run = Command::new(bin_path)
        .output()
        .expect("failed to run binary");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "Hello World\n",
        "binary must print 'Hello World\\n'"
    );

    // Clean up
    let _ = std::fs::remove_file(obj_path);
    let _ = std::fs::remove_file(bin_path);
}

// ---------------------------------------------------------------------------
// ffi.ftl tests — Phase 10
// ---------------------------------------------------------------------------

#[test]
fn ffi_generates_complete_ir() {
    let program = parse_file("testdata/ffi.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("ffi codegen failed");
    assert!(
        result.llvm_ir.contains("define i32 @main"),
        "IR must define main"
    );
    // Verify all extern calls are present in the IR body
    assert!(
        result.llvm_ir.contains("call") && result.llvm_ir.contains("@malloc"),
        "IR must contain call to malloc"
    );
    assert!(
        result.llvm_ir.contains("@memcpy"),
        "IR must contain memcpy declaration or call"
    );
    assert!(
        result.llvm_ir.contains("@fopen"),
        "IR must contain fopen declaration or call"
    );
    assert!(
        result.llvm_ir.contains("@fwrite"),
        "IR must contain fwrite declaration or call"
    );
    assert!(
        result.llvm_ir.contains("@fclose"),
        "IR must contain fclose declaration or call"
    );
    assert!(
        result.llvm_ir.contains("@free"),
        "IR must contain free declaration or call"
    );
}

#[test]
fn ffi_ir_has_malloc_free() {
    let program = parse_file("testdata/ffi.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("ffi codegen failed");
    // Verify malloc and free are declared with correct signatures
    assert!(
        result.llvm_ir.contains("declare") && result.llvm_ir.contains("@malloc"),
        "malloc must be declared"
    );
    assert!(
        result.llvm_ir.contains("declare") && result.llvm_ir.contains("@free"),
        "free must be declared"
    );
}

// ---------------------------------------------------------------------------
// concurrency.ftl tests — Phase 10
// ---------------------------------------------------------------------------

#[test]
fn concurrency_generates_ir() {
    let program = parse_file("testdata/concurrency.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("concurrency codegen failed");
    assert!(
        result.llvm_ir.contains("define i32 @main"),
        "IR must define main"
    );
    // Should contain alloca for M-node allocations
    assert!(
        result.llvm_ir.contains("alloca"),
        "IR must contain alloca for M-node allocations"
    );
    // Should contain atomic operations
    assert!(
        result.llvm_ir.contains("store atomic") || result.llvm_ir.contains("atomic"),
        "IR must contain atomic operations"
    );
}

// ---------------------------------------------------------------------------
// Branch codegen test — Phase 10
// ---------------------------------------------------------------------------

#[test]
fn branch_codegen() {
    // concurrency.ftl contains K:f_cons_body = branch { ... }
    let program = parse_file("testdata/concurrency.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("branch codegen failed");
    // Branch codegen produces then/else/merge basic blocks
    assert!(
        result.llvm_ir.contains("then") || result.llvm_ir.contains("br i1"),
        "IR must contain branch basic blocks"
    );
}

// ---------------------------------------------------------------------------
// Memory operations in IR — Phase 10
// ---------------------------------------------------------------------------

#[test]
fn memory_ops_in_ir() {
    let program = parse_file("testdata/concurrency.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("memory ops codegen failed");
    // Verify alloca instructions exist for M-node allocs
    assert!(
        result.llvm_ir.contains("alloca"),
        "IR must contain alloca for memory allocations"
    );
    // Verify store instructions exist
    assert!(
        result.llvm_ir.contains("store"),
        "IR must contain store instructions"
    );
    // Verify load instructions exist
    assert!(
        result.llvm_ir.contains("load"),
        "IR must contain load instructions"
    );
}
