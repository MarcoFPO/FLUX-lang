// ---------------------------------------------------------------------------
// flux-mcp — MCP (Model Context Protocol) server for FLUX FTL
// ---------------------------------------------------------------------------
//
// Exposes the FLUX FTL compiler pipeline as tools for LLMs via JSON-RPC 2.0
// over stdio (stdin/stdout). Implements the MCP protocol without external
// MCP crates — only serde_json and standard I/O.
//
// Tools:
//   flux_check    — Parse, validate, type-check, region-check, prove contracts
//   flux_compile  — Compile to content-addressed binary graph
//   flux_build    — Full pipeline: check + compile + LLVM codegen + link
//   flux_ir       — Generate LLVM IR from FTL source
//   flux_evolve   — Evolve graph variants using a genetic algorithm
//   flux_prove    — Formally prove V-Node contracts using Z3
//   flux_generate — Generate FTL programs from natural language via LLM
// ---------------------------------------------------------------------------

use std::io::{BufRead, BufReader, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use flux_ftl::codegen::{self, CodegenConfig, FluxTarget, OptLevel, OutputFormat};
use flux_ftl::compiler::{self, CompileMetadata};
use flux_ftl::evolution::{self, EvolutionConfig, GraphPool};
use flux_ftl::llm::{GenerateRequest, GenerationLoop, LlmConfig, LlmProvider, RequirementType};
use flux_ftl::optimizer::{self, OptimizationConfig};
use flux_ftl::pipeline::{self, FullStatus};
use flux_ftl::prover::{prove_contracts, BmcConfig, ProverConfig};

// ---------------------------------------------------------------------------
// JSON-RPC types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

// ---------------------------------------------------------------------------
// MCP Tool definition
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Tool {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn send_response(stdout: &std::io::Stdout, response: &JsonRpcResponse) {
    let json = serde_json::to_string(response).expect("failed to serialize response");
    let mut out = stdout.lock();
    let _ = writeln!(out, "{}", json);
    let _ = out.flush();
}

fn send_result(stdout: &std::io::Stdout, id: Value, result: Value) {
    send_response(stdout, &JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(result),
        error: None,
    });
}

fn send_error(stdout: &std::io::Stdout, id: Value, code: i64, message: &str) {
    send_response(stdout, &JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
        }),
    });
}

fn send_tool_result(stdout: &std::io::Stdout, id: Value, text: &str) {
    let content = serde_json::json!({
        "content": [{"type": "text", "text": text}]
    });
    send_result(stdout, id, content);
}

fn send_tool_error(stdout: &std::io::Stdout, id: Value, text: &str) {
    let content = serde_json::json!({
        "content": [{"type": "text", "text": text}],
        "isError": true
    });
    send_result(stdout, id, content);
}

// ---------------------------------------------------------------------------
// Handler: initialize
// ---------------------------------------------------------------------------

fn handle_initialize(stdout: &std::io::Stdout, id: Value) {
    let result = serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "flux-ftl",
            "version": "1.0.0"
        }
    });
    send_result(stdout, id, result);
}

// ---------------------------------------------------------------------------
// Handler: tools/list
// ---------------------------------------------------------------------------

