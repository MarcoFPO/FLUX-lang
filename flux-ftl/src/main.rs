use std::io::Read;
use std::process::ExitCode;

use clap::ValueEnum;
use serde::Serialize;

use flux_ftl::codegen::{self, CodegenConfig, FluxTarget, OptLevel, OutputFormat};
use flux_ftl::compiler::{self, CompileMetadata};
use flux_ftl::error::Status;
use flux_ftl::evolution::{self, EvolutionConfig, GraphPool};
use flux_ftl::feedback::{self, LlmFeedback, ValidationError as FeedbackValidationError};
use flux_ftl::llm::{GenerateRequest, GenerationLoop, LlmConfig, LlmProvider, RequirementType};
use flux_ftl::optimizer::{self, OptimizationConfig};
use flux_ftl::parser::parse_ftl;
use flux_ftl::prover::{prove_contracts, BmcConfig, ProofResult, ProofStatus, ProverConfig};
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
        /// Enable Bounded Model Checking as Z3 fallback
        #[arg(long)]
        bmc: bool,
        /// BMC unfolding depth (default: 10)
        #[arg(long, default_value = "10")]
        bmc_depth: u32,
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
        /// Target architecture: x86_64, aarch64, riscv64, wasm32, host
        #[arg(long, default_value = "host")]
        target: String,
        /// Enable Bounded Model Checking as Z3 fallback
        #[arg(long)]
        bmc: bool,
        /// BMC unfolding depth (default: 10)
        #[arg(long, default_value = "10")]
        bmc_depth: u32,
    },
    /// Emit LLVM IR for debugging
    Ir {
        file: String,
        /// Target architecture: x86_64, aarch64, riscv64, wasm32, host
        #[arg(long, default_value = "host")]
        target: String,
    },
    /// Generate FTL from natural language using LLM
    Generate {
        /// Natural language requirement
        requirement: String,

        /// Requirement type: translate, optimize, invent, discover
        #[arg(long, default_value = "translate")]
        requirement_type: String,

        /// LLM provider: anthropic, openai
        #[arg(long, default_value = "anthropic")]
        provider: String,

        /// Model name (default depends on provider)
        #[arg(long)]
        model: Option<String>,

        /// Max repair iterations
        #[arg(long, default_value = "5")]
        max_iterations: u32,

        /// Output file for generated FTL (stdout if not set)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Evolve graph variants using a genetic algorithm
    Evolve {
        /// Input FTL file to use as base program
        file: String,
        /// Number of generations to run
        #[arg(long, default_value = "50")]
        generations: u32,
        /// Population size
        #[arg(long, default_value = "30")]
        population: usize,
        /// Mutation rate (0.0 - 1.0)
        #[arg(long, default_value = "0.3")]
        mutation_rate: f64,
        /// Crossover rate (0.0 - 1.0)
        #[arg(long, default_value = "0.5")]
        crossover_rate: f64,
        /// Random seed for reproducibility
        #[arg(long)]
        seed: Option<u64>,
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
    run_check_with_bmc(input, None)
}

fn run_check_with_bmc(input: &str, bmc_config: Option<BmcConfig>) -> FullResult {
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
        let config = ProverConfig {
            bmc_config,
            ..ProverConfig::default()
        };
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

fn cmd_check(file: &str, format: &OutputFmt, bmc: bool, bmc_depth: u32) -> ExitCode {
    let input = match read_input(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };

    let bmc_config = if bmc {
        Some(BmcConfig {
            max_depth: bmc_depth,
            ..BmcConfig::default()
        })
    } else {
        None
    };

    let result = run_check_with_bmc(&input, bmc_config);
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

fn cmd_build(file: &str, output: Option<&str>, opt_level: u8, target_str: &str, bmc: bool, bmc_depth: u32) -> ExitCode {
    let flux_target = match FluxTarget::parse(target_str) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };
    let input = match read_input(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };

    let bmc_config = if bmc {
        Some(BmcConfig {
            max_depth: bmc_depth,
            ..BmcConfig::default()
        })
    } else {
        None
    };

    let result = run_check_with_bmc(&input, bmc_config);
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
        target_triple: flux_target.resolved_triple(),
        target: flux_target,
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

fn cmd_ir(file: &str, target_str: &str) -> ExitCode {
    let flux_target = match FluxTarget::parse(target_str) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };
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
        target_triple: flux_target.resolved_triple(),
        target: flux_target,
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
// Generate subcommand
// ---------------------------------------------------------------------------

fn cmd_generate(
    requirement: &str,
    requirement_type: &str,
    provider: &str,
    model: Option<&str>,
    max_iterations: u32,
    output: Option<&str>,
) -> ExitCode {
    // Parse requirement type
    let req_type = match RequirementType::from_str_loose(requirement_type) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };

    // Parse provider
    let llm_provider = match LlmProvider::from_str_loose(provider) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };

    // Build config from environment
    let mut config = match LlmConfig::from_env(llm_provider, model.map(String::from)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(1);
        }
    };
    config.max_iterations = max_iterations;

    // Create generation loop
    let gen_loop = match GenerationLoop::new(config) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };

    let request = GenerateRequest {
        requirement: requirement.to_string(),
        requirement_type: req_type,
        context: None,
        examples: Vec::new(),
    };

    // Run async generation loop
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: failed to create async runtime: {}", e);
            return ExitCode::from(2);
        }
    };

    let result = match rt.block_on(gen_loop.generate(&request)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: generation failed: {}", e);
            return ExitCode::from(1);
        }
    };

    // Serialize result as JSON
    let json = match serde_json::to_string_pretty(&result) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("error: JSON serialization: {}", e);
            return ExitCode::from(2);
        }
    };

    // Output to file or stdout
    if let Some(path) = output {
        if let Err(e) = std::fs::write(path, &json) {
            eprintln!("error: write output file: {}", e);
            return ExitCode::from(2);
        }
        eprintln!("Output written to {}", path);
    } else {
        println!("{}", json);
    }

    let exit_code = match result.final_status {
        flux_ftl::llm::GenerationStatus::Success
        | flux_ftl::llm::GenerationStatus::PartialSuccess => 0,
        _ => 1,
    };

    ExitCode::from(exit_code)
}

