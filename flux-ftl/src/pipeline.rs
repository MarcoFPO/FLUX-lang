// ---------------------------------------------------------------------------
// pipeline.rs — Shared check/compile pipeline used by CLI and MCP server
// ---------------------------------------------------------------------------

use serde::Serialize;

use crate::compiler::{self, CompileMetadata};
use crate::error::Status;
use crate::feedback::{self, LlmFeedback, ValidationError as FeedbackValidationError};
use crate::parser::parse_ftl;
use crate::prover::{prove_contracts, BmcConfig, ProofResult, ProofStatus, ProverConfig};
use crate::region_checker::check_regions;
use crate::type_checker::check_types_and_effects;
use crate::validator::validate;

// ---------------------------------------------------------------------------
// Public result types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct FullResult {
    pub status: FullStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ast: Option<crate::ast::Program>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parse_errors: Vec<crate::error::ParseError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub validation_errors: Vec<GenericError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub proof_results: Vec<ProofResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiled: Option<CompileMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compile_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<LlmFeedback>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FullStatus {
    Ok,
    ParseError,
    ValidationFail,
    ProofFail,
}

#[derive(Debug, Serialize)]
pub struct GenericError {
    pub error_code: u32,
    pub node_id: String,
    pub violation: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

// ---------------------------------------------------------------------------
// Pipeline functions
// ---------------------------------------------------------------------------

/// Run the full check pipeline without BMC.
pub fn run_check(input: &str) -> FullResult {
    run_check_with_bmc(input, None)
}

/// Run the full check pipeline with optional BMC configuration.
pub fn run_check_with_bmc(input: &str, bmc_config: Option<BmcConfig>) -> FullResult {
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

/// Serialize a FullResult to JSON string.
pub fn result_to_json(result: &FullResult) -> Result<String, String> {
    serde_json::to_string(result).map_err(|e| format!("JSON serialization: {}", e))
}