fn build_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "flux_check".to_string(),
            description: "Parse, validate, type-check, region-check, and prove contracts for FTL source code. Returns structured JSON with parse_errors, validation_errors, proof_results, and LLM feedback with repair suggestions.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "ftl_source": { "type": "string", "description": "FTL source code to check" },
                    "bmc": { "type": "boolean", "description": "Enable Bounded Model Checking as Z3 fallback", "default": false },
                    "bmc_depth": { "type": "integer", "description": "BMC unrolling depth", "default": 10 }
                },
                "required": ["ftl_source"]
            }),
        },
        Tool {
            name: "flux_compile".to_string(),
            description: "Compile validated FTL to a content-addressed binary graph with BLAKE3 hashes. Returns compilation metadata including entry_hash, total_nodes, and node details.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "ftl_source": { "type": "string", "description": "FTL source code to compile" }
                },
                "required": ["ftl_source"]
            }),
        },
        Tool {
            name: "flux_build".to_string(),
            description: "Full pipeline: check + compile + LLVM codegen + link to executable binary. Returns the path to the generated executable.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "ftl_source": { "type": "string" },
                    "output_path": { "type": "string", "description": "Path for output executable" },
                    "target": { "type": "string", "enum": ["host", "x86_64", "aarch64", "riscv64", "wasm32"], "default": "host" },
                    "opt_level": { "type": "integer", "enum": [0, 1, 2, 3], "default": 2 },
                    "debug_info": { "type": "boolean", "description": "Emit DWARF debug information", "default": false },
                    "lto": { "type": "boolean", "description": "Enable Link-Time Optimization", "default": false }
                },
                "required": ["ftl_source", "output_path"]
            }),
        },
        Tool {
            name: "flux_ir".to_string(),
            description: "Generate LLVM IR from FTL source. Useful for debugging and inspecting generated code.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "ftl_source": { "type": "string" },
                    "target": { "type": "string", "enum": ["host", "x86_64", "aarch64", "riscv64", "wasm32"], "default": "host" }
                },
                "required": ["ftl_source"]
            }),
        },
        Tool {
            name: "flux_evolve".to_string(),
            description: "Evolve graph variants using a genetic algorithm. Takes a base FTL program and produces optimized variants through mutation, crossover, and selection.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "ftl_source": { "type": "string" },
                    "generations": { "type": "integer", "default": 50 },
                    "population": { "type": "integer", "default": 30 },
                    "mutation_rate": { "type": "number", "default": 0.3 },
                    "seed": { "type": "integer", "description": "Random seed for reproducibility" }
                },
                "required": ["ftl_source"]
            }),
        },
        Tool {
            name: "flux_prove".to_string(),
            description: "Formally prove V-Node contracts using Z3 SMT solver with optional BMC fallback. Returns per-contract proof status (Proven/Disproven/Unknown/BmcProven/Assumed) with counterexamples for disproven contracts.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "ftl_source": { "type": "string" },
                    "bmc": { "type": "boolean", "default": false },
                    "bmc_depth": { "type": "integer", "default": 10 }
                },
                "required": ["ftl_source"]
            }),
        },
        Tool {
            name: "flux_generate".to_string(),
            description: "Generate FTL programs from natural language requirements using an LLM. Iteratively generates, checks, and repairs FTL code until it passes all checks or max_iterations is reached. Returns a GenerationResult with the generated program, iterations, and final status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "requirement": { "type": "string", "description": "Natural language description of the desired FTL program" },
                    "requirement_type": { "type": "string", "description": "Type of requirement: translate, explain, optimize, refactor", "default": "translate" },
                    "provider": { "type": "string", "description": "LLM provider: anthropic or openai", "default": "anthropic" },
                    "model": { "type": "string", "description": "Model name override (optional, uses provider default if omitted)" },
                    "max_iterations": { "type": "integer", "description": "Maximum number of generate-check-repair iterations", "default": 5 }
                },
                "required": ["requirement"]
            }),
        },
    ]
}

fn handle_tools_list(stdout: &std::io::Stdout, id: Value) {
    let tools = build_tools();
    let result = serde_json::json!({ "tools": tools });
    send_result(stdout, id, result);
}

// ---------------------------------------------------------------------------
// Handler: tools/call
// ---------------------------------------------------------------------------

fn handle_tools_call(stdout: &std::io::Stdout, id: Value, params: Option<Value>) {
    let params = match params {
        Some(p) => p,
        None => {
            send_error(stdout, id, -32602, "Missing params for tools/call");
            return;
        }
    };

    let tool_name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => {
            send_error(stdout, id, -32602, "Missing 'name' in tools/call params");
            return;
        }
    };

    let args = params.get("arguments").cloned().unwrap_or(Value::Object(serde_json::Map::new()));

    match tool_name.as_str() {
        "flux_check" => handle_flux_check(stdout, id, &args),
        "flux_compile" => handle_flux_compile(stdout, id, &args),
        "flux_build" => handle_flux_build(stdout, id, &args),
        "flux_ir" => handle_flux_ir(stdout, id, &args),
        "flux_evolve" => handle_flux_evolve(stdout, id, &args),
        "flux_prove" => handle_flux_prove(stdout, id, &args),
        "flux_generate" => handle_flux_generate(stdout, id, &args),
        _ => send_tool_error(stdout, id, &format!("Unknown tool: {}", tool_name)),
    }
}

// ---------------------------------------------------------------------------
// Tool: flux_check
// ---------------------------------------------------------------------------

