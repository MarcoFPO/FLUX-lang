//! Phase 13: LLM integration tests.
//!
//! These tests verify prompt templates, FTL extraction, pipeline logic, and
//! serialization — all without making real HTTP requests to any LLM API.

use flux_ftl::feedback::{self, ValidationError as FeedbackValidationError};
use flux_ftl::llm::{
    extract_ftl, run_pipeline, GenerateRequest, GenerationResult, GenerationStatus,
    IterationRecord, LlmConfig, LlmProvider, Message, PromptTemplates,
    RequirementType,
};

// ---------------------------------------------------------------------------
// Prompt template tests
// ---------------------------------------------------------------------------

#[test]
fn test_system_prompt_contains_ftl_syntax() {
    let prompt = PromptTemplates::system_prompt();

    // Must describe all node types
    assert!(prompt.contains("T-Node"), "Should describe T-Node");
    assert!(prompt.contains("C-Node"), "Should describe C-Node");
    assert!(prompt.contains("E-Node"), "Should describe E-Node");
    assert!(prompt.contains("K-Node"), "Should describe K-Node");
    assert!(prompt.contains("V-Node"), "Should describe V-Node");
    assert!(prompt.contains("M-Node"), "Should describe M-Node");
    assert!(prompt.contains("R-Node"), "Should describe R-Node");
    assert!(prompt.contains("X-Node"), "Should describe X-Node");

    // Must contain the ```ftl instruction
    assert!(prompt.contains("```ftl"), "Should instruct about ftl code blocks");

    // Must contain an example
    assert!(prompt.contains("entry: K:f1"), "Should contain an entry example");
}

#[test]
fn test_system_prompt_contains_hello_world() {
    let prompt = PromptTemplates::system_prompt();
    assert!(
        prompt.contains("Hello World"),
        "System prompt should include Hello World example"
    );
    assert!(
        prompt.contains("syscall_write"),
        "Example should demonstrate effect nodes"
    );
    assert!(
        prompt.contains("contract"),
        "Example should demonstrate contracts"
    );
}

#[test]
fn test_generation_prompt_contains_requirement() {
    let request = GenerateRequest {
        requirement: "Sort an array using merge sort".to_string(),
        requirement_type: RequirementType::Translate,
        context: None,
        examples: Vec::new(),
    };

    let prompt = PromptTemplates::generation_prompt(&request);

    assert!(prompt.contains("Sort an array using merge sort"));
    assert!(prompt.contains("TRANSLATE"));
    assert!(prompt.contains("```ftl"));
}

#[test]
fn test_generation_prompt_with_context() {
    let request = GenerateRequest {
        requirement: "Print hello".to_string(),
        requirement_type: RequirementType::Optimize,
        context: Some("Target is Linux x86_64".to_string()),
        examples: Vec::new(),
    };

    let prompt = PromptTemplates::generation_prompt(&request);

    assert!(prompt.contains("Linux x86_64"));
    assert!(prompt.contains("OPTIMIZE"));
}

#[test]
fn test_generation_prompt_with_examples() {
    let request = GenerateRequest {
        requirement: "Add two numbers".to_string(),
        requirement_type: RequirementType::Translate,
        context: None,
        examples: vec!["T:a1 = unit\nentry: K:f1".to_string()],
    };

    let prompt = PromptTemplates::generation_prompt(&request);

    assert!(prompt.contains("Reference Examples"));
    assert!(prompt.contains("T:a1 = unit"));
}

#[test]
fn test_repair_prompt_contains_feedback_and_previous_ftl() {
    let fb = feedback::generate_feedback(
        &[],
        &[FeedbackValidationError {
            error_code: 1001,
            node_id: "C:c1".to_string(),
            violation: "duplicate_id".to_string(),
            message: "Duplicate node ID C:c1".to_string(),
            suggestion: Some("Use a unique ID".to_string()),
        }],
        &[],
    );

    let previous = "T:a1 = unit\nC:c1 = const { value: 1, type: T:a1 }\nentry: K:f1";

    let prompt = PromptTemplates::repair_prompt(&fb, previous);

    assert!(prompt.contains("Repair Required"));
    assert!(prompt.contains("C:c1"));
    assert!(prompt.contains("Duplicate node ID"));
    assert!(prompt.contains("T:a1 = unit"));
    assert!(prompt.contains("```ftl"));
}

