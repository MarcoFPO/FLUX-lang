//! Phase 13: LLM Integration — Iterative FTL generation with feedback loop.
//!
//! This module provides an abstraction over LLM providers (Anthropic Claude,
//! OpenAI-compatible) and implements a generate-check-repair loop that turns
//! natural-language requirements into valid, verified FTL programs.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::compiler;
use crate::error::Status;
use crate::feedback::{self, LlmFeedback, ValidationError as FeedbackValidationError};
use crate::parser::parse_ftl;
use crate::prover::{prove_contracts, ProofResult, ProofStatus, ProverConfig};
use crate::region_checker::check_regions;
use crate::type_checker::check_types_and_effects;
use crate::validator::validate;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// LLM provider selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    /// Anthropic Claude API (Messages API v1).
    Anthropic,
    /// OpenAI-compatible chat completions endpoint (works with local LLMs too).
    OpenAi,
}

impl fmt::Display for LlmProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmProvider::Anthropic => write!(f, "anthropic"),
            LlmProvider::OpenAi => write!(f, "openai"),
        }
    }
}

impl LlmProvider {
    /// Parse a provider name from a CLI string.
    pub fn from_str_loose(s: &str) -> Result<Self, LlmError> {
        match s.to_lowercase().as_str() {
            "anthropic" | "claude" => Ok(LlmProvider::Anthropic),
            "openai" | "openai-compatible" | "local" => Ok(LlmProvider::OpenAi),
            other => Err(LlmError::ConfigError(format!(
                "Unknown provider '{}'. Use 'anthropic' or 'openai'.",
                other
            ))),
        }
    }

    /// Default model name for this provider.
    pub fn default_model(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "claude-sonnet-4-20250514",
            LlmProvider::OpenAi => "gpt-4o",
        }
    }

    /// Environment variable name for the API key.
    pub fn api_key_env(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "ANTHROPIC_API_KEY",
            LlmProvider::OpenAi => "OPENAI_API_KEY",
        }
    }

    /// Default base URL for the API.
    pub fn default_base_url(&self) -> &'static str {
        match self {
            LlmProvider::Anthropic => "https://api.anthropic.com",
            LlmProvider::OpenAi => "https://api.openai.com",
        }
    }
}

/// Full configuration for an LLM-backed generation session.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider: LlmProvider,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f64,
    pub max_iterations: u32,
    pub timeout_secs: u64,
    pub base_url: String,
}

impl LlmConfig {
    /// Build configuration from environment variables and optional overrides.
    pub fn from_env(provider: LlmProvider, model: Option<String>) -> Result<Self, LlmError> {
        let api_key = std::env::var(provider.api_key_env())
            .map_err(|_| LlmError::MissingApiKey(provider.api_key_env().to_string()))?;

        let model = model.unwrap_or_else(|| provider.default_model().to_string());
        let base_url = std::env::var("LLM_BASE_URL")
            .unwrap_or_else(|_| provider.default_base_url().to_string());

        Ok(Self {
            provider,
            api_key,
            model,
            max_tokens: 4096,
            temperature: 0.7,
            max_iterations: 5,
            timeout_secs: 60,
            base_url,
        })
    }

    /// Create a config with explicit values (useful for testing).
    pub fn new(
        provider: LlmProvider,
        api_key: String,
        model: String,
    ) -> Self {
        Self {
            base_url: provider.default_base_url().to_string(),
            provider,
            api_key,
            model,
            max_tokens: 4096,
            temperature: 0.7,
            max_iterations: 5,
            timeout_secs: 60,
        }
    }
}

// ---------------------------------------------------------------------------
// Requirement types
// ---------------------------------------------------------------------------

/// Classification of the generation task — influences prompt style and strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RequirementType {
    /// Direct synthesis: turn a description into exactly one FTL graph.
    Translate,
    /// Generate multiple variants and pick the best (Pareto-optimal).
    Optimize,
    /// Exploratory synthesis with evolution-style iteration.
    Invent,
    /// Open-ended search in the graph space.
    Discover,
}

impl RequirementType {
    /// Parse from CLI string.
    pub fn from_str_loose(s: &str) -> Result<Self, LlmError> {
        match s.to_lowercase().as_str() {
            "translate" | "t" => Ok(RequirementType::Translate),
            "optimize" | "o" => Ok(RequirementType::Optimize),
            "invent" | "i" => Ok(RequirementType::Invent),
            "discover" | "d" => Ok(RequirementType::Discover),
            other => Err(LlmError::ConfigError(format!(
                "Unknown requirement type '{}'. Use translate/optimize/invent/discover.",
                other
            ))),
        }
    }