fn handle_flux_check(stdout: &std::io::Stdout, id: Value, args: &Value) {
    let ftl_source = match args.get("ftl_source").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => {
            send_tool_error(stdout, id, "Missing required argument: ftl_source");
            return;
        }
    };

    let bmc = args.get("bmc").and_then(|v| v.as_bool()).unwrap_or(false);
    let bmc_depth = args.get("bmc_depth").and_then(|v| v.as_u64()).unwrap_or(10) as u32;

    let bmc_config = if bmc {
        Some(BmcConfig {
            max_depth: bmc_depth,
            ..BmcConfig::default()
        })
    } else {
        None
    };

    let result = pipeline::run_check_with_bmc(ftl_source, bmc_config);
    let json = match pipeline::result_to_json(&result) {
        Ok(j) => j,
        Err(e) => {
            send_tool_error(stdout, id, &format!("Serialization error: {}", e));
            return;
        }
    };

    send_tool_result(stdout, id, &json);
}

// ---------------------------------------------------------------------------
// Tool: flux_compile
// ---------------------------------------------------------------------------

fn handle_flux_compile(stdout: &std::io::Stdout, id: Value, args: &Value) {
    let ftl_source = match args.get("ftl_source").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => {
            send_tool_error(stdout, id, "Missing required argument: ftl_source");
            return;
        }
    };

    let result = pipeline::run_check(ftl_source);
    if result.status != FullStatus::Ok {
        let json = pipeline::result_to_json(&result).unwrap_or_else(|e| e);
        send_tool_error(stdout, id, &format!("Check failed:\n{}", json));
        return;
    }

    let ast = match &result.ast {
        Some(a) => a,
        None => {
            send_tool_error(stdout, id, "No AST available after check");
            return;
        }
    };

    match compiler::compile(ast) {
        Ok(graph) => {
            let metadata = CompileMetadata::from(&graph);
            let json = serde_json::to_string(&metadata)
                .unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e));
            send_tool_result(stdout, id, &json);
        }
        Err(e) => {
            send_tool_error(stdout, id, &format!("Compilation error: {}", e));
        }
    }
}

// ---------------------------------------------------------------------------
// Tool: flux_build
// ---------------------------------------------------------------------------

fn handle_flux_build(stdout: &std::io::Stdout, id: Value, args: &Value) {
    let ftl_source = match args.get("ftl_source").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => {
            send_tool_error(stdout, id, "Missing required argument: ftl_source");
            return;
        }
    };

    let output_path = match args.get("output_path").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            send_tool_error(stdout, id, "Missing required argument: output_path");
            return;
        }
    };

    let target_str = args.get("target").and_then(|v| v.as_str()).unwrap_or("host");
    let opt_level_num = args.get("opt_level").and_then(|v| v.as_u64()).unwrap_or(2) as u8;
    let debug_info = args.get("debug_info").and_then(|v| v.as_bool()).unwrap_or(false);
    let lto = args.get("lto").and_then(|v| v.as_bool()).unwrap_or(false);

    let flux_target = match FluxTarget::parse(target_str) {
        Ok(t) => t,
        Err(e) => {
            send_tool_error(stdout, id, &format!("Invalid target: {}", e));
            return;
        }
    };

    let result = pipeline::run_check(ftl_source);
    if result.status != FullStatus::Ok {
        let json = pipeline::result_to_json(&result).unwrap_or_else(|e| e);
        send_tool_error(stdout, id, &format!("Check failed:\n{}", json));
        return;
    }

    let ast = match &result.ast {
        Some(a) => a,
        None => {
            send_tool_error(stdout, id, "No AST available after check");
            return;
        }
    };

    // Apply graph-level optimizations before codegen
    let opt_config = OptimizationConfig {
        llvm_opt_level: opt_level_num,
        enable_graph_opts: opt_level_num > 0,
        strip_dead_nodes: opt_level_num > 0,
        fold_constants: opt_level_num > 0,
    };
    let opt_result = optimizer::optimize_graph(ast, &opt_config);
    let optimized_ast = &opt_result.optimized_program;

    let opt = match opt_level_num {
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
            send_tool_error(stdout, id, &format!("Codegen error: {}", e));
            return;
        }
    };

    // Write object file to a temp path
    let tmp_dir = std::env::temp_dir();
    let obj_path = tmp_dir.join("flux_mcp_build.o");
    if let Err(e) = std::fs::write(&obj_path, &cg_result.output_bytes) {
        send_tool_error(stdout, id, &format!("Failed to write object file: {}", e));
        return;
    }

    // Link with cc
    let link_status = std::process::Command::new("cc")
        .arg(&obj_path)
        .arg("-o")
        .arg(&output_path)
        .arg("-lc")
        .status();

    // Clean up temp file
    let _ = std::fs::remove_file(&obj_path);

    match link_status {
        Ok(s) if s.success() => {
            let result_json = serde_json::json!({
                "status": "OK",
                "output_path": output_path,
                "opt_level": opt_level_num,
                "target": target_str,
            });
            send_tool_result(stdout, id, &result_json.to_string());
        }
        Ok(s) => {
            send_tool_error(stdout, id, &format!(
                "Linker failed with exit code {}",
                s.code().unwrap_or(-1)
            ));
        }
        Err(e) => {
            send_tool_error(stdout, id, &format!("Failed to run linker (cc): {}", e));
        }
    }
}

