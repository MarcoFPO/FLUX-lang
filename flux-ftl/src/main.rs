use std::io::Read;
use std::process::ExitCode;

use flux_ftl::codegen::{self, CodegenConfig, FluxTarget, OptLevel, OutputFormat};
use flux_ftl::compiler;
use flux_ftl::evolution::{self, EvolutionConfig, GraphPool};
use flux_ftl::llm::{GenerateRequest, GenerationLoop, LlmConfig, LlmProvider, RequirementType};
use flux_ftl::optimizer::{self, OptimizationConfig};
use flux_ftl::pipeline::{self, FullResult, FullStatus};
use flux_ftl::prover::BmcConfig;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(clap::Parser)]
#[command(name = "flux-ftl")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    Check {
        #[arg(default_value = "-")]
        file: String,
        #[arg(long)]
        bmc: bool,
        #[arg(long, default_value = "10")]
        bmc_depth: u32,
    },
    Compile {
        file: String,
        #[arg(short, long)]
        output: Option<String>,
    },
    Build {
        file: String,
        #[arg(short, long)]
        output: Option<String>,
        #[arg(long, default_value = "2")]
        opt_level: u8,
        #[arg(long, default_value = "host")]
        target: String,
        #[arg(long)]
        bmc: bool,
        #[arg(long, default_value = "10")]
        bmc_depth: u32,
        /// Emit DWARF debug information
        #[arg(long)]
        debug_info: bool,
        /// Enable Link-Time Optimization
        #[arg(long)]
        lto: bool,
    },
    Ir {
        file: String,
        #[arg(long, default_value = "host")]
        target: String,
    },
    Generate {
        requirement: String,
        #[arg(long, default_value = "translate")]
        requirement_type: String,
        #[arg(long, default_value = "anthropic")]
        provider: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value = "5")]
        max_iterations: u32,
        #[arg(short, long)]
        output: Option<String>,
    },
    Evolve {
        file: String,
        #[arg(long, default_value = "50")]
        generations: u32,
        #[arg(long, default_value = "30")]
        population: usize,
        #[arg(long, default_value = "0.3")]
        mutation_rate: f64,
        #[arg(long, default_value = "0.5")]
        crossover_rate: f64,
        #[arg(long)]
        seed: Option<u64>,
    },
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
// Output formatting
// ---------------------------------------------------------------------------

fn print_json(result: &FullResult) -> Result<(), String> {
    let json = pipeline::result_to_json(result)?;
    println!("{}", json);
    Ok(())
}

// ---------------------------------------------------------------------------
// Subcommand implementations
// ---------------------------------------------------------------------------

/// Parse input and resolve imports if the input comes from a file.
/// Returns the merged Program or prints errors and returns None.
fn parse_and_resolve(input: &str, file: &str) -> Option<flux_ftl::ast::Program> {
    let parse_result = flux_ftl::parser::parse_ftl(input);
    let ast = match parse_result.status {
        flux_ftl::error::Status::Ok => parse_result.ast?,
        flux_ftl::error::Status::Error => return None,
    };

    if file != "-" && !ast.imports.is_empty() {
        let path = std::path::Path::new(file);
        match pipeline::resolve_imports(&ast, path) {
            Ok(merged) => Some(merged),
            Err(errs) => {
                for e in &errs {
                    eprintln!("error: {}", e);
                }
                None
            }
        }
    } else {
        Some(ast)
    }
}

fn cmd_check(file: &str, bmc: bool, bmc_depth: u32) -> ExitCode {
    let input = match read_input(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }
    };

    // Try to parse and resolve imports for file-based input
    if file != "-"
        && let Some(merged) = parse_and_resolve(&input, file)
    {
        let bmc_config = if bmc {
            Some(BmcConfig {
                max_depth: bmc_depth,
                ..BmcConfig::default()
            })
        } else {
            None
        };

        let result = pipeline::run_check_program_with_bmc(merged, bmc_config);
        let exit = match result.status {
            FullStatus::Ok => ExitCode::SUCCESS,
            FullStatus::ValidationFail | FullStatus::ProofFail => ExitCode::from(1),
            FullStatus::ParseError => ExitCode::from(1),
        };

        if let Err(e) = print_json(&result) {
            eprintln!("error: {}", e);
            return ExitCode::from(2);
        }

        return exit;
    }

    let bmc_config = if bmc {
        Some(BmcConfig {
            max_depth: bmc_depth,
            ..BmcConfig::default()
        })
    } else {
        None
    };

    let result = pipeline::run_check_with_bmc(&input, bmc_config);
    let exit = match result.status {
        FullStatus::Ok => ExitCode::SUCCESS,
        FullStatus::ValidationFail | FullStatus::ProofFail => ExitCode::from(1),
        FullStatus::ParseError => ExitCode::from(1),
    };

    if let Err(e) = print_json(&result) {
        eprintln!("error: {}", e);
        return ExitCode::from(2);
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

    let result = pipeline::run_check(&input);
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

#[allow(clippy::too_many_arguments)]
fn cmd_build(file: &str, output: Option<&str>, opt_level: u8, target_str: &str, bmc: bool, bmc_depth: u32, debug_info: bool, lto: bool) -> ExitCode {
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

    // Try import resolution for file-based input
    let result = if file != "-" {
        if let Some(merged) = parse_and_resolve(&input, file) {
            pipeline::run_check_program_with_bmc(merged, bmc_config)
        } else {
            pipeline::run_check_with_bmc(&input, bmc_config)
        }
    } else {
        pipeline::run_check_with_bmc(&input, bmc_config)
    };

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
        emit_debug_info: debug_info,
        lto,
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

    let result = pipeline::run_check(&input);
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

    let result = pipeline::run_check(&input);
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
        Some(Commands::Check { ref file, bmc, bmc_depth }) => cmd_check(file, bmc, bmc_depth),
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
            debug_info,
            lto,
        }) => cmd_build(file, output.as_deref(), opt_level, target, bmc, bmc_depth, debug_info, lto),
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
            cmd_check("-", false, 10)
        }
    }
}