    /// Human-readable instruction for the LLM.
    pub fn instruction(&self) -> &'static str {
        match self {
            RequirementType::Translate => {
                "TRANSLATE the following requirement into exactly one FTL computation graph."
            }
            RequirementType::Optimize => {
                "OPTIMIZE: generate multiple FTL graph variants for the requirement \
                 and choose the most efficient one."
            }
            RequirementType::Invent => {
                "INVENT: explore novel computation structures to solve the requirement. \
                 Be creative with node composition."
            }
            RequirementType::Discover => {
                "DISCOVER: perform an open-ended search in the FTL graph space for the \
                 requirement. Consider unconventional approaches."
            }
        }
    }
}

/// A generation request from the user.
#[derive(Debug, Clone)]
pub struct GenerateRequest {
    /// Natural-language description of what the program should do.
    pub requirement: String,
    /// Classification of the task.
    pub requirement_type: RequirementType,
    /// Optional additional context (e.g. constraints, target platform).
    pub context: Option<String>,
    /// Example FTL programs to include in the prompt.
    pub examples: Vec<String>,
}

// ---------------------------------------------------------------------------
// API message types
// ---------------------------------------------------------------------------

/// A single message in an LLM conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".to_string(), content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".to_string(), content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".to_string(), content: content.into() }
    }
}

// ---------------------------------------------------------------------------
// Anthropic API types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    system: String,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
}

// ---------------------------------------------------------------------------
// OpenAI API types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    max_tokens: u32,
    temperature: f64,
    messages: Vec<OpenAiMessage>,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Deserialize)]
struct OpenAiResponseMessage {
    content: String,
}

// ---------------------------------------------------------------------------
// Prompt templates
// ---------------------------------------------------------------------------

/// Centralized prompt construction for FTL generation and repair.
pub struct PromptTemplates;