#[test]
fn test_repair_prompt_from_parse_errors() {
    let errors = vec!["L1:C5: expected '='".to_string()];
    let previous = "T:a1 unit";

    let prompt = PromptTemplates::repair_prompt_from_parse_errors(&errors, previous);

    assert!(prompt.contains("Parse Error"));
    assert!(prompt.contains("expected '='"));
    assert!(prompt.contains("T:a1 unit"));
}

// ---------------------------------------------------------------------------
// FTL extraction tests
// ---------------------------------------------------------------------------

#[test]
fn test_extract_ftl_basic() {
    let response = "Here is the program:\n\n```ftl\nT:a1 = unit\nentry: K:f1\n```\n\nDone.";
    let result = extract_ftl(response).unwrap();
    assert!(result.contains("T:a1 = unit"));
    assert!(result.contains("entry: K:f1"));
}

#[test]
fn test_extract_ftl_no_block() {
    let response = "I cannot generate FTL for that.";
    assert!(extract_ftl(response).is_none());
}

#[test]
fn test_extract_ftl_empty_block() {
    let response = "```ftl\n\n```";
    assert!(extract_ftl(response).is_none());
}

#[test]
fn test_extract_ftl_multiple_blocks() {
    let response = "```rust\nfn main() {}\n```\n\n```ftl\nT:a1 = boolean\nentry: K:f1\n```";
    let result = extract_ftl(response).unwrap();
    assert!(result.contains("T:a1 = boolean"));
    assert!(!result.contains("fn main"));
}

#[test]
fn test_extract_ftl_with_comments() {
    let response = "```ftl\n// Hello World\nT:a1 = unit\nentry: K:f1\n```";
    let result = extract_ftl(response).unwrap();
    assert!(result.contains("// Hello World"));
}

#[test]
fn test_extract_ftl_preserves_whitespace() {
    let response = "```ftl\nT:a1 = integer { bits: 32, signed: true }\n\nR:b1 = region { lifetime: static }\n\nentry: K:f1\n```";
    let result = extract_ftl(response).unwrap();
    assert!(result.contains("T:a1 = integer { bits: 32, signed: true }"));
    assert!(result.contains("R:b1 = region { lifetime: static }"));
}

// ---------------------------------------------------------------------------
// Pipeline result tests
// ---------------------------------------------------------------------------

#[test]
fn test_pipeline_result_valid_hello_world() {
    let ftl = std::fs::read_to_string("testdata/hello_world.ftl").unwrap();
    let result = run_pipeline(&ftl);

    assert!(result.parse_ok, "Hello world should parse OK");
    // Note: may have validation warnings but not fatal errors for the hello world example
}

#[test]
fn test_pipeline_result_parse_error() {
    let ftl = "THIS IS NOT VALID FTL!!!";
    let result = run_pipeline(ftl);

    assert!(!result.parse_ok, "Invalid input should fail parsing");
    assert!(!result.parse_errors.is_empty(), "Should have parse errors");
    assert!(result.feedback.is_some(), "Should generate feedback even on parse error");
}

#[test]
fn test_pipeline_result_empty_input() {
    let result = run_pipeline("");

    // Empty input either fails to parse or fails validation (no entry point).
    // Either way it should not be a full success.
    assert!(!result.is_success(), "Empty input should not be a full success");
}

