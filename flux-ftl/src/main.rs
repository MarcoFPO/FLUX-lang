use std::io::Read;
use std::process::ExitCode;

use clap::ValueEnum;
use serde::Serialize;

use flux_ftl::codegen::{self, CodegenConfig, OptLevel, OutputFormat};
use flux_ftl::compiler::{self, CompileMetadata};
use flux_ftl::error::Status;
use flux_ftl::feedback::{self, LlmFeedback, ValidationError as FeedbackValidationError};
use flux_ftl::optimizer::{self, OptimizationConfig};
use flux_ftl::parser::parse_ftl;
use flux_ftl::prover::{prove_contracts, ProofResult, ProofStatus, ProverConfig};
use flux_ftl::region_checker::check_regions;
use flux_ftl::type_checker::check_types_and_effects;
use flux_ftl::validator::validate;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(clap::Parser)]
#[command(name = "flux-ftl", version, about = "FLUX Text Language Compiler")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Parse, validate, prove, and generate feedback
    Check {
        #[arg(default_value = "-")]
        file: String,
        #[arg(long, default_value = "json", value_enum)]
        format: OutputFmt,
    },
    /// Check + compile to binary graph (.flux.bin)
    Compile {
        file: String,
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Check + compile + LLVM codegen -> executable
    Build {
        file: String,
        #[arg(short, long)]
        output: Option<String>,
        #[arg(long, default_value = "2")]
        opt_level: u8,
    },
    /// Emit LLVM IR for debugging
    Ir {
        file: String,
    },
}

#[derive(Debug, Clone, ValueEnum)]
enum OutputFmt {
    Json,
    Text,
}

// ---------------------------------------------------------------------------
// Shared result types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct FullResult {
    status: FullStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    ast: Option<flux_ftl::ast::Program>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    parse_errors: Vec<flux_ftl::error::ParseError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    validation_errors: Vec<GenericError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    proof_results: Vec<ProofResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compiled: Option<CompileMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compile_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    feedback: Option<LlmFeedback>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum FullStatus {
    Ok,
    ParseError,
    ValidationFail,
    ProofFail,
}

#[derive(Debug, Serialize)]
struct GenericError {
    error_code: u32,
    node_id: String,
    violation: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<String>,
}

// ---------------------------------------------------------------------------
// Input helper
// ---------------------------------------------------------------------------

fn read_input(file: &str) -> Result<String, String> {
    if file == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| e.to_string())?;
        Ok(s)
    } else {
        std::fs::read_to_string(file).map_err(|e| format!("{}: {}", file, e))
    }
}

// ---------------------------------------------------------------------------
// Check pipeline — shared across all subcommands
// ---------------------------------------------------------------------------

fn run_check(input: &str) -> FullResult {
    let parse_result = parse_ftl(input);

    let ast = match parse_result.status {
        Status::Ok => match parse_result.ast {
            Some(ast) => ast,
            None => {
                return FullResult {
                    status: FullStatus::ParseError,
                    ast: None,
                    parse_errors: parse_result.errors,
                    validation_errors: Vec::new(),
                    proof_results: Vec::new(),
                    compiled: None,
                    compile_error: None,
                    feedback: None,
                };
            }
        },
        Status::Error => {
            let fb = feedback::generate_feedback(&parse_result.errors, &[], &[]);
            return FullResult {
                status: FullStatus::ParseError,
                ast: None,
                parse_errors: parse_result.errors,
                validation_errors: Vec::new(),
                proof_results: Vec::new(),
                compiled: None,
                compile_error: None,
                feedback: Some(fb),
            };
        }
    };

    let mut validation_errors = Vec::new();

    // Phase 1: Structural validation
    let vr = validate(&ast);
    for e in &vr.errors {
        validation_errors.push(GenericError {
            error_code: e.error_code,
            node_id: e.node_id.clone(),
            violation: e.violation.clone(),
            message: e.message.clone(),
            suggestion: e.suggestion.clone(),
        });
    }
    for w in &vr.warnings {
        validation_errors.push(GenericError {
            error_code: w.error_code,
            node_id: w.node_id.clone(),
            violation: w.violation.clone(),
            message: w.message.clone(),
            suggestion: w.suggestion.clone(),
        });
    }

    // Phase 2: Type and effect checks
    for e in check_types_and_effects(&ast) {
        validation_errors.push(GenericError {
            error_code: e.error_code,
            node_id: e.node_id,
            violation: e.violation,
            message: e.message,
            suggestion: e.suggestion,
        });
    }

    // Phase 3: Region checks
    for e in check_regions(&ast) {
        validation_errors.push(GenericError {
            error_code: e.error_code,
            node_id: e.node_id,
            violation: e.violation,
            message: e.message,
            suggestion: e.suggestion,
        });
    }

    let has_fatal = validation_errors
        .iter()
        .any(|e| e.error_code < 2000 || e.error_code >= 3000);

    // Phase 4: Contract proving (only if no fatal validation errors)
    let proof_results = if !has_fatal {
        let config = ProverConfig::default();
        prove_contracts(&ast, &config)
    } else {
        Vec::new()
    };

    let has_disproven = proof_results
        .iter()
        .any(|r| r.status == ProofStatus::Disproven);

    let status = if has_fatal {
        FullStatus::ValidationFail
    } else if has_disproven {
        FullStatus::ProofFail
    } else {
        FullStatus::Ok
    };

    // Phase 5: Compilation (run unless fatal validation errors)
    let (compiled, compile_error) = if !has_fatal {
        match compiler::compile(&ast) {
            Ok(graph) => (Some(CompileMetadata::from(&graph)), None),
            Err(e) => (None, Some(format!("{}", e))),
        }
    } else {
        (None, None)
    };

    // Phase 7: Generate LLM feedback
    let feedback_validation_errors: Vec<FeedbackValidationError> = validation_errors
        .iter()
        .map(|ge| FeedbackValidationError {
            error_code: ge.error_code,
            node_id: ge.node_id.clone(),
            violation: ge.violation.clone(),
            message: ge.message.clone(),
            suggestion: ge.suggestion.clone(),
        })
        .collect();

    let fb = feedback::generate_feedback(&[], &feedback_validation_errors, &proof_results);

    FullResult {
        status,
        ast: Some(ast),
        parse_errors: Vec::new(),
        validation_errors,
        proof_results,
        compiled,
        compile_error,
        feedback: Some(fb),
    }
}

