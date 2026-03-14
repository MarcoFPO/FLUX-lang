# FLUX MCP Server Integration

## Configuration

### Claude Desktop
Add to `claude_desktop_config.json`:
```json
{
  "mcpServers": {
    "flux-ftl": {
      "command": "/path/to/flux-mcp",
      "args": []
    }
  }
}
```

### Claude Code
Add to `.mcp.json` in project root or `~/.claude/mcp.json`:
```json
{
  "mcpServers": {
    "flux-ftl": {
      "command": "/path/to/flux-mcp",
      "args": []
    }
  }
}
```

### Generic MCP Client
The server communicates via JSON-RPC 2.0 over stdio (one JSON object per line).

## Available Tools

### flux_check

Validates FTL source code through the full pipeline: parse, structural validation, type/effect checking, region checking, contract proving, compilation.

**Input:**
```json
{
  "ftl_source": "string (required) -- FTL source code",
  "bmc": "boolean (optional) -- enable Bounded Model Checking",
  "bmc_depth": "integer (optional, default: 10) -- BMC unrolling depth"
}
```

**Output:**
```json
{
  "status": "OK | PARSE_ERROR | VALIDATION_FAIL | PROOF_FAIL",
  "parse_errors": [
    {
      "line": "integer",
      "column": "integer",
      "message": "string"
    }
  ],
  "validation_errors": [
    {
      "error_code": "integer",
      "node_id": "string -- e.g. T:a1",
      "violation": "string",
      "message": "string",
      "suggestion": "string | null"
    }
  ],
  "proof_results": [
    {
      "contract_id": "string -- V-node ID",
      "target_id": "string -- target node ID",
      "clause_index": "integer",
      "clause_kind": "pre | post | invariant | assume",
      "status": "PROVEN | DISPROVEN | UNKNOWN | ASSUMED | TIMEOUT | BMC_PROVEN | BMC_REFUTED",
      "counterexample": "string | null"
    }
  ],
  "feedback": {
    "status": "PASS | FIXABLE | FATAL",
    "summary": "string",
    "issues": [
      {
        "severity": "error | warning | info",
        "category": "parse_error | structural_validation | type_mismatch | effect_violation | region_error | contract_violation | proof_failure",
        "node_id": "string",
        "message": "string",
        "suggestion": {
          "action": "replace | add | remove | modify | restructure",
          "target_node": "string | null",
          "description": "string",
          "example": "string | null"
        },
        "context": {
          "related_nodes": ["string"],
          "expected": "string | null",
          "actual": "string | null"
        }
      }
    ],
    "iteration_hint": {
      "estimated_fixes": "integer",
      "priority_order": ["string -- node IDs to fix first"],
      "strategy": "string -- suggested repair strategy"
    }
  }
}
```

### flux_compile

Compiles FTL to content-addressed binary graph (BLAKE3 hashed).

**Input:**
```json
{
  "ftl_source": "string (required)"
}
```

**Output:**
```json
{
  "entry_hash": "string -- BLAKE3 hash of entry node",
  "total_nodes": "integer",
  "nodes": ["node metadata array"]
}
```

### flux_build

Full pipeline: parse, validate, prove, optimize, codegen (LLVM), link to executable.

**Input:**
```json
{
  "ftl_source": "string (required)",
  "output_path": "string (required) -- path for output binary",
  "target": "string (optional, default: host) -- host | x86_64 | aarch64 | riscv64 | wasm32",
  "opt_level": "integer (optional, default: 2) -- 0 (none), 1 (less), 2 (default), 3 (aggressive)",
  "bmc": "boolean (optional)",
  "bmc_depth": "integer (optional, default: 10)"
}
```

**Output:**
```json
{
  "executable_path": "string",
  "optimization_stats": {
    "nodes_before": "integer",
    "nodes_after": "integer",
    "constants_folded": "integer",
    "dead_nodes_removed": "integer",
    "identities_removed": "integer"
  }
}
```

### flux_ir

Generate LLVM IR from FTL source.

**Input:**
```json
{
  "ftl_source": "string (required)",
  "target": "string (optional, default: host)"
}
```

**Output:**
```json
{
  "llvm_ir": "string -- LLVM IR text"
}
```

### flux_evolve

Evolve graph variants using genetic algorithm (mutation, crossover, selection).