impl PromptTemplates {
    /// Comprehensive system prompt that teaches the LLM about FTL syntax.
    pub fn system_prompt() -> String {
        r#"You are an expert FTL (FLUX Text Language) code generator. FTL is a graph-based intermediate representation where programs are defined as typed computation graphs with formal verification contracts.

## FTL Node Types

Every node has a prefixed ID (e.g. T:a1, C:c1). Available node types:

### T-Node (Type definitions)
- `T:id = integer { bits: N, signed: true/false }` — integer type
- `T:id = float { bits: N }` — floating point type
- `T:id = boolean` — boolean type
- `T:id = unit` — unit/void type
- `T:id = array { element: T:ref, max_length: N }` — fixed-max-length array
- `T:id = struct { fields: [name1: T:ref, name2: T:ref] }` — product type
- `T:id = variant { cases: [TAG1: T:ref, TAG2: T:ref] }` — sum type
- `T:id = opaque { size: N, align: N }` — opaque external type
- `T:id = fn { params: [T:ref, ...], result: T:ref, effects: [EFF] }` — function type

Builtin type aliases: u8, u16, u32, u64, i8, i16, i32, i64, f32, f64, bool, unit

### R-Node (Region / memory lifetime)
- `R:id = region { lifetime: static }` — lives for program duration
- `R:id = region { lifetime: scoped, parent: R:ref }` — scoped to parent region

### C-Node (Compute — pure, no side effects)
- `C:id = const { value: LITERAL, type: T:ref }` — constant value
- `C:id = const_bytes { value: [byte, ...], type: T:ref, region: R:ref }` — byte array
- `C:id = add/sub/mul/div/mod/and/or/xor/shl/shr/eq/neq/lt/lte/gt/gte { inputs: [ref, ref], type: T:ref }` — arithmetic/logic
- `C:id = call_pure { target: "fn_name", inputs: [ref, ...], type: T:ref }` — pure function call

### E-Node (Effect — side-effectful operations)
- `E:id = syscall_write/read/exit/open/close/ioctl/nanosleep { inputs: [ref, ...], type: T:ref, effects: [IO/PROC/MEM/...], success: K:ref, failure: K:ref }` — system calls
- `E:id = call_extern { target: X:ref, inputs: [ref, ...], type: T:ref, effects: [EFF], success: K:ref, failure: K:ref }` — FFI call

### K-Node (Control flow)
- `K:id = seq { steps: [ref, ...] }` — sequential execution
- `K:id = branch { condition: C:ref, true: K:ref, false: K:ref }` — conditional
- `K:id = loop { condition: C:ref, body: K:ref, state: ref, state_type: T:ref }` — loop
- `K:id = par { branches: [K:ref, ...], sync: barrier/none }` — parallel execution

### V-Node (Contract — formal verification)
- `V:id = contract { target: ref, pre: FORMULA }` — precondition
- `V:id = contract { target: ref, post: FORMULA }` — postcondition
- `V:id = contract { target: ref, invariant: FORMULA }` — loop invariant
- `V:id = contract { target: ref, assume: FORMULA, post: FORMULA }` — assumption + postcondition
- `V:id = contract { target: ref, trust: EXTERN, assume: FORMULA, post: FORMULA }` — trusted external

Formula language: `==`, `!=`, `<`, `<=`, `>`, `>=`, `AND`, `OR`, `NOT`, `result`, `state`, `forall i in START..END: BODY`, field access with `.field` or `.val`

### M-Node (Memory operations)
- `M:id = alloc { type: T:ref, region: R:ref }` — allocate in region
- `M:id = load { source: M:ref, index: C:ref, type: T:ref }` — load from memory
- `M:id = store { target: M:ref, index: C:ref, value: ref }` — store to memory

### X-Node (Extern / FFI declarations)
- `X:id = extern { name: "c_function", abi: C, params: [T:ref, ...], result: T:ref, effects: [EFF] }` — external function

### Entry point
Every program must end with: `entry: K:ref`

## Example: Hello World

```ftl
// Types
T:a1 = array { element: u8, max_length: 12 }
T:a2 = integer { bits: 64, signed: false }
T:a3 = unit

// Regions
R:b1 = region { lifetime: static }

// Compute
C:c1 = const_bytes { value: [72,101,108,108,111,32,87,111,114,108,100,10], type: T:a1, region: R:b1 }
C:c2 = const { value: 1, type: T:a2 }
C:c3 = const { value: 12, type: T:a2 }
C:c4 = const { value: 0, type: T:a2 }
C:c5 = const { value: 1, type: T:a2 }

// Effects
E:d1 = syscall_write { inputs: [C:c2, C:c1, C:c3], type: T:a2, effects: [IO], success: K:f2, failure: K:f3 }

// Success: exit(0)
K:f2 = seq { steps: [E:d2] }
E:d2 = syscall_exit { inputs: [C:c4], type: T:a3, effects: [PROC] }

// Failure: exit(1)
K:f3 = seq { steps: [E:d3] }
E:d3 = syscall_exit { inputs: [C:c5], type: T:a3, effects: [PROC] }

// Contracts
V:e1 = contract { target: E:d1, pre: C:c2.val == 1 }
V:e2 = contract { target: E:d1, pre: C:c3.val == 12 }

// Entry
K:f1 = seq { steps: [E:d1] }
entry: K:f1
```

## Rules for Valid FTL

1. Every node ID must be unique and use the correct prefix (T:, C:, E:, K:, V:, M:, R:, X:).
2. All referenced nodes must be defined somewhere in the program.
3. Exactly one `entry:` declaration required, pointing to a K-node.
4. Effect nodes (E:) must declare their effects list (IO, MEM, PROC, NET, etc.).
5. Effect nodes must have success and failure continuation K-nodes (except syscall_exit).
6. Scoped regions must reference an existing parent region.
7. Contracts reference existing target nodes with valid formulas.
8. Types must be fully defined before use.
9. Compute nodes are pure — no side effects allowed.
10. Comments start with `//`.

## Output Format

Your response MUST contain the FTL program inside a fenced code block:

```ftl
// ... your FTL program here ...
```

Include comments explaining each section. If you cannot fulfill the requirement, explain why and provide the closest approximation in valid FTL."#.to_string()
    }

    /// Build the initial generation prompt for a request.
    pub fn generation_prompt(request: &GenerateRequest) -> String {
        let mut prompt = String::new();

        // Requirement type instruction
        prompt.push_str(request.requirement_type.instruction());
        prompt.push_str("\n\n");

        // The actual requirement
        prompt.push_str("## Requirement\n\n");
        prompt.push_str(&request.requirement);
        prompt.push('\n');

        // Optional context
        if let Some(ctx) = &request.context {
            prompt.push_str("\n## Additional Context\n\n");
            prompt.push_str(ctx);
            prompt.push('\n');
        }

        // Example programs
        if !request.examples.is_empty() {
            prompt.push_str("\n## Reference Examples\n\n");
            for (i, example) in request.examples.iter().enumerate() {
                prompt.push_str(&format!("### Example {}\n\n```ftl\n{}\n```\n\n", i + 1, example));
            }
        }

        prompt.push_str("\nGenerate a complete, valid FTL program. ");
        prompt.push_str("Wrap your FTL code in a ```ftl code block.");

        prompt
    }