// ---------------------------------------------------------------------------
// Output formatting
// ---------------------------------------------------------------------------

fn print_json(result: &FullResult) -> Result<(), String> {
    let json = serde_json::to_string(result).map_err(|e| format!("JSON serialization: {}", e))?;
    println!("{}", json);
    Ok(())
}

fn print_text(result: &FullResult) {
    let ok = |b: bool| if b { "[PASS]" } else { "[FAIL]" };

    let parse_ok = result.status != FullStatus::ParseError;
    println!("Parse    {}", ok(parse_ok));

    if !parse_ok {
        for e in &result.parse_errors {
            println!("  - {}", e.message);
        }
        return;
    }

    let validate_ok =
        result.status != FullStatus::ValidationFail && result.validation_errors.is_empty();
    println!("Validate {}", ok(validate_ok));
    for e in &result.validation_errors {
        println!("  - [{}] {}: {}", e.error_code, e.node_id, e.message);
    }

    let prove_ok = result.status != FullStatus::ProofFail;
    println!("Prove    {}", ok(prove_ok));
    for r in &result.proof_results {
        if r.status == ProofStatus::Disproven {
            println!("  - {} DISPROVEN", r.contract_id);
        }
    }

    let compile_ok = result.compiled.is_some();
    println!("Compile  {}", ok(compile_ok));
    if let Some(err) = &result.compile_error {
        println!("  - {}", err);
    }
}

// ---------------------------------------------------------------------------
// Subcommand implementations
// ---------------------------------------------------------------------------

fn cmd_check(file: &str, format: &OutputFmt) -> ExitCode {
    let input = match read_input(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };

    let result = run_check(&input);
    let exit = match result.status {
        FullStatus::Ok => ExitCode::SUCCESS,
        FullStatus::ValidationFail | FullStatus::ProofFail => ExitCode::from(1),
        FullStatus::ParseError => ExitCode::from(1),
    };

    match format {
        OutputFmt::Json => {
            if let Err(e) = print_json(&result) {
                eprintln!("error: {}", e);
                return ExitCode::from(2);
            }
        }
        OutputFmt::Text => print_text(&result),
    }

    exit
}

fn cmd_compile(file: &str, output: Option<&str>) -> ExitCode {
    let input = match read_input(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };

    let result = run_check(&input);
    if result.status != FullStatus::Ok {
        if let Err(e) = print_json(&result) {
            eprintln!("error: {}", e);
        }
        return ExitCode::from(1);
    }

    let ast = match &result.ast {
        Some(a) => a,
        None => {
            eprintln!("error: no AST available after check");
            return ExitCode::from(2);
        }
    };

    let graph = match compiler::compile(ast) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: compile: {}", e);
            return ExitCode::from(2);
        }
    };

    let out_path = match output {
        Some(p) => p.to_string(),
        None => {
            let base = if file == "-" {
                "out".to_string()
            } else {
                file.trim_end_matches(".ftl").to_string()
            };
            format!("{}.flux.bin", base)
        }
    };

    if let Err(e) = compiler::write_binary(&graph, std::path::Path::new(&out_path)) {
        eprintln!("error: write binary: {}", e);
        return ExitCode::from(2);
    }

    println!("{}", out_path);
    ExitCode::SUCCESS
}