// ---------------------------------------------------------------------------
// Evolve subcommand
// ---------------------------------------------------------------------------

fn cmd_evolve(
    file: &str,
    generations: u32,
    population: usize,
    mutation_rate: f64,
    crossover_rate: f64,
    seed: Option<u64>,
) -> ExitCode {
    let input = match read_input(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };

    let result = run_check(&input);
    let ast = match &result.ast {
        Some(a) => a,
        None => {
            eprintln!("error: failed to parse input FTL");
            if let Err(e) = print_json(&result) {
                eprintln!("error: {}", e);
            }
            return ExitCode::from(1);
        }
    };

    let config = EvolutionConfig {
        population_size: population,
        mutation_rate,
        crossover_rate,
        max_generations: generations,
        seed,
        ..Default::default()
    };

    eprintln!(
        "Evolution: population={}, generations={}, mutation_rate={}, crossover_rate={}",
        population, generations, mutation_rate, crossover_rate,
    );
    if let Some(s) = seed {
        eprintln!("  seed: {}", s);
    }

    let mut pool = GraphPool::new(config);
    pool.seed_population(ast, population);

    let evo_result = pool.run(generations);

    eprintln!("\nEvolution complete:");
    eprintln!("  generations run: {}", evo_result.generations_run);
    eprintln!(
        "  best fitness:    {:.4}",
        evo_result.population_stats.best_fitness
    );
    eprintln!(
        "  avg fitness:     {:.4}",
        evo_result.population_stats.avg_fitness
    );
    eprintln!(
        "  proven count:    {}",
        evo_result.population_stats.proven_count
    );
    eprintln!(
        "  incubated:       {}",
        evo_result.population_stats.incubated_count
    );
    eprintln!(
        "  best node count: {}",
        evolution::count_nodes(&evo_result.best.program)
    );
    eprintln!(
        "  best depth:      {}",
        evolution::calculate_depth(&evo_result.best.program)
    );

    // Output the best program as JSON
    match serde_json::to_string_pretty(&evo_result.best.program) {
        Ok(json) => {
            println!("{}", json);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: JSON serialization: {}", e);
            ExitCode::from(2)
        }
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    use clap::Parser;
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Check { ref file, ref format, bmc, bmc_depth }) => cmd_check(file, format, bmc, bmc_depth),
        Some(Commands::Compile { ref file, ref output }) => {
            cmd_compile(file, output.as_deref())
        }
        Some(Commands::Build {
            ref file,
            ref output,
            opt_level,
            ref target,
            bmc,
            bmc_depth,
        }) => cmd_build(file, output.as_deref(), opt_level, target, bmc, bmc_depth),
        Some(Commands::Ir { ref file, ref target }) => cmd_ir(file, target),
        Some(Commands::Generate {
            ref requirement,
            ref requirement_type,
            ref provider,
            ref model,
            max_iterations,
            ref output,
        }) => cmd_generate(
            requirement,
            requirement_type,
            provider,
            model.as_deref(),
            max_iterations,
            output.as_deref(),
        ),
        Some(Commands::Evolve {
            ref file,
            generations,
            population,
            mutation_rate,
            crossover_rate,
            seed,
        }) => cmd_evolve(file, generations, population, mutation_rate, crossover_rate, seed),
        None => {
            // Backward compatible: stdin -> JSON
            cmd_check("-", &OutputFmt::Json, false, 10)
        }
    }
}
