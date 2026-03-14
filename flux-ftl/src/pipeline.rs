// ---------------------------------------------------------------------------
// pipeline.rs — Shared check/compile pipeline used by CLI and MCP server
// ---------------------------------------------------------------------------

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use crate::ast::Program;
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

    run_check_program_with_bmc(ast, bmc_config)
}

/// Run the full check pipeline on an already-parsed Program (e.g. after import
/// resolution).
pub fn run_check_program(ast: Program) -> FullResult {
    run_check_program_with_bmc(ast, None)
}

/// Run the full check pipeline on an already-parsed Program with optional BMC.
pub fn run_check_program_with_bmc(ast: Program, bmc_config: Option<BmcConfig>) -> FullResult {
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
// Import resolution
// ---------------------------------------------------------------------------

/// Resolve all imports in a parsed program, merging imported nodes into the
/// main program.  Returns errors for circular imports, missing files, and
/// duplicate node IDs across modules.
pub fn resolve_imports(program: &Program, base_path: &Path) -> Result<Program, Vec<String>> {
    let mut visited: HashSet<String> = HashSet::new();
    let canonical = base_path
        .canonicalize()
        .map_err(|e| vec![format!("cannot canonicalize base path: {}", e)])?;
    let canonical_str = canonical.to_string_lossy().to_string();
    visited.insert(canonical_str);

    let base_dir = canonical
        .parent()
        .ok_or_else(|| vec!["base path has no parent directory".to_string()])?;

    resolve_imports_recursive(program, base_dir, &mut visited)
}

fn resolve_imports_recursive(
    program: &Program,
    base_dir: &Path,
    visited: &mut HashSet<String>,
) -> Result<Program, Vec<String>> {
    let mut merged = program.clone();
    // Clear imports in the merged result — they have been resolved
    merged.imports = Vec::new();

    for import_path in &program.imports {
        let full_path = base_dir.join(import_path);
        let canonical = full_path
            .canonicalize()
            .map_err(|e| vec![format!("import '{}': {}", import_path, e)])?;
        let canonical_str = canonical.to_string_lossy().to_string();

        // Check circular imports
        if visited.contains(&canonical_str) {
            return Err(vec![format!(
                "circular import detected: '{}'",
                import_path
            )]);
        }
        visited.insert(canonical_str);

        // Read and parse the imported file
        let source = std::fs::read_to_string(&full_path)
            .map_err(|e| vec![format!("import '{}': {}", import_path, e)])?;

        let parse_result = parse_ftl(&source);
        let imported = match parse_result.status {
            Status::Ok => match parse_result.ast {
                Some(ast) => ast,
                None => return Err(vec![format!("import '{}': no AST produced", import_path)]),
            },
            Status::Error => {
                let msgs: Vec<String> = parse_result
                    .errors
                    .iter()
                    .map(|e| format!("import '{}' line {}: {}", import_path, e.line, e.message))
                    .collect();
                return Err(msgs);
            }
        };

        // Recursively resolve imports in the imported module
        let import_dir = full_path
            .parent()
            .ok_or_else(|| vec![format!("import '{}': no parent directory", import_path)])?;
        let resolved = resolve_imports_recursive(&imported, import_dir, visited)?;

        // Check for duplicate node IDs before merging
        let mut errors = Vec::new();
        check_duplicate_ids(&merged, &resolved, import_path, &mut errors);
        if !errors.is_empty() {
            return Err(errors);
        }

        // Merge all nodes from the resolved import into our program
        merged.types.extend(resolved.types);
        merged.regions.extend(resolved.regions);
        merged.computes.extend(resolved.computes);
        merged.effects.extend(resolved.effects);
        merged.controls.extend(resolved.controls);
        merged.contracts.extend(resolved.contracts);
        merged.memories.extend(resolved.memories);
        merged.externs.extend(resolved.externs);
    }

    Ok(merged)
}

/// Check for duplicate node IDs between the main program and an import.
fn check_duplicate_ids(
    main: &Program,
    imported: &Program,
    import_path: &str,
    errors: &mut Vec<String>,
) {
    let mut existing: HashSet<String> = HashSet::new();
    for t in &main.types { existing.insert(t.id.0.clone()); }
    for r in &main.regions { existing.insert(r.id.0.clone()); }
    for c in &main.computes { existing.insert(c.id.0.clone()); }
    for e in &main.effects { existing.insert(e.id.0.clone()); }
    for k in &main.controls { existing.insert(k.id.0.clone()); }
    for v in &main.contracts { existing.insert(v.id.0.clone()); }
    for m in &main.memories { existing.insert(m.id.0.clone()); }
    for x in &main.externs { existing.insert(x.id.0.clone()); }

    let check = |id: &str| {
        if existing.contains(id) {
            Some(format!(
                "duplicate node ID '{}' from import '{}'",
                id, import_path
            ))
        } else {
            None
        }
    };

    for t in &imported.types { if let Some(e) = check(&t.id.0) { errors.push(e); } }
    for r in &imported.regions { if let Some(e) = check(&r.id.0) { errors.push(e); } }
    for c in &imported.computes { if let Some(e) = check(&c.id.0) { errors.push(e); } }
    for e_def in &imported.effects { if let Some(e) = check(&e_def.id.0) { errors.push(e); } }
    for k in &imported.controls { if let Some(e) = check(&k.id.0) { errors.push(e); } }
    for v in &imported.contracts { if let Some(e) = check(&v.id.0) { errors.push(e); } }
    for m in &imported.memories { if let Some(e) = check(&m.id.0) { errors.push(e); } }
    for x in &imported.externs { if let Some(e) = check(&x.id.0) { errors.push(e); } }
}

/// Serialize a FullResult to JSON string.
pub fn result_to_json(result: &FullResult) -> Result<String, String> {
    serde_json::to_string(result).map_err(|e| format!("JSON serialization: {}", e))
}