fn cmd_build(file: &str, output: Option<&str>, opt_level: u8) -> ExitCode {
    let input = match read_input(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };

    let result = run_check(&input);
    if result.status != FullStatus::Ok {
        if let Err(e) = print_json(&result) {
            eprintln!("error: {}", e);
        }
        return ExitCode::from(1);
    }

    let ast = match &result.ast {
        Some(a) => a,
        None => {
            eprintln!("error: no AST available after check");
            return ExitCode::from(2);
        }
    };

    // Apply graph-level optimizations before codegen
    let opt_config = OptimizationConfig {
        llvm_opt_level: opt_level,
        enable_graph_opts: opt_level > 0,
        strip_dead_nodes: opt_level > 0,
        fold_constants: opt_level > 0,
    };
    let opt_result = optimizer::optimize_graph(ast, &opt_config);
    let optimized_ast = &opt_result.optimized_program;

    if opt_result.stats.constants_folded > 0 || opt_result.stats.dead_nodes_removed > 0 {
        eprintln!(
            "optimizer: folded {} constants, removed {} dead nodes, removed {} identities ({} -> {} nodes)",
            opt_result.stats.constants_folded,
            opt_result.stats.dead_nodes_removed,
            opt_result.stats.identities_removed,
            opt_result.stats.nodes_before,
            opt_result.stats.nodes_after,
        );
    }

    let opt = match opt_level {
        0 => OptLevel::None,
        1 => OptLevel::Less,
        3 => OptLevel::Aggressive,
        _ => OptLevel::Default,
    };

    let config = CodegenConfig {
        opt_level: opt,
        output_format: OutputFormat::ObjectFile,
        ..CodegenConfig::default()
    };

    let cg_result = match codegen::codegen(optimized_ast, &config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: codegen: {}", e);
            return ExitCode::from(2);
        }
    };

    // Write object file to a temp path
    let tmp_dir = std::env::temp_dir();
    let obj_path = tmp_dir.join("flux_build.o");
    if let Err(e) = std::fs::write(&obj_path, &cg_result.output_bytes) {
        eprintln!("error: write object file: {}", e);
        return ExitCode::from(2);
    }

    let out_path = match output {
        Some(p) => p.to_string(),
        None => {
            if file == "-" {
                "a.out".to_string()
            } else {
                file.trim_end_matches(".ftl").to_string()
            }
        }
    };

    // Link with cc
    let link_status = std::process::Command::new("cc")
        .arg(&obj_path)
        .arg("-o")
        .arg(&out_path)
        .arg("-lc")
        .status();

    // Clean up temp file
    let _ = std::fs::remove_file(&obj_path);

    match link_status {
        Ok(s) if s.success() => {
            println!("{}", out_path);
            ExitCode::SUCCESS
        }
        Ok(s) => {
            eprintln!(
                "error: linker failed with exit code {}",
                s.code().unwrap_or(-1)
            );
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("error: failed to run linker (cc): {}", e);
            ExitCode::from(2)
        }
    }
}

fn cmd_ir(file: &str) -> ExitCode {
    let input = match read_input(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };

    let result = run_check(&input);
    if result.status != FullStatus::Ok {
        if let Err(e) = print_json(&result) {
            eprintln!("error: {}", e);
        }
        return ExitCode::from(1);
    }

    let ast = match &result.ast {
        Some(a) => a,
        None => {
            eprintln!("error: no AST available after check");
            return ExitCode::from(2);
        }
    };

    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };

    let cg_result = match codegen::codegen(ast, &config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: codegen: {}", e);
            return ExitCode::from(2);
        }
    };

    print!("{}", cg_result.llvm_ir);
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    use clap::Parser;
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Check { ref file, ref format }) => cmd_check(file, format),
        Some(Commands::Compile { ref file, ref output }) => {
            cmd_compile(file, output.as_deref())
        }
        Some(Commands::Build {
            ref file,
            ref output,
            opt_level,
        }) => cmd_build(file, output.as_deref(), opt_level),
        Some(Commands::Ir { ref file }) => cmd_ir(file),
        None => {
            // Backward compatible: stdin -> JSON
            cmd_check("-", &OutputFmt::Json)
        }
    }
}