    /// Build a repair prompt based on feedback from the pipeline.
    pub fn repair_prompt(feedback: &LlmFeedback, previous_ftl: &str) -> String {
        let mut prompt = String::new();

        prompt.push_str("## Repair Required\n\n");
        prompt.push_str("The previous FTL program has errors that need to be fixed.\n\n");

        // Previous source
        prompt.push_str("### Previous FTL (with errors)\n\n```ftl\n");
        prompt.push_str(previous_ftl);
        prompt.push_str("\n```\n\n");

        // Feedback summary
        prompt.push_str("### Feedback\n\n");
        prompt.push_str(&format!("**Status:** {:?}\n\n", feedback.status));
        prompt.push_str(&format!("**Summary:** {}\n\n", feedback.summary));

        // Issues
        if !feedback.issues.is_empty() {
            prompt.push_str("### Issues to Fix\n\n");
            for (i, issue) in feedback.issues.iter().enumerate() {
                prompt.push_str(&format!(
                    "{}. **[{:?}]** `{}`: {}\n",
                    i + 1,
                    issue.severity,
                    issue.node_id,
                    issue.message
                ));
                if let Some(suggestion) = &issue.suggestion {
                    prompt.push_str(&format!("   - Suggestion: {}\n", suggestion.description));
                    if let Some(example) = &suggestion.example {
                        prompt.push_str(&format!("   - Example: `{}`\n", example));
                    }
                }
            }
            prompt.push('\n');
        }

        // Iteration hints
        prompt.push_str("### Repair Strategy\n\n");
        prompt.push_str(&format!(
            "- Estimated fixes needed: {}\n",
            feedback.iteration_hint.estimated_fixes
        ));
        prompt.push_str(&format!("- Strategy: {}\n", feedback.iteration_hint.strategy));
        if !feedback.iteration_hint.priority_order.is_empty() {
            prompt.push_str(&format!(
                "- Priority order: {}\n",
                feedback.iteration_hint.priority_order.join(", ")
            ));
        }

        prompt.push_str(
            "\nFix ALL listed issues and produce a corrected, complete FTL program. \
             Wrap your FTL code in a ```ftl code block.",
        );

        prompt
    }

    /// Build a repair prompt from raw parse errors (when feedback could not be generated).
    pub fn repair_prompt_from_parse_errors(errors: &[String], previous_ftl: &str) -> String {
        let mut prompt = String::new();

        prompt.push_str("## Parse Error — Repair Required\n\n");
        prompt.push_str("The previous FTL program failed to parse.\n\n");

        prompt.push_str("### Previous FTL (failed to parse)\n\n```ftl\n");
        prompt.push_str(previous_ftl);
        prompt.push_str("\n```\n\n");

        prompt.push_str("### Parse Errors\n\n");
        for (i, err) in errors.iter().enumerate() {
            prompt.push_str(&format!("{}. {}\n", i + 1, err));
        }

        prompt.push_str(
            "\nFix all parse errors and produce a syntactically valid FTL program. \
             Wrap your FTL code in a ```ftl code block.",
        );

        prompt
    }
}

// ---------------------------------------------------------------------------
// Pipeline result — runs parse + validate + prove + compile
// ---------------------------------------------------------------------------

/// Outcome of running the full FTL pipeline on a source string.
#[derive(Debug)]
pub struct PipelineResult {
    pub parse_ok: bool,
    pub parse_errors: Vec<String>,
    pub validation_ok: bool,
    pub validation_errors: Vec<String>,
    pub proof_results: Vec<ProofResult>,
    pub feedback: Option<LlmFeedback>,
    pub compiled: bool,
}

impl PipelineResult {
    /// True if everything passed: parse, validate, prove, compile.
    pub fn is_success(&self) -> bool {
        self.parse_ok
            && self.validation_ok
            && self.compiled
            && !self.proof_results.iter().any(|r| r.status == ProofStatus::Disproven)
    }

    /// True if parse and validate passed but some proofs are UNKNOWN (not DISPROVEN).
    pub fn is_partial_success(&self) -> bool {
        self.parse_ok
            && self.validation_ok
            && !self.proof_results.iter().any(|r| r.status == ProofStatus::Disproven)
            && self.proof_results.iter().any(|r| r.status == ProofStatus::Unknown)
    }