// ---------------------------------------------------------------------------
// Tool: flux_ir
// ---------------------------------------------------------------------------

fn handle_flux_ir(stdout: &std::io::Stdout, id: Value, args: &Value) {
    let ftl_source = match args.get("ftl_source").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => {
            send_tool_error(stdout, id, "Missing required argument: ftl_source");
            return;
        }
    };

    let target_str = args.get("target").and_then(|v| v.as_str()).unwrap_or("host");

    let flux_target = match FluxTarget::parse(target_str) {
        Ok(t) => t,
        Err(e) => {
            send_tool_error(stdout, id, &format!("Invalid target: {}", e));
            return;
        }
    };

    let result = pipeline::run_check(ftl_source);
    if result.status != FullStatus::Ok {
        let json = pipeline::result_to_json(&result).unwrap_or_else(|e| e);
        send_tool_error(stdout, id, &format!("Check failed:\n{}", json));
        return;
    }

    let ast = match &result.ast {
        Some(a) => a,
        None => {
            send_tool_error(stdout, id, "No AST available after check");
            return;
        }
    };

    let config = CodegenConfig {
        output_format: OutputFormat::LlvmIr,
        target_triple: flux_target.resolved_triple(),
        target: flux_target,
        ..CodegenConfig::default()
    };

    match codegen::codegen(ast, &config) {
        Ok(cg_result) => {
            send_tool_result(stdout, id, &cg_result.llvm_ir);
        }
        Err(e) => {
            send_tool_error(stdout, id, &format!("Codegen error: {}", e));
        }
    }
}

// ---------------------------------------------------------------------------
// Tool: flux_evolve
// ---------------------------------------------------------------------------

fn handle_flux_evolve(stdout: &std::io::Stdout, id: Value, args: &Value) {
    let ftl_source = match args.get("ftl_source").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => {
            send_tool_error(stdout, id, "Missing required argument: ftl_source");
            return;
        }
    };

    let generations = args.get("generations").and_then(|v| v.as_u64()).unwrap_or(50) as u32;
    let population = args.get("population").and_then(|v| v.as_u64()).unwrap_or(30) as usize;
    let mutation_rate = args.get("mutation_rate").and_then(|v| v.as_f64()).unwrap_or(0.3);
    let seed = args.get("seed").and_then(|v| v.as_u64());

    let result = pipeline::run_check(ftl_source);
    let ast = match &result.ast {
        Some(a) => a,
        None => {
            let json = pipeline::result_to_json(&result).unwrap_or_else(|e| e);
            send_tool_error(stdout, id, &format!("Check failed:\n{}", json));
            return;
        }
    };

    let config = EvolutionConfig {
        population_size: population,
        mutation_rate,
        max_generations: generations,
        seed,
        ..Default::default()
    };

    let mut pool = GraphPool::new(config);
    pool.seed_population(ast, population);
    let evo_result = pool.run(generations);

    let result_json = serde_json::json!({
        "generations_run": evo_result.generations_run,
        "best_fitness": evo_result.population_stats.best_fitness,
        "avg_fitness": evo_result.population_stats.avg_fitness,
        "proven_count": evo_result.population_stats.proven_count,
        "incubated_count": evo_result.population_stats.incubated_count,
        "best_node_count": evolution::count_nodes(&evo_result.best.program),
        "best_depth": evolution::calculate_depth(&evo_result.best.program),
        "best_program": evo_result.best.program,
    });

    send_tool_result(stdout, id, &result_json.to_string());
}