**Input:**
```json
{
  "ftl_source": "string (required)",
  "generations": "integer (optional, default: 50)",
  "population": "integer (optional, default: 30)",
  "mutation_rate": "number (optional, default: 0.3)",
  "crossover_rate": "number (optional, default: 0.5)",
  "seed": "integer (optional) -- for reproducibility"
}
```

**Output:**
```json
{
  "best_program": "object -- best evolved FTL program as JSON AST",
  "fitness": "number",
  "generations_run": "integer",
  "stats": {
    "best_fitness": "number",
    "avg_fitness": "number",
    "proven_count": "integer",
    "incubated_count": "integer"
  }
}
```

### flux_prove

Formal contract verification only (no compilation).

**Input:**
```json
{
  "ftl_source": "string (required)",
  "bmc": "boolean (optional)",
  "bmc_depth": "integer (optional, default: 10)"
}
```

**Output:**
```json
{
  "results": [
    {
      "contract_id": "string -- V-node ID",
      "target_id": "string -- target node ID",
      "clause_kind": "pre | post | invariant | assume",
      "status": "PROVEN | DISPROVEN | UNKNOWN | ASSUMED | TIMEOUT | BMC_PROVEN | BMC_REFUTED",
      "counterexample": "string | null"
    }
  ]
}
```

### flux_generate

Generate FTL from natural language or structured requirements using an LLM backend.

**Input:**
```json
{
  "requirement": "string (required) -- natural language or structured requirement",
  "requirement_type": "string (optional, default: translate) -- translate | optimize | extend",
  "provider": "string (optional, default: anthropic) -- LLM provider",
  "model": "string (optional) -- specific model name",
  "max_iterations": "integer (optional, default: 5)"
}
```

**Output:**
```json
{
  "ftl_source": "string -- generated FTL",
  "final_status": "Success | PartialSuccess | Failed",
  "iterations": "integer"
}
```

## Workflow: Generate, Check, Repair

```
1. Generate FTL for a requirement
2. Call flux_check with the FTL source
3. IF status != "OK":
   a. Read feedback.issues for error details
   b. Read feedback.iteration_hint for repair strategy
   c. Fix the FTL based on suggestions
   d. GOTO 2
4. IF status == "OK":
   a. Call flux_build to create executable
   b. OR call flux_evolve to optimize variants
   c. OR call flux_prove for deeper verification with BMC
```

## Interpreting Feedback

### Status Mapping

| feedback.status | Meaning | Action |
|-----------------|---------|--------|
| PASS | All checks passed | Proceed to build/evolve |
| FIXABLE | Errors found, repair suggestions available | Apply suggestions, re-check |
| FATAL | Structural errors, program cannot be analyzed | Rewrite affected sections |

### Issue Priority

Process issues in the order given by `feedback.iteration_hint.priority_order`.
Each entry is a node ID. Fix errors on that node first before moving to the next.

### Error Code Ranges

| Range | Category | Severity |
|-------|----------|----------|
| 1000-1999 | Structural validation | Fatal |
| 2000-2999 | Warnings | Non-fatal |
| 3000+ | Type/effect/region | Fatal |

### Proof Status Interpretation

| Status | Meaning |
|--------|---------|
| PROVEN | Contract verified correct by Z3 |
| DISPROVEN | Counterexample found -- contract violated |
| UNKNOWN | Solver timeout or undecidable |
| ASSUMED | Marked with `assume:` clause, not verified |
| BMC_PROVEN | Verified up to BMC depth |
| BMC_REFUTED | Counterexample found via BMC |
| TIMEOUT | Solver exceeded time limit |

## CLI Equivalent Commands

The MCP tools map to CLI subcommands:

| MCP Tool | CLI Command |
|----------|-------------|
| flux_check | `flux-ftl check <file> [--bmc] [--bmc-depth N]` |
| flux_compile | `flux-ftl compile <file> [-o output]` |
| flux_build | `flux-ftl build <file> [-o output] [--opt-level N] [--target T]` |
| flux_ir | `flux-ftl ir <file> [--target T]` |
| flux_evolve | `flux-ftl evolve <file> [--generations N] [--population N]` |
| flux_generate | `flux-ftl generate "<requirement>" [--provider P]` |

All CLI commands accept `-` for stdin. `flux-ftl check` defaults to stdin when no file is given.
Output is JSON to stdout.