    /// Summary string for an iteration record.
    pub fn proof_summary(&self) -> Vec<String> {
        self.proof_results
            .iter()
            .map(|r| format!("{}: {:?}", r.contract_id, r.status))
            .collect()
    }
}

/// Run the full FTL pipeline: parse -> validate -> prove -> compile.
pub fn run_pipeline(ftl_source: &str) -> PipelineResult {
    // --- Parse ---
    let parse_result = parse_ftl(ftl_source);

    let ast = match parse_result.status {
        Status::Ok => match parse_result.ast {
            Some(ast) => ast,
            None => {
                return PipelineResult {
                    parse_ok: false,
                    parse_errors: vec!["Parser returned Ok but no AST".to_string()],
                    validation_ok: false,
                    validation_errors: Vec::new(),
                    proof_results: Vec::new(),
                    feedback: None,
                    compiled: false,
                };
            }
        },
        Status::Error => {
            let errors: Vec<String> = parse_result
                .errors
                .iter()
                .map(|e| format!("L{}:C{}: {}", e.line, e.column, e.message))
                .collect();

            let fb = feedback::generate_feedback(&parse_result.errors, &[], &[]);

            return PipelineResult {
                parse_ok: false,
                parse_errors: errors,
                validation_ok: false,
                validation_errors: Vec::new(),
                proof_results: Vec::new(),
                feedback: Some(fb),
                compiled: false,
            };
        }
    };

    // --- Validate (structural + type + region) ---
    let vr = validate(&ast);
    let type_errors = check_types_and_effects(&ast);
    let region_errors = check_regions(&ast);

    let mut all_validation_errors: Vec<FeedbackValidationError> = Vec::new();
    let mut validation_messages: Vec<String> = Vec::new();

    for e in &vr.errors {
        validation_messages.push(format!("[{}] {}: {}", e.error_code, e.node_id, e.message));
        all_validation_errors.push(FeedbackValidationError {
            error_code: e.error_code,
            node_id: e.node_id.clone(),
            violation: e.violation.clone(),
            message: e.message.clone(),
            suggestion: e.suggestion.clone(),
        });
    }
    for e in &vr.warnings {
        validation_messages.push(format!("[{}] {}: {} (warning)", e.error_code, e.node_id, e.message));
        all_validation_errors.push(FeedbackValidationError {
            error_code: e.error_code,
            node_id: e.node_id.clone(),
            violation: e.violation.clone(),
            message: e.message.clone(),
            suggestion: e.suggestion.clone(),
        });
    }
    for e in &type_errors {
        validation_messages.push(format!("[{}] {}: {}", e.error_code, e.node_id, e.message));
        all_validation_errors.push(FeedbackValidationError {
            error_code: e.error_code,
            node_id: e.node_id.clone(),
            violation: e.violation.clone(),
            message: e.message.clone(),
            suggestion: e.suggestion.clone(),
        });
    }
    for e in &region_errors {
        validation_messages.push(format!("[{}] {}: {}", e.error_code, e.node_id, e.message));
        all_validation_errors.push(FeedbackValidationError {
            error_code: e.error_code,
            node_id: e.node_id.clone(),
            violation: e.violation.clone(),
            message: e.message.clone(),
            suggestion: e.suggestion.clone(),
        });
    }

    let has_fatal = all_validation_errors
        .iter()
        .any(|e| e.error_code < 2000 || e.error_code >= 3000);

    let validation_ok = !has_fatal;

    // --- Prove contracts (only when no fatal validation errors) ---
    let proof_results = if validation_ok {
        let config = ProverConfig::default();
        prove_contracts(&ast, &config)
    } else {
        Vec::new()
    };

    let has_disproven = proof_results
        .iter()
        .any(|r| r.status == ProofStatus::Disproven);

    // --- Compile (only when no fatal validation errors) ---
    let compiled = if validation_ok && !has_disproven {
        compiler::compile(&ast).is_ok()
    } else {
        false
    };

    // --- Feedback ---
    let fb = feedback::generate_feedback(&[], &all_validation_errors, &proof_results);

    PipelineResult {
        parse_ok: true,
        parse_errors: Vec::new(),
        validation_ok,
        validation_errors: validation_messages,
        proof_results,
        feedback: Some(fb),
        compiled,
    }
}

// ---------------------------------------------------------------------------
// FTL extraction from LLM response
// ---------------------------------------------------------------------------

