// ---------------------------------------------------------------------------
// Integration tests for the LLVM codegen module
// ---------------------------------------------------------------------------

use std::io::Write;
use std::process::Command;

use flux_ftl::codegen::{codegen, CodegenConfig, FluxTarget, OutputFormat};
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

// ---------------------------------------------------------------------------
// Phase 17: Multi-Target Codegen tests
// ---------------------------------------------------------------------------

#[test]
fn test_target_from_str() {
    assert_eq!(FluxTarget::parse("x86_64").unwrap(), FluxTarget::X86_64);
    assert_eq!(FluxTarget::parse("x86-64").unwrap(), FluxTarget::X86_64);
    assert_eq!(
        FluxTarget::parse("aarch64").unwrap(),
        FluxTarget::Aarch64
    );
    assert_eq!(FluxTarget::parse("arm64").unwrap(), FluxTarget::Aarch64);
    assert_eq!(
        FluxTarget::parse("riscv64").unwrap(),
        FluxTarget::Riscv64
    );
    assert_eq!(FluxTarget::parse("wasm32").unwrap(), FluxTarget::Wasm32);
    assert_eq!(FluxTarget::parse("wasm").unwrap(), FluxTarget::Wasm32);
    assert_eq!(FluxTarget::parse("host").unwrap(), FluxTarget::Host);
    assert_eq!(FluxTarget::parse("native").unwrap(), FluxTarget::Host);
    assert!(FluxTarget::parse("unknown-arch").is_err());
}

#[test]
fn test_target_triple() {
    assert_eq!(FluxTarget::X86_64.triple(), "x86_64-unknown-linux-gnu");
    assert_eq!(FluxTarget::Aarch64.triple(), "aarch64-unknown-linux-gnu");
    assert_eq!(FluxTarget::Riscv64.triple(), "riscv64-unknown-linux-gnu");
    assert_eq!(FluxTarget::Wasm32.triple(), "wasm32-unknown-unknown");
    assert_eq!(FluxTarget::Host.triple(), "host");
}

#[test]
fn test_ir_x86_64_target() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        target: FluxTarget::X86_64,
        target_triple: FluxTarget::X86_64.triple().to_string(),
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen for x86_64 failed");
    assert!(
        result.llvm_ir.contains("x86_64"),
        "IR must contain x86_64 target triple, got:\n{}",
        result.llvm_ir.lines().take(5).collect::<Vec<_>>().join("\n")
    );
    assert!(
        result.llvm_ir.contains("define i32 @main"),
        "IR must define main"
    );
}

#[test]
fn test_ir_aarch64_target() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        target: FluxTarget::Aarch64,
        target_triple: FluxTarget::Aarch64.triple().to_string(),
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen for aarch64 failed");
    assert!(
        result.llvm_ir.contains("aarch64"),
        "IR must contain aarch64 target triple, got:\n{}",
        result.llvm_ir.lines().take(5).collect::<Vec<_>>().join("\n")
    );
    assert!(
        result.llvm_ir.contains("define i32 @main"),
        "IR must define main"
    );
}

#[test]
fn test_ir_wasm32_target() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        target: FluxTarget::Wasm32,
        target_triple: FluxTarget::Wasm32.triple().to_string(),
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen for wasm32 failed");
    assert!(
        result.llvm_ir.contains("wasm32"),
        "IR must contain wasm32 target triple, got:\n{}",
        result.llvm_ir.lines().take(5).collect::<Vec<_>>().join("\n")
    );
    assert!(
        result.llvm_ir.contains("define i32 @main"),
        "IR must define main"
    );
}

#[test]
fn test_ir_host_target() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        target: FluxTarget::Host,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen for host failed");
    assert!(
        result.llvm_ir.contains("define i32 @main"),
        "IR must define main for host target"
    );
    // Host target should have a target triple set in the module
    assert!(
        result.llvm_ir.contains("target triple"),
        "IR must contain target triple directive"
    );
}

// ---------------------------------------------------------------------------
// Phase 22: Debug Info and LTO tests
// ---------------------------------------------------------------------------

#[test]
fn test_debug_info_compiles() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        emit_debug_info: true,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen with debug_info failed");
    assert!(
        result.llvm_ir.contains("define i32 @main"),
        "IR must define main with debug info enabled"
    );
    // Debug info should add !dbg metadata or DICompileUnit
    assert!(
        result.llvm_ir.contains("!llvm.dbg") || result.llvm_ir.contains("DICompileUnit"),
        "IR must contain debug info metadata"
    );
}

#[test]
fn test_debug_info_object_file() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::ObjectFile,
        emit_debug_info: true,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen with debug_info to object failed");
    assert!(result.output_bytes.len() > 4, "object file too small");
    assert_eq!(
        &result.output_bytes[..4],
        b"\x7fELF",
        "output must be a valid ELF object file with debug info"
    );
}

#[test]
fn test_lto_compiles() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        lto: true,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen with LTO failed");
    assert!(
        result.llvm_ir.contains("define i32 @main"),
        "IR must define main with LTO enabled"
    );
}

#[test]
fn test_lto_object_file() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::ObjectFile,
        lto: true,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen with LTO to object failed");
    assert!(result.output_bytes.len() > 4, "object file too small");
    assert_eq!(
        &result.output_bytes[..4],
        b"\x7fELF",
        "output must be a valid ELF object file with LTO"
    );
}

#[test]
fn test_bitcode_output() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::Bitcode,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen to bitcode failed");
    assert!(!result.output_bytes.is_empty(), "bitcode should not be empty");
    // LLVM bitcode starts with 'BC' magic
    assert!(
        result.output_bytes.len() > 2
            && (result.output_bytes[0] == b'B' && result.output_bytes[1] == b'C'),
        "output must start with BC bitcode magic"
    );
}

#[test]
fn test_debug_info_and_lto_combined() {
    let program = parse_file("testdata/hello_world.ftl");
    let config = CodegenConfig {
        output_format: OutputFormat::ObjectFile,
        emit_debug_info: true,
        lto: true,
        ..CodegenConfig::default()
    };
    let result = codegen(&program, &config).expect("codegen with debug_info+LTO failed");
    assert!(result.output_bytes.len() > 4, "object file too small");
    assert_eq!(
        &result.output_bytes[..4],
        b"\x7fELF",
        "output must be a valid ELF with debug info and LTO"
    );
}