#[test]
fn test_pipeline_result_is_success_logic() {
    // We can only test this indirectly through run_pipeline with valid FTL
    let ftl = std::fs::read_to_string("testdata/hello_world.ftl").unwrap();
    let result = run_pipeline(&ftl);

    // is_success requires: parse_ok && validation_ok && compiled && no disproven
    if result.parse_ok && result.validation_ok && result.compiled {
        let has_disproven = result
            .proof_results
            .iter()
            .any(|r| r.status == flux_ftl::prover::ProofStatus::Disproven);
        assert_eq!(result.is_success(), !has_disproven);
    }
}

#[test]
fn test_pipeline_proof_summary() {
    let ftl = std::fs::read_to_string("testdata/hello_world.ftl").unwrap();
    let result = run_pipeline(&ftl);

    let summary = result.proof_summary();
    // Each proof result should have a summary entry
    assert_eq!(summary.len(), result.proof_results.len());
}

// ---------------------------------------------------------------------------
// RequirementType tests
// ---------------------------------------------------------------------------

#[test]
fn test_requirement_type_parse_all_variants() {
    assert_eq!(
        RequirementType::from_str_loose("translate").unwrap(),
        RequirementType::Translate
    );
    assert_eq!(
        RequirementType::from_str_loose("t").unwrap(),
        RequirementType::Translate
    );
    assert_eq!(
        RequirementType::from_str_loose("optimize").unwrap(),
        RequirementType::Optimize
    );
    assert_eq!(
        RequirementType::from_str_loose("o").unwrap(),
        RequirementType::Optimize
    );
    assert_eq!(
        RequirementType::from_str_loose("invent").unwrap(),
        RequirementType::Invent
    );
    assert_eq!(
        RequirementType::from_str_loose("i").unwrap(),
        RequirementType::Invent
    );
    assert_eq!(
        RequirementType::from_str_loose("discover").unwrap(),
        RequirementType::Discover
    );
    assert_eq!(
        RequirementType::from_str_loose("d").unwrap(),
        RequirementType::Discover
    );
}

#[test]
fn test_requirement_type_case_insensitive() {
    assert_eq!(
        RequirementType::from_str_loose("TRANSLATE").unwrap(),
        RequirementType::Translate
    );
    assert_eq!(
        RequirementType::from_str_loose("Optimize").unwrap(),
        RequirementType::Optimize
    );
}

#[test]
fn test_requirement_type_invalid() {
    assert!(RequirementType::from_str_loose("unknown").is_err());
    assert!(RequirementType::from_str_loose("").is_err());
}

#[test]
fn test_requirement_type_instruction() {
    assert!(RequirementType::Translate.instruction().contains("TRANSLATE"));
    assert!(RequirementType::Optimize.instruction().contains("OPTIMIZE"));
    assert!(RequirementType::Invent.instruction().contains("INVENT"));
    assert!(RequirementType::Discover.instruction().contains("DISCOVER"));
}

// ---------------------------------------------------------------------------
// LlmConfig tests
// ---------------------------------------------------------------------------

#[test]
fn test_llm_config_defaults() {
    let config = LlmConfig::new(
        LlmProvider::Anthropic,
        "test-key".to_string(),
        "test-model".to_string(),
    );

    assert_eq!(config.max_tokens, 4096);
    assert!((config.temperature - 0.7).abs() < f64::EPSILON);
    assert_eq!(config.max_iterations, 5);
    assert_eq!(config.timeout_secs, 60);
    assert!(config.base_url.contains("anthropic"));
}

#[test]
fn test_llm_config_openai_defaults() {
    let config = LlmConfig::new(
        LlmProvider::OpenAi,
        "sk-test".to_string(),
        "gpt-4o".to_string(),
    );

    assert!(config.base_url.contains("openai"));
}