/// Extract FTL source code from an LLM response that contains a ```ftl ... ``` block.
pub fn extract_ftl(response: &str) -> Option<String> {
    // Look for ```ftl ... ``` blocks
    let ftl_start = "```ftl";
    let start_idx = response.find(ftl_start)?;
    let after_marker = start_idx + ftl_start.len();

    // Skip to next newline after ```ftl
    let content_start = response[after_marker..].find('\n')? + after_marker + 1;

    // Find the closing ```
    let content_after = &response[content_start..];
    let end_idx = content_after.find("```")?;

    let ftl = content_after[..end_idx].trim().to_string();
    if ftl.is_empty() {
        None
    } else {
        Some(ftl)
    }
}

// ---------------------------------------------------------------------------
// Generation result types
// ---------------------------------------------------------------------------

/// Final result of a generate-check-repair loop.
#[derive(Debug, Serialize)]
pub struct GenerationResult {
    pub ftl_source: String,
    pub iterations: u32,
    pub final_status: GenerationStatus,
    pub history: Vec<IterationRecord>,
}

/// Record of one iteration in the generate/repair loop.
#[derive(Debug, Serialize)]
pub struct IterationRecord {
    pub iteration: u32,
    pub ftl_source: String,
    pub parse_ok: bool,
    pub validation_errors: usize,
    pub proof_summary: Vec<String>,
    pub feedback_status: String,
}

/// Overall status of a generation attempt.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GenerationStatus {
    /// Parse + Validate + Prove + Compile all succeeded.
    Success,
    /// Parse + Validate OK, some proofs UNKNOWN (none DISPROVEN).
    PartialSuccess,
    /// Maximum iterations reached without full success.
    MaxIterations,
    /// Unrecoverable failure.
    Failed,
}

// ---------------------------------------------------------------------------
// LLM client
// ---------------------------------------------------------------------------

/// HTTP client for communicating with LLM APIs.
pub struct LlmClient {
    config: LlmConfig,
    http_client: reqwest::Client,
}

impl LlmClient {
    /// Create a new client from config.
    pub fn new(config: LlmConfig) -> Result<Self, LlmError> {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| LlmError::NetworkError(format!("Failed to build HTTP client: {}", e)))?;

        Ok(Self { config, http_client })
    }

    /// Send messages to the LLM and return the text response.
    pub async fn call(&self, system: &str, messages: &[Message]) -> Result<String, LlmError> {
        match self.config.provider {
            LlmProvider::Anthropic => self.call_anthropic(system, messages).await,
            LlmProvider::OpenAi => self.call_openai(system, messages).await,
        }
    }

    async fn call_anthropic(&self, system: &str, messages: &[Message]) -> Result<String, LlmError> {
        let url = format!("{}/v1/messages", self.config.base_url);

        let api_messages: Vec<AnthropicMessage> = messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| AnthropicMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let body = AnthropicRequest {
            model: self.config.model.clone(),
            max_tokens: self.config.max_tokens,
            temperature: Some(self.config.temperature),
            system: system.to_string(),
            messages: api_messages,
        };

        let resp = self
            .http_client
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    LlmError::Timeout
                } else {
                    LlmError::NetworkError(e.to_string())
                }
            })?;

        let status = resp.status();
        let response_text = resp
            .text()
            .await
            .map_err(|e| LlmError::NetworkError(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(LlmError::ApiError(format!(
                "Anthropic API returned {}: {}",
                status, response_text
            )));
        }

        let parsed: AnthropicResponse = serde_json::from_str(&response_text)
            .map_err(|e| LlmError::ParseError(format!("Failed to parse Anthropic response: {} — body: {}", e, response_text)))?;

        parsed
            .content
            .first()
            .map(|c| c.text.clone())
            .ok_or_else(|| LlmError::ApiError("Anthropic response had no content blocks".to_string()))
    }

    async fn call_openai(&self, system: &str, messages: &[Message]) -> Result<String, LlmError> {
        let url = format!("{}/v1/chat/completions", self.config.base_url);

        let mut api_messages = vec![OpenAiMessage {
            role: "system".to_string(),
            content: system.to_string(),
        }];

        for m in messages {
            if m.role != "system" {
                api_messages.push(OpenAiMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                });
            }
        }

        let body = OpenAiRequest {
            model: self.config.model.clone(),
            max_tokens: self.config.max_tokens,
            temperature: self.config.temperature,
            messages: api_messages,
        };

        let resp = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    LlmError::Timeout
                } else {
                    LlmError::NetworkError(e.to_string())
                }
            })?;

        let status = resp.status();
        let response_text = resp
            .text()
            .await
            .map_err(|e| LlmError::NetworkError(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(LlmError::ApiError(format!(
                "OpenAI API returned {}: {}",
                status, response_text
            )));
        }

        let parsed: OpenAiResponse = serde_json::from_str(&response_text)
            .map_err(|e| LlmError::ParseError(format!("Failed to parse OpenAI response: {} — body: {}", e, response_text)))?;

        parsed
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| LlmError::ApiError("OpenAI response had no choices".to_string()))
    }
}