// ---------------------------------------------------------------------------
// Tool: flux_prove
// ---------------------------------------------------------------------------

fn handle_flux_prove(stdout: &std::io::Stdout, id: Value, args: &Value) {
    let ftl_source = match args.get("ftl_source").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => {
            send_tool_error(stdout, id, "Missing required argument: ftl_source");
            return;
        }
    };

    let bmc = args.get("bmc").and_then(|v| v.as_bool()).unwrap_or(false);
    let bmc_depth = args.get("bmc_depth").and_then(|v| v.as_u64()).unwrap_or(10) as u32;

    // Parse and validate first
    let check_result = pipeline::run_check(ftl_source);
    let ast = match &check_result.ast {
        Some(a) => a,
        None => {
            let json = pipeline::result_to_json(&check_result).unwrap_or_else(|e| e);
            send_tool_error(stdout, id, &format!("Check failed:\n{}", json));
            return;
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

    let prover_config = ProverConfig {
        bmc_config,
        ..ProverConfig::default()
    };

    let proof_results = prove_contracts(ast, &prover_config);
    let json = serde_json::to_string(&proof_results)
        .unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e));

    send_tool_result(stdout, id, &json);
}

// ---------------------------------------------------------------------------
// Tool: flux_generate
// ---------------------------------------------------------------------------

fn handle_flux_generate(stdout: &std::io::Stdout, id: Value, args: &Value) {
    let requirement = match args.get("requirement").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => {
            send_tool_error(stdout, id, "Missing required argument: requirement");
            return;
        }
    };

    let requirement_type = args
        .get("requirement_type")
        .and_then(|v| v.as_str())
        .unwrap_or("translate");
    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("anthropic");
    let model = args.get("model").and_then(|v| v.as_str());
    let max_iterations = args
        .get("max_iterations")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as u32;

    // Parse requirement type
    let req_type = match RequirementType::from_str_loose(requirement_type) {
        Ok(t) => t,
        Err(e) => {
            send_tool_error(stdout, id, &format!("Invalid requirement_type: {}", e));
            return;
        }
    };

    // Parse provider
    let llm_provider = match LlmProvider::from_str_loose(provider) {
        Ok(p) => p,
        Err(e) => {
            send_tool_error(stdout, id, &format!("Invalid provider: {}", e));
            return;
        }
    };

    // Build config from environment
    let mut config = match LlmConfig::from_env(llm_provider, model.map(String::from)) {
        Ok(c) => c,
        Err(e) => {
            send_tool_error(stdout, id, &format!("Configuration error: {}", e));
            return;
        }
    };
    config.max_iterations = max_iterations;

    // Create generation loop
    let gen_loop = match GenerationLoop::new(config) {
        Ok(l) => l,
        Err(e) => {
            send_tool_error(
                stdout,
                id,
                &format!("Failed to create generation loop: {}", e),
            );
            return;
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
            send_tool_error(
                stdout,
                id,
                &format!("Failed to create async runtime: {}", e),
            );
            return;
        }
    };

    let result = match rt.block_on(gen_loop.generate(&request)) {
        Ok(r) => r,
        Err(e) => {
            send_tool_error(stdout, id, &format!("Generation failed: {}", e));
            return;
        }
    };

    let json = match serde_json::to_string(&result) {
        Ok(j) => j,
        Err(e) => {
            send_tool_error(stdout, id, &format!("Serialization error: {}", e));
            return;
        }
    };

    send_tool_result(stdout, id, &json);
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

fn main() {
    let stdin = BufReader::new(std::io::stdin());
    let stdout = std::io::stdout();

    for line in stdin.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                send_error(&stdout, Value::Null, -32700, &format!("Parse error: {}", e));
                continue;
            }
        };

        // Notifications (no id) don't get responses
        if request.id.is_none() {
            continue;
        }

        let id = request.id.unwrap();

        match request.method.as_str() {
            "initialize" => handle_initialize(&stdout, id),
            "tools/list" => handle_tools_list(&stdout, id),
            "tools/call" => handle_tools_call(&stdout, id, request.params),
            _ => send_error(
                &stdout,
                id,
                -32601,
                &format!("Method not found: {}", request.method),
            ),
        }
    }
}