#[test]
fn test_llm_config_from_env_missing_key() {
    // Ensure the env var is not set for this test
    // SAFETY: This test is single-threaded and only removes a test-specific variable.
    unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
    let result = LlmConfig::from_env(LlmProvider::Anthropic, None);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// LlmProvider tests
// ---------------------------------------------------------------------------

#[test]
fn test_provider_from_str_loose() {
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
    assert_eq!(
        LlmProvider::from_str_loose("local").unwrap(),
        LlmProvider::OpenAi
    );
    assert!(LlmProvider::from_str_loose("gemini").is_err());
}

#[test]
fn test_provider_api_key_env() {
    assert_eq!(LlmProvider::Anthropic.api_key_env(), "ANTHROPIC_API_KEY");
    assert_eq!(LlmProvider::OpenAi.api_key_env(), "OPENAI_API_KEY");
}

#[test]
fn test_provider_default_base_url() {
    assert!(LlmProvider::Anthropic.default_base_url().contains("anthropic"));
    assert!(LlmProvider::OpenAi.default_base_url().contains("openai"));
}

// ---------------------------------------------------------------------------
// Message serialization tests
// ---------------------------------------------------------------------------

#[test]
fn test_message_serialization() {
    let msg = Message::user("Hello, world!");
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"role\":\"user\""));
    assert!(json.contains("Hello, world!"));
}

#[test]
fn test_message_deserialization() {
    let json = r#"{"role":"assistant","content":"Here is FTL..."}"#;
    let msg: Message = serde_json::from_str(json).unwrap();
    assert_eq!(msg.role, "assistant");
    assert_eq!(msg.content, "Here is FTL...");
}

// ---------------------------------------------------------------------------
// GenerationResult serialization tests
// ---------------------------------------------------------------------------

#[test]
fn test_generation_result_serialization() {
    let result = GenerationResult {
        ftl_source: "T:a1 = unit\nentry: K:f1".to_string(),
        iterations: 2,
        final_status: GenerationStatus::Success,
        history: vec![
            IterationRecord {
                iteration: 1,
                ftl_source: "invalid".to_string(),
                parse_ok: false,
                validation_errors: 0,
                proof_summary: Vec::new(),
                feedback_status: "Fatal".to_string(),
            },
            IterationRecord {
                iteration: 2,
                ftl_source: "T:a1 = unit\nentry: K:f1".to_string(),
                parse_ok: true,
                validation_errors: 0,
                proof_summary: vec!["V:e1: Proven".to_string()],
                feedback_status: "Pass".to_string(),
            },
        ],
    };

    let json = serde_json::to_string_pretty(&result).unwrap();
    assert!(json.contains("\"final_status\": \"SUCCESS\""), "JSON: {}", json);
    assert!(json.contains("\"iterations\": 2"), "JSON: {}", json);
    assert!(json.contains("V:e1: Proven"), "JSON: {}", json);
}

#[test]
fn test_generation_status_variants() {
    let statuses = [
        GenerationStatus::Success,
        GenerationStatus::PartialSuccess,
        GenerationStatus::MaxIterations,
        GenerationStatus::Failed,
    ];

    for status in &statuses {
        let json = serde_json::to_string(status).unwrap();
        assert!(!json.is_empty());
    }
}

// ---------------------------------------------------------------------------
// IterationRecord tests
// ---------------------------------------------------------------------------

#[test]
fn test_iteration_record_creation() {
    let record = IterationRecord {
        iteration: 1,
        ftl_source: "T:a1 = unit".to_string(),
        parse_ok: true,
        validation_errors: 3,
        proof_summary: vec!["V:e1: Proven".to_string(), "V:e2: Unknown".to_string()],
        feedback_status: "Fixable".to_string(),
    };

    assert_eq!(record.iteration, 1);
    assert!(record.parse_ok);
    assert_eq!(record.validation_errors, 3);
    assert_eq!(record.proof_summary.len(), 2);
    assert_eq!(record.feedback_status, "Fixable");
}

#[test]
fn test_iteration_record_serialization() {
    let record = IterationRecord {
        iteration: 3,
        ftl_source: "entry: K:f1".to_string(),
        parse_ok: false,
        validation_errors: 0,
        proof_summary: Vec::new(),
        feedback_status: "Fatal".to_string(),
    };

    let json = serde_json::to_string(&record).unwrap();
    assert!(json.contains("\"iteration\":3"));
    assert!(json.contains("\"parse_ok\":false"));
}