// ---------------------------------------------------------------------------
// Generation loop
// ---------------------------------------------------------------------------

/// The main generate-check-repair loop.
pub struct GenerationLoop {
    client: LlmClient,
    max_iterations: u32,
}

impl GenerationLoop {
    pub fn new(config: LlmConfig) -> Result<Self, LlmError> {
        let max_iterations = config.max_iterations;
        let client = LlmClient::new(config)?;
        Ok(Self {
            client,
            max_iterations,
        })
    }

    /// Run the iterative generation loop.
    ///
    /// 1. Send system_prompt + generation_prompt to LLM
    /// 2. Extract FTL from response
    /// 3. Run pipeline (parse + validate + prove + compile)
    /// 4. If errors: build repair prompt with feedback, send again
    /// 5. Repeat until success or max_iterations
    pub async fn generate(&self, request: &GenerateRequest) -> Result<GenerationResult, LlmError> {
        let system = PromptTemplates::system_prompt();
        let initial_prompt = PromptTemplates::generation_prompt(request);

        let mut history: Vec<IterationRecord> = Vec::new();
        let mut messages: Vec<Message> = vec![Message::user(&initial_prompt)];
        let mut last_ftl = String::new();

        for iteration in 1..=self.max_iterations {
            // Call LLM
            let response = self.client.call(&system, &messages).await?;

            // Extract FTL from response
            let ftl = match extract_ftl(&response) {
                Some(ftl) => ftl,
                None => {
                    // No FTL block found — ask the LLM to try again
                    messages.push(Message::assistant(&response));
                    messages.push(Message::user(
                        "Your response did not contain a ```ftl code block. \
                         Please provide the FTL program inside a ```ftl ... ``` block.",
                    ));

                    history.push(IterationRecord {
                        iteration,
                        ftl_source: String::new(),
                        parse_ok: false,
                        validation_errors: 0,
                        proof_summary: Vec::new(),
                        feedback_status: "NO_FTL_BLOCK".to_string(),
                    });
                    continue;
                }
            };

            last_ftl = ftl.clone();

            // Run pipeline
            let pipeline = run_pipeline(&ftl);

            // Record this iteration
            let feedback_status = pipeline
                .feedback
                .as_ref()
                .map(|f| format!("{:?}", f.status))
                .unwrap_or_else(|| "N/A".to_string());

            history.push(IterationRecord {
                iteration,
                ftl_source: ftl.clone(),
                parse_ok: pipeline.parse_ok,
                validation_errors: pipeline.validation_errors.len(),
                proof_summary: pipeline.proof_summary(),
                feedback_status: feedback_status.clone(),
            });

            // Check if we succeeded
            if pipeline.is_success() {
                return Ok(GenerationResult {
                    ftl_source: ftl,
                    iterations: iteration,
                    final_status: GenerationStatus::Success,
                    history,
                });
            }

            if pipeline.is_partial_success() {
                return Ok(GenerationResult {
                    ftl_source: ftl,
                    iterations: iteration,
                    final_status: GenerationStatus::PartialSuccess,
                    history,
                });
            }

            // Build repair prompt
            let repair = if !pipeline.parse_ok {
                PromptTemplates::repair_prompt_from_parse_errors(&pipeline.parse_errors, &ftl)
            } else if let Some(fb) = &pipeline.feedback {
                PromptTemplates::repair_prompt(fb, &ftl)
            } else {
                // Shouldn't happen, but handle gracefully
                let errors = pipeline
                    .validation_errors
                    .iter()
                    .chain(pipeline.parse_errors.iter())
                    .cloned()
                    .collect::<Vec<_>>();
                PromptTemplates::repair_prompt_from_parse_errors(&errors, &ftl)
            };

            // Append assistant response and repair request to conversation
            messages.push(Message::assistant(&response));
            messages.push(Message::user(&repair));
        }

        // Max iterations reached
        Ok(GenerationResult {
            ftl_source: last_ftl,
            iterations: self.max_iterations,
            final_status: if history
                .last()
                .is_some_and(|h| h.parse_ok && h.validation_errors == 0)
            {
                GenerationStatus::PartialSuccess
            } else {
                GenerationStatus::MaxIterations
            },
            history,
        })
    }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur during LLM-powered generation.
#[derive(Debug)]
pub enum LlmError {
    /// The LLM API returned an error.
    ApiError(String),
    /// The request timed out.
    Timeout,
    /// Failed to parse the LLM response.
    ParseError(String),
    /// Network-level error (DNS, connection, TLS).
    NetworkError(String),
    /// Required API key environment variable not set.
    MissingApiKey(String),
    /// Invalid configuration.
    ConfigError(String),
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::ApiError(msg) => write!(f, "API error: {}", msg),
            LlmError::Timeout => write!(f, "Request timed out"),
            LlmError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            LlmError::NetworkError(msg) => write!(f, "Network error: {}", msg),
            LlmError::MissingApiKey(var) => {
                write!(f, "Missing API key: set {} environment variable", var)
            }
            LlmError::ConfigError(msg) => write!(f, "Config error: {}", msg),
        }
    }
}

