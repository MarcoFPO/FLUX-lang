use serde::Serialize;

use crate::error::ParseError;
use crate::prover::{ProofResult, ProofStatus};

// ---------------------------------------------------------------------------
// Public types — structured LLM feedback
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct LlmFeedback {
    pub status: FeedbackStatus,
    pub summary: String,
    pub issues: Vec<FeedbackIssue>,
    pub iteration_hint: IterationHint,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FeedbackStatus {
    Pass,
    Fixable,
    Fatal,
}

#[derive(Debug, Serialize)]
pub struct FeedbackIssue {
    pub severity: Severity,
    pub category: IssueCategory,
    pub node_id: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<FeedbackSuggestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<IssueContext>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IssueCategory {
    ParseError,
    StructuralValidation,
    TypeMismatch,
    EffectViolation,
    RegionError,
    ContractViolation,
    ProofFailure,
}

#[derive(Debug, Serialize)]
pub struct FeedbackSuggestion {
    pub action: SuggestionAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_node: Option<String>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionAction {
    Replace,
    Add,
    Remove,
    Modify,
    Restructure,
}

#[derive(Debug, Serialize)]
pub struct IssueContext {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related_nodes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IterationHint {
    pub estimated_fixes: usize,
    pub priority_order: Vec<String>,
    pub strategy: String,
}

// ---------------------------------------------------------------------------
// ValidationError — input type the feedback module consumes from main.rs
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ValidationError {
    pub error_code: u32,
    pub node_id: String,
    pub violation: String,
    pub message: String,
    pub suggestion: Option<String>,
}

// ---------------------------------------------------------------------------
// Feedback generation
// ---------------------------------------------------------------------------

/// Generate structured LLM feedback from parse errors, validation errors,
/// and proof results. The output is designed to be actionable: an LLM can
/// read the JSON and produce a corrected FTL source.
pub fn generate_feedback(
    parse_errors: &[ParseError],
    validation_errors: &[ValidationError],
    proof_results: &[ProofResult],
) -> LlmFeedback {
    let mut issues = Vec::new();
    let mut priority_order = Vec::new();

    // --- Phase 1: Parse errors (highest priority) ---
    for pe in parse_errors {
        let node_id = format!("parse:L{}:C{}", pe.line, pe.column);
        priority_order.push(node_id.clone());

        let suggestion = build_parse_suggestion(pe);

        issues.push(FeedbackIssue {
            severity: Severity::Error,
            category: IssueCategory::ParseError,
            node_id,
            message: pe.message.clone(),
            suggestion: Some(suggestion),
            context: Some(IssueContext {
                related_nodes: Vec::new(),
                expected: None,
                actual: Some(format!("line {}, column {}", pe.line, pe.column)),
            }),
        });
    }

    // --- Phase 2: Validation errors ---
    for ve in validation_errors {
        let issue = convert_validation_error(ve);
        // Only add to priority if it's an error (not warning/info)
        if issue.severity == Severity::Error {
            priority_order.push(issue.node_id.clone());
        }
        issues.push(issue);
    }

    // --- Phase 3: Proof results ---
    for pr in proof_results {
        if let Some(issue) = convert_proof_result(pr) {
            priority_order.push(issue.node_id.clone());
            issues.push(issue);
        }
    }

    // --- Determine overall status ---
    let status = determine_status(&issues, parse_errors);

    // --- Build summary ---
    let summary = build_summary(&issues, &status);

    // --- Build iteration hint ---
    let estimated_fixes = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();

    // Deduplicate priority_order while preserving order
    let priority_order = deduplicate(priority_order);

    let strategy = build_strategy(&issues);

    let iteration_hint = IterationHint {
        estimated_fixes,
        priority_order,
        strategy,
    };

    LlmFeedback {
        status,
        summary,
        issues,
        iteration_hint,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn build_parse_suggestion(pe: &ParseError) -> FeedbackSuggestion {
    let msg = &pe.message;

    // Try to provide specific suggestions based on common parse error patterns
    if msg.contains("expected") {
        let description = format!(
            "Fix syntax at line {}, column {}: {}",
            pe.line, pe.column, msg
        );
        FeedbackSuggestion {
            action: SuggestionAction::Modify,
            target_node: None,
            description,
            example: None,
        }
    } else {
        FeedbackSuggestion {
            action: SuggestionAction::Restructure,
            target_node: None,
            description: format!(
                "Review and fix syntax near line {}, column {}: {}",
                pe.line, pe.column, msg
            ),
            example: None,
        }
    }
}

fn convert_validation_error(ve: &ValidationError) -> FeedbackIssue {
    let code = ve.error_code;

    let (severity, category) = classify_error_code(code);

    let suggestion = build_validation_suggestion(ve, &category);

    let context = IssueContext {
        related_nodes: Vec::new(),
        expected: None,
        actual: Some(ve.violation.clone()),
    };

    FeedbackIssue {
        severity,
        category,
        node_id: ve.node_id.clone(),
        message: ve.message.clone(),
        suggestion: Some(suggestion),
        context: Some(context),
    }
}

fn classify_error_code(code: u32) -> (Severity, IssueCategory) {
    match code {
        1000..=1999 => (Severity::Error, IssueCategory::StructuralValidation),
        2000..=2999 => (Severity::Error, IssueCategory::TypeMismatch),
        3000..=3999 => (Severity::Error, IssueCategory::EffectViolation),
        4000..=4999 => (Severity::Warning, IssueCategory::ContractViolation),
        5000..=5999 => (Severity::Error, IssueCategory::EffectViolation),
        6000..=6999 => (Severity::Error, IssueCategory::RegionError),
        _ => (Severity::Error, IssueCategory::StructuralValidation),
    }
}

fn build_validation_suggestion(
    ve: &ValidationError,
    category: &IssueCategory,
) -> FeedbackSuggestion {
    // Use the existing suggestion from the validator if available
    let base_description = ve
        .suggestion
        .clone()
        .unwrap_or_else(|| ve.message.clone());

    match category {
        IssueCategory::StructuralValidation => FeedbackSuggestion {
            action: SuggestionAction::Modify,
            target_node: Some(ve.node_id.clone()),
            description: base_description,
            example: build_structural_example(ve),
        },
        IssueCategory::TypeMismatch => FeedbackSuggestion {
            action: SuggestionAction::Replace,
            target_node: Some(ve.node_id.clone()),
            description: format!(
                "Fix type mismatch on {}: {}",
                ve.node_id, base_description
            ),
            example: None,
        },
        IssueCategory::EffectViolation => FeedbackSuggestion {
            action: SuggestionAction::Add,
            target_node: Some(ve.node_id.clone()),
            description: format!(
                "Add missing effect declaration on {}: {}",
                ve.node_id, base_description
            ),
            example: None,
        },
        IssueCategory::RegionError => build_region_suggestion(ve),
        IssueCategory::ContractViolation => FeedbackSuggestion {
            action: SuggestionAction::Modify,
            target_node: Some(ve.node_id.clone()),
            description: base_description,
            example: None,
        },
        _ => FeedbackSuggestion {
            action: SuggestionAction::Modify,
            target_node: Some(ve.node_id.clone()),
            description: base_description,
            example: None,
        },
    }
}

fn build_structural_example(ve: &ValidationError) -> Option<String> {
    let code = ve.error_code;
    match code {
        1001 => Some("Ensure each node has a unique ID, e.g. C:c1, C:c2".to_string()),
        1002 => Some(
            "Reference only defined nodes, e.g. inputs: [C:c1] where C:c1 is defined".to_string(),
        ),
        1003 => Some("Add an entry point: entry: K:f1".to_string()),
        1004 => Some("Ensure entry references a defined K-node: entry: K:f1".to_string()),
        _ => None,
    }
}

fn build_region_suggestion(ve: &ValidationError) -> FeedbackSuggestion {
    let code = ve.error_code;
    match code {
        6001 => FeedbackSuggestion {
            action: SuggestionAction::Modify,
            target_node: Some(ve.node_id.clone()),
            description: format!(
                "Region parent references undefined region on {}: {}",
                ve.node_id,
                ve.suggestion.clone().unwrap_or_default()
            ),
            example: Some(
                "R:b2 = region { lifetime: scoped, parent: R:b1 } where R:b1 is defined"
                    .to_string(),
            ),
        },
        6002 => FeedbackSuggestion {
            action: SuggestionAction::Restructure,
            target_node: Some(ve.node_id.clone()),
            description: format!(
                "Break region cycle involving {}: {}",
                ve.node_id,
                ve.suggestion.clone().unwrap_or_default()
            ),
            example: Some(
                "Ensure region parents form a tree: R:b1 (static) <- R:b2 (scoped) <- R:b3 (scoped)"
                    .to_string(),
            ),
        },
        6003 => FeedbackSuggestion {
            action: SuggestionAction::Modify,
            target_node: Some(ve.node_id.clone()),
            description: format!(
                "Static region must not have a parent on {}",
                ve.node_id
            ),
            example: Some("R:b1 = region { lifetime: static }".to_string()),
        },
        6004 => FeedbackSuggestion {
            action: SuggestionAction::Add,
            target_node: Some(ve.node_id.clone()),
            description: format!(
                "Scoped region {} needs a parent region",
                ve.node_id
            ),
            example: Some(
                "R:b2 = region { lifetime: scoped, parent: R:b1 }".to_string(),
            ),
        },
        6005 => FeedbackSuggestion {
            action: SuggestionAction::Modify,
            target_node: Some(ve.node_id.clone()),
            description: format!(
                "Fix region reference on {}: {}",
                ve.node_id,
                ve.suggestion.clone().unwrap_or_default()
            ),
            example: Some(
                "Ensure allocation references a defined region: M:g1 = alloc { type: T:a1, region: R:b1 }"
                    .to_string(),
            ),
        },
        6006 => FeedbackSuggestion {
            action: SuggestionAction::Restructure,
            target_node: Some(ve.node_id.clone()),
            description: format!(
                "Region escape detected at {}: data from a shorter-lived region flows into a longer-lived one. {}",
                ve.node_id,
                ve.suggestion.clone().unwrap_or_default()
            ),
            example: Some(
                "Ensure store targets have equal or shorter lifetime than value sources"
                    .to_string(),
            ),
        },
        _ => FeedbackSuggestion {
            action: SuggestionAction::Modify,
            target_node: Some(ve.node_id.clone()),
            description: ve.suggestion.clone().unwrap_or_else(|| ve.message.clone()),
            example: None,
        },
    }
}

fn convert_proof_result(pr: &ProofResult) -> Option<FeedbackIssue> {
    match pr.status {
        ProofStatus::Proven | ProofStatus::Assumed => None,
        ProofStatus::Disproven => {
            let counterexample_info = pr
                .counterexample
                .as_ref()
                .map(|ce| format!(" Counterexample: {}", ce))
                .unwrap_or_default();

            Some(FeedbackIssue {
                severity: Severity::Error,
                category: IssueCategory::ProofFailure,
                node_id: pr.contract_id.clone(),
                message: format!(
                    "Contract {} ({} clause, index {}) is DISPROVEN for target {}.{}",
                    pr.contract_id,
                    pr.clause_kind,
                    pr.clause_index,
                    pr.target_id,
                    counterexample_info
                ),
                suggestion: Some(FeedbackSuggestion {
                    action: SuggestionAction::Modify,
                    target_node: Some(pr.target_id.clone()),
                    description: format!(
                        "Strengthen the {} clause of contract {} or fix the compute node {} so the property holds",
                        pr.clause_kind, pr.contract_id, pr.target_id
                    ),
                    example: Some(format!(
                        "V:eN = contract {{ target: {}, {}: <corrected_expression> }}",
                        pr.target_id, pr.clause_kind
                    )),
                }),
                context: Some(IssueContext {
                    related_nodes: vec![pr.target_id.clone()],
                    expected: Some("PROVEN".to_string()),
                    actual: Some(format!("DISPROVEN{}", counterexample_info)),
                }),
            })
        }
        ProofStatus::Unknown => Some(FeedbackIssue {
            severity: Severity::Warning,
            category: IssueCategory::ProofFailure,
            node_id: pr.contract_id.clone(),
            message: format!(
                "Contract {} ({} clause, index {}) could not be proven or disproven for target {}",
                pr.contract_id, pr.clause_kind, pr.clause_index, pr.target_id
            ),
            suggestion: Some(FeedbackSuggestion {
                action: SuggestionAction::Modify,
                target_node: Some(pr.contract_id.clone()),
                description: format!(
                    "Simplify the {} clause of contract {} or add assume clauses to help the prover",
                    pr.clause_kind, pr.contract_id
                ),
                example: None,
            }),
            context: Some(IssueContext {
                related_nodes: vec![pr.target_id.clone()],
                expected: Some("PROVEN".to_string()),
                actual: Some("UNKNOWN".to_string()),
            }),
        }),
        ProofStatus::Timeout => Some(FeedbackIssue {
            severity: Severity::Warning,
            category: IssueCategory::ProofFailure,
            node_id: pr.contract_id.clone(),
            message: format!(
                "Prover timed out on contract {} ({} clause, index {}) for target {}",
                pr.contract_id, pr.clause_kind, pr.clause_index, pr.target_id
            ),
            suggestion: Some(FeedbackSuggestion {
                action: SuggestionAction::Restructure,
                target_node: Some(pr.contract_id.clone()),
                description: format!(
                    "Split the complex invariant in contract {} into simpler, independently provable clauses",
                    pr.contract_id
                ),
                example: None,
            }),
            context: Some(IssueContext {
                related_nodes: vec![pr.target_id.clone()],
                expected: Some("PROVEN".to_string()),
                actual: Some("TIMEOUT".to_string()),
            }),
        }),
    }
}

fn determine_status(issues: &[FeedbackIssue], parse_errors: &[ParseError]) -> FeedbackStatus {
    if !parse_errors.is_empty() {
        return FeedbackStatus::Fatal;
    }

    let has_fatal_structural = issues.iter().any(|i| {
        i.severity == Severity::Error
            && matches!(
                i.category,
                IssueCategory::StructuralValidation
                    | IssueCategory::RegionError
                    | IssueCategory::EffectViolation
            )
    });

    if has_fatal_structural {
        return FeedbackStatus::Fatal;
    }

    let has_errors = issues.iter().any(|i| i.severity == Severity::Error);

    if has_errors {
        return FeedbackStatus::Fixable;
    }

    // Warnings only (e.g. UNKNOWN proofs) or no issues at all
    if issues.is_empty() {
        FeedbackStatus::Pass
    } else {
        // Only warnings/info remain
        let has_warnings = issues.iter().any(|i| i.severity == Severity::Warning);
        if has_warnings {
            FeedbackStatus::Fixable
        } else {
            FeedbackStatus::Pass
        }
    }
}

fn build_summary(issues: &[FeedbackIssue], status: &FeedbackStatus) -> String {
    match status {
        FeedbackStatus::Pass => "Program is valid and all contracts are verified.".to_string(),
        FeedbackStatus::Fatal => {
            let parse_count = issues
                .iter()
                .filter(|i| i.category == IssueCategory::ParseError)
                .count();
            let structural_count = issues
                .iter()
                .filter(|i| i.category == IssueCategory::StructuralValidation)
                .count();

            if parse_count > 0 {
                format!(
                    "Fatal: {} parse error(s) prevent further analysis. Fix syntax first.",
                    parse_count
                )
            } else {
                format!(
                    "Fatal: {} structural validation error(s) found. Fix graph structure first.",
                    structural_count
                )
            }
        }
        FeedbackStatus::Fixable => {
            let error_count = issues.iter().filter(|i| i.severity == Severity::Error).count();
            let warning_count = issues
                .iter()
                .filter(|i| i.severity == Severity::Warning)
                .count();

            let mut parts = Vec::new();
            if error_count > 0 {
                parts.push(format!("{} error(s)", error_count));
            }
            if warning_count > 0 {
                parts.push(format!("{} warning(s)", warning_count));
            }
            format!(
                "Fixable: {} found. Follow suggestions to repair.",
                parts.join(" and ")
            )
        }
    }
}

fn build_strategy(issues: &[FeedbackIssue]) -> String {
    let has_parse = issues
        .iter()
        .any(|i| i.category == IssueCategory::ParseError);
    let has_structural = issues
        .iter()
        .any(|i| i.category == IssueCategory::StructuralValidation);
    let has_type = issues
        .iter()
        .any(|i| i.category == IssueCategory::TypeMismatch);
    let has_effect = issues
        .iter()
        .any(|i| i.category == IssueCategory::EffectViolation);
    let has_region = issues
        .iter()
        .any(|i| i.category == IssueCategory::RegionError);
    let has_proof = issues
        .iter()
        .any(|i| i.category == IssueCategory::ProofFailure);

    if has_parse {
        return "Fix parse errors first — nothing else can be validated until syntax is correct."
            .to_string();
    }

    let mut steps = Vec::new();
    if has_structural {
        steps.push("structural errors");
    }
    if has_type {
        steps.push("type mismatches");
    }
    if has_effect {
        steps.push("effect violations");
    }
    if has_region {
        steps.push("region errors");
    }
    if has_proof {
        steps.push("contract/proof failures");
    }

    if steps.is_empty() {
        "No fixes needed.".to_string()
    } else if steps.len() == 1 {
        format!("Fix {} to resolve all issues.", steps[0])
    } else {
        let last = steps.pop().unwrap_or_default();
        format!("Fix {} first, then {}.", steps.join(", then "), last)
    }
}

fn deduplicate(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for item in items {
        if seen.insert(item.clone()) {
            result.push(item);
        }
    }
    result
}
