# FLUX FTL MCP Server Integration

The FLUX FTL MCP server (`flux-mcp`) exposes the compiler pipeline as tools for LLMs via JSON-RPC 2.0 over stdio.

## Tools

### flux_check

Parse, validate, type-check, region-check, and prove contracts for FTL source code.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "ftl_source": { "type": "string", "description": "FTL source code to check" },
    "bmc": { "type": "boolean", "default": false, "description": "Enable Bounded Model Checking as Z3 fallback" },
    "bmc_depth": { "type": "integer", "default": 10, "description": "BMC unrolling depth" }
  },
  "required": ["ftl_source"]
}
```

**Output:** JSON with `status`, `parse_errors`, `validation_errors`, `proof_results`, and LLM feedback.

### flux_compile

Compile validated FTL to a content-addressed binary graph with BLAKE3 hashes.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "ftl_source": { "type": "string", "description": "FTL source code to compile" }
  },
  "required": ["ftl_source"]
}
```

**Output:** JSON with `entry_hash`, `total_nodes`, and node details.

### flux_build

Full pipeline: check + compile + LLVM codegen + link to executable binary.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "ftl_source": { "type": "string" },
    "output_path": { "type": "string", "description": "Path for output executable" },
    "target": { "type": "string", "enum": ["host", "x86_64", "aarch64", "riscv64", "wasm32"], "default": "host" },
    "opt_level": { "type": "integer", "enum": [0, 1, 2, 3], "default": 2 }
  },
  "required": ["ftl_source", "output_path"]
}
```

**Output:** JSON with `status`, `output_path`, `opt_level`, `target`.

### flux_ir

Generate LLVM IR from FTL source.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "ftl_source": { "type": "string" },
    "target": { "type": "string", "enum": ["host", "x86_64", "aarch64", "riscv64", "wasm32"], "default": "host" }
  },
  "required": ["ftl_source"]
}
```

**Output:** LLVM IR as text.

### flux_evolve

Evolve graph variants using a genetic algorithm.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "ftl_source": { "type": "string" },
    "generations": { "type": "integer", "default": 50 },
    "population": { "type": "integer", "default": 30 },
    "mutation_rate": { "type": "number", "default": 0.3 },
    "seed": { "type": "integer", "description": "Random seed for reproducibility" }
  },
  "required": ["ftl_source"]
}
```

**Output:** JSON with `generations_run`, `best_fitness`, `avg_fitness`, `best_program`, etc.

### flux_prove

Formally prove V-Node contracts using Z3 SMT solver with optional BMC fallback.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "ftl_source": { "type": "string" },
    "bmc": { "type": "boolean", "default": false },
    "bmc_depth": { "type": "integer", "default": 10 }
  },
  "required": ["ftl_source"]
}
```

**Output:** Array of proof results, each with `contract_id`, `target_id`, `clause_index`, `clause_kind`, `status` (Proven/Disproven/Unknown/BmcProven/Assumed), and optional `counterexample`.

### flux_generate

Generate FTL programs from natural language requirements using an LLM. Iteratively generates, checks, and repairs FTL code until it passes all checks or `max_iterations` is reached.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "requirement": { "type": "string", "description": "Natural language description of the desired FTL program" },
    "requirement_type": { "type": "string", "description": "Type of requirement: translate, explain, optimize, refactor", "default": "translate" },
    "provider": { "type": "string", "description": "LLM provider: anthropic or openai", "default": "anthropic" },
    "model": { "type": "string", "description": "Model name override (optional, uses provider default if omitted)" },
    "max_iterations": { "type": "integer", "description": "Maximum number of generate-check-repair iterations", "default": 5 }
  },
  "required": ["requirement"]
}
```

**Output:** GenerationResult JSON with:
- `final_ftl_source` -- the generated FTL program (if successful)
- `final_status` -- Success, PartialSuccess, or Failed
- `iterations` -- number of iterations performed
- `iteration_history` -- details of each generate-check-repair cycle

**Environment:** Requires `ANTHROPIC_API_KEY` (for anthropic provider) or `OPENAI_API_KEY` (for openai provider) to be set. Returns a tool error if the API key is missing.

## Protocol

The server communicates via JSON-RPC 2.0 over stdin/stdout. Supported methods:

- `initialize` -- Returns server capabilities and info
- `tools/list` -- Returns all available tools with schemas
- `tools/call` -- Invoke a tool by name with arguments

## Usage

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | flux-mcp
```