impl std::error::Error for LlmError {}

// ---------------------------------------------------------------------------
// Tests (unit-level, no HTTP)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_ftl_basic() {
        let response = "Here is your program:\n\n```ftl\nT:a1 = unit\nentry: K:f1\n```\n\nDone.";
        let result = extract_ftl(response);
        assert!(result.is_some());
        let ftl = result.unwrap();
        assert!(ftl.contains("T:a1 = unit"));
        assert!(ftl.contains("entry: K:f1"));
    }

    #[test]
    fn test_extract_ftl_no_block() {
        let response = "Sorry, I cannot generate FTL for that requirement.";
        assert!(extract_ftl(response).is_none());
    }

    #[test]
    fn test_extract_ftl_empty_block() {
        let response = "```ftl\n\n```";
        assert!(extract_ftl(response).is_none());
    }

    #[test]
    fn test_extract_ftl_with_other_blocks() {
        let response = "First some rust:\n```rust\nfn main() {}\n```\n\nNow FTL:\n```ftl\nT:a1 = unit\nentry: K:f1\n```\n";
        let result = extract_ftl(response).unwrap();
        assert!(result.contains("T:a1 = unit"));
        assert!(!result.contains("fn main"));
    }

    #[test]
    fn test_provider_from_str() {
        assert_eq!(
            LlmProvider::from_str_loose("anthropic").unwrap(),
            LlmProvider::Anthropic
        );
        assert_eq!(
            LlmProvider::from_str_loose("claude").unwrap(),
            LlmProvider::Anthropic
        );
        assert_eq!(
            LlmProvider::from_str_loose("openai").unwrap(),
            LlmProvider::OpenAi
        );
        assert!(LlmProvider::from_str_loose("unknown").is_err());
    }

    #[test]
    fn test_requirement_type_from_str() {
        assert_eq!(
            RequirementType::from_str_loose("translate").unwrap(),
            RequirementType::Translate
        );
        assert_eq!(
            RequirementType::from_str_loose("optimize").unwrap(),
            RequirementType::Optimize
        );
        assert_eq!(
            RequirementType::from_str_loose("invent").unwrap(),
            RequirementType::Invent
        );
        assert_eq!(
            RequirementType::from_str_loose("discover").unwrap(),
            RequirementType::Discover
        );
        assert!(RequirementType::from_str_loose("x").is_err());
    }

    #[test]
    fn test_message_constructors() {
        let s = Message::system("hello");
        assert_eq!(s.role, "system");
        assert_eq!(s.content, "hello");

        let u = Message::user("world");
        assert_eq!(u.role, "user");

        let a = Message::assistant("response");
        assert_eq!(a.role, "assistant");
    }

    #[test]
    fn test_llm_config_defaults() {
        let config = LlmConfig::new(
            LlmProvider::Anthropic,
            "test-key".to_string(),
            "claude-sonnet-4-20250514".to_string(),
        );
        assert_eq!(config.max_tokens, 4096);
        assert!((config.temperature - 0.7).abs() < f64::EPSILON);
        assert_eq!(config.max_iterations, 5);
        assert_eq!(config.timeout_secs, 60);
    }

    #[test]
    fn test_provider_display() {
        assert_eq!(LlmProvider::Anthropic.to_string(), "anthropic");
        assert_eq!(LlmProvider::OpenAi.to_string(), "openai");
    }

    #[test]
    fn test_provider_default_model() {
        assert!(LlmProvider::Anthropic.default_model().contains("claude"));
        assert!(LlmProvider::OpenAi.default_model().contains("gpt"));
    }
}
