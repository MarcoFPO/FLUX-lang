use flux_ftl::feedback::{
    generate_feedback, FeedbackStatus, IssueCategory, Severity, SuggestionAction,
    ValidationError,
};
use flux_ftl::parser::parse_ftl;
use flux_ftl::prover::{prove_contracts, ProverConfig};
use flux_ftl::region_checker::check_regions;
use flux_ftl::type_checker::check_types_and_effects;
use flux_ftl::validator::validate;

// ===========================================================================
// Helpers
// ===========================================================================

/// Run the full pipeline and return feedback for a valid FTL program.
fn feedback_for(input: &str) -> flux_ftl::feedback::LlmFeedback {
    let parse_result = parse_ftl(input);
    match parse_result.status {
        flux_ftl::error::Status::Error => {
            return generate_feedback(&parse_result.errors, &[], &[]);
        }
        flux_ftl::error::Status::Ok => {}
    }

    let ast = parse_result.ast.unwrap();

    let mut validation_errors = Vec::new();

    // Structural validation
    let vr = validate(&ast);
    for e in &vr.errors {
        validation_errors.push(ValidationError {
            error_code: e.error_code,
            node_id: e.node_id.clone(),
            violation: e.violation.clone(),
            message: e.message.clone(),
            suggestion: e.suggestion.clone(),
        });
    }
    for w in &vr.warnings {
        validation_errors.push(ValidationError {
            error_code: w.error_code,
            node_id: w.node_id.clone(),
            violation: w.violation.clone(),
            message: w.message.clone(),
            suggestion: w.suggestion.clone(),
        });
    }

    // Type and effect checks
    for e in check_types_and_effects(&ast) {
        validation_errors.push(ValidationError {
            error_code: e.error_code,
            node_id: e.node_id,
            violation: e.violation,
            message: e.message,
            suggestion: e.suggestion,
        });
    }

    // Region checks
    for e in check_regions(&ast) {
        validation_errors.push(ValidationError {
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

    let proof_results = if !has_fatal {
        let config = ProverConfig::default();
        prove_contracts(&ast, &config)
    } else {
        Vec::new()
    };

    generate_feedback(&[], &validation_errors, &proof_results)
}

// ===========================================================================
// 1. Valid program → PASS feedback
// ===========================================================================

#[test]
fn valid_program_pass_feedback() {
    let input = std::fs::read_to_string("testdata/hello_world.ftl").unwrap();
    let fb = feedback_for(&input);

    assert_eq!(fb.status, FeedbackStatus::Pass);
    assert!(fb.issues.is_empty(), "no issues for valid program");
    assert_eq!(fb.iteration_hint.estimated_fixes, 0);
    assert!(
        fb.summary.contains("valid"),
        "summary should mention valid: {}",
        fb.summary
    );
}

// ===========================================================================
// 2. Parse error → FATAL feedback with ParseError issues
// ===========================================================================

#[test]
fn parse_error_feedback() {
    let input = "T:a1 = integer { bits: 32, signed: true\nentry: K:f1";
    let fb = feedback_for(input);

    assert_eq!(fb.status, FeedbackStatus::Fatal);
    assert!(
        !fb.issues.is_empty(),
        "should have at least one parse error issue"
    );
    assert!(fb.issues.iter().all(|i| i.category == IssueCategory::ParseError));
    assert!(fb.issues.iter().all(|i| i.severity == Severity::Error));

    // Summary should mention parse errors
    assert!(
        fb.summary.contains("parse error"),
        "summary should mention parse error: {}",
        fb.summary
    );

    // Strategy should prioritize parse fixes
    assert!(
        fb.iteration_hint.strategy.contains("parse"),
        "strategy should mention parse: {}",
        fb.iteration_hint.strategy
    );
}

// ===========================================================================
// 3. Validation error → FIXABLE/FATAL feedback with structured issues
// ===========================================================================

#[test]
fn validation_error_feedback_region_cycle() {
    let input = std::fs::read_to_string("testdata/errors/region_cycle.ftl").unwrap();
    let fb = feedback_for(&input);

    // Region cycle (6002) is a fatal error (>= 3000)
    assert_eq!(fb.status, FeedbackStatus::Fatal);
    assert!(!fb.issues.is_empty());

    let region_issues: Vec<_> = fb
        .issues
        .iter()
        .filter(|i| i.category == IssueCategory::RegionError)
        .collect();
    assert!(
        !region_issues.is_empty(),
        "should have region error issues"
    );

    // Should have suggestions
    for issue in &region_issues {
        assert!(issue.suggestion.is_some(), "region issues should have suggestions");
    }
}

#[test]
fn validation_error_feedback_region_no_parent() {
    let input = std::fs::read_to_string("testdata/errors/region_no_parent.ftl").unwrap();
    let fb = feedback_for(&input);

    assert_eq!(fb.status, FeedbackStatus::Fatal);

    let region_issues: Vec<_> = fb
        .issues
        .iter()
        .filter(|i| i.category == IssueCategory::RegionError)
        .collect();
    assert!(!region_issues.is_empty());

    // 6004 should suggest adding a parent
    let issue_6004 = region_issues
        .iter()
        .find(|i| {
            i.suggestion
                .as_ref()
                .map(|s| s.action == SuggestionAction::Add)
                .unwrap_or(false)
        });
    assert!(
        issue_6004.is_some(),
        "should have an Add suggestion for missing parent"
    );
}

#[test]
fn validation_error_feedback_region_escape() {
    let input = std::fs::read_to_string("testdata/errors/region_escape.ftl").unwrap();
    let fb = feedback_for(&input);

    assert_eq!(fb.status, FeedbackStatus::Fatal);

    let region_issues: Vec<_> = fb
        .issues
        .iter()
        .filter(|i| i.category == IssueCategory::RegionError)
        .collect();
    assert!(!region_issues.is_empty());

    // 6006 should suggest restructuring
    let escape_issue = region_issues
        .iter()
        .find(|i| {
            i.suggestion
                .as_ref()
                .map(|s| s.action == SuggestionAction::Restructure)
                .unwrap_or(false)
        });
    assert!(
        escape_issue.is_some(),
        "should have a Restructure suggestion for region escape"
    );
}

// ===========================================================================
// 4. Proof failure → ProofFailure issues
// ===========================================================================

#[test]
fn proof_failure_feedback() {
    // concurrency.ftl has V:e2 DISPROVEN and V:e1 UNKNOWN
    let input = std::fs::read_to_string("testdata/concurrency.ftl").unwrap();
    let fb = feedback_for(&input);

    assert_eq!(fb.status, FeedbackStatus::Fixable);

    let proof_issues: Vec<_> = fb
        .issues
        .iter()
        .filter(|i| i.category == IssueCategory::ProofFailure)
        .collect();
    assert!(
        proof_issues.len() >= 2,
        "should have at least 2 proof issues (DISPROVEN + UNKNOWN), got {}",
        proof_issues.len()
    );

    // Check for DISPROVEN issue
    let disproven = proof_issues
        .iter()
        .find(|i| i.message.contains("DISPROVEN"));
    assert!(disproven.is_some(), "should have a DISPROVEN proof issue");
    let disproven = disproven.unwrap();
    assert_eq!(disproven.severity, Severity::Error);
    assert!(disproven.suggestion.is_some());

    // Check for UNKNOWN issue
    let unknown = proof_issues
        .iter()
        .find(|i| i.message.contains("not be proven"));
    assert!(unknown.is_some(), "should have an UNKNOWN proof issue");
    let unknown = unknown.unwrap();
    assert_eq!(unknown.severity, Severity::Warning);
}

#[test]
fn proof_failure_snake_game_disproven() {
    let input = std::fs::read_to_string("testdata/snake_game.ftl").unwrap();
    let fb = feedback_for(&input);

    // snake_game has 3 DISPROVEN post-conditions
    let disproven_issues: Vec<_> = fb
        .issues
        .iter()
        .filter(|i| {
            i.category == IssueCategory::ProofFailure
                && i.severity == Severity::Error
                && i.message.contains("DISPROVEN")
        })
        .collect();

    assert_eq!(
        disproven_issues.len(),
        3,
        "snake_game should have 3 disproven proof issues"
    );

    // Each should have context with related_nodes and expected/actual
    for issue in &disproven_issues {
        let ctx = issue.context.as_ref().unwrap();
        assert_eq!(ctx.expected.as_deref(), Some("PROVEN"));
        assert!(ctx.actual.as_ref().unwrap().contains("DISPROVEN"));
    }
}

// ===========================================================================
// 5. Iteration hint tests
// ===========================================================================

#[test]
fn feedback_has_iteration_hint() {
    // Parse error: highest priority
    let input = "GARBAGE {{{";
    let fb = feedback_for(input);
    assert_eq!(fb.status, FeedbackStatus::Fatal);
    assert!(fb.iteration_hint.estimated_fixes > 0);
    assert!(!fb.iteration_hint.priority_order.is_empty());
    assert!(
        fb.iteration_hint
            .strategy
            .contains("parse"),
        "strategy should prioritize parse errors: {}",
        fb.iteration_hint.strategy
    );
}

#[test]
fn feedback_iteration_hint_priority_order() {
    // concurrency.ftl: proof failures only (no parse/validation errors)
    let input = std::fs::read_to_string("testdata/concurrency.ftl").unwrap();
    let fb = feedback_for(&input);

    // Priority order should list contract IDs
    assert!(
        !fb.iteration_hint.priority_order.is_empty(),
        "priority_order should be non-empty for programs with issues"
    );

    // Strategy should mention contract/proof failures
    assert!(
        fb.iteration_hint.strategy.contains("contract")
            || fb.iteration_hint.strategy.contains("proof"),
        "strategy should mention proof/contract: {}",
        fb.iteration_hint.strategy
    );
}

#[test]
fn valid_program_iteration_hint_no_fixes() {
    let input = std::fs::read_to_string("testdata/hello_world.ftl").unwrap();
    let fb = feedback_for(&input);

    assert_eq!(fb.iteration_hint.estimated_fixes, 0);
    assert!(fb.iteration_hint.priority_order.is_empty());
    assert!(
        fb.iteration_hint.strategy.contains("No fixes"),
        "strategy: {}",
        fb.iteration_hint.strategy
    );
}

// ===========================================================================
// 6. Suggestions are actionable
// ===========================================================================

#[test]
fn feedback_suggestions_are_actionable() {
    // region_escape has clear actionable suggestions
    let input = std::fs::read_to_string("testdata/errors/region_escape.ftl").unwrap();
    let fb = feedback_for(&input);

    for issue in &fb.issues {
        if issue.severity == Severity::Error {
            let suggestion = issue.suggestion.as_ref();
            assert!(
                suggestion.is_some(),
                "error issues should have suggestions: {:?}",
                issue
            );

            let suggestion = suggestion.unwrap();
            // Action should be set
            assert!(
                matches!(
                    suggestion.action,
                    SuggestionAction::Replace
                        | SuggestionAction::Add
                        | SuggestionAction::Remove
                        | SuggestionAction::Modify
                        | SuggestionAction::Restructure
                ),
                "suggestion action should be set"
            );

            // Description should be non-empty
            assert!(
                !suggestion.description.is_empty(),
                "suggestion description should not be empty"
            );
        }
    }
}

#[test]
fn feedback_suggestions_have_target_nodes() {
    let input = std::fs::read_to_string("testdata/errors/region_cycle.ftl").unwrap();
    let fb = feedback_for(&input);

    for issue in &fb.issues {
        if issue.category == IssueCategory::RegionError {
            let suggestion = issue.suggestion.as_ref().unwrap();
            assert!(
                suggestion.target_node.is_some(),
                "region error suggestions should specify a target node"
            );
        }
    }
}

#[test]
fn feedback_proof_suggestions_have_examples() {
    let input = std::fs::read_to_string("testdata/concurrency.ftl").unwrap();
    let fb = feedback_for(&input);

    let disproven_issues: Vec<_> = fb
        .issues
        .iter()
        .filter(|i| i.severity == Severity::Error && i.category == IssueCategory::ProofFailure)
        .collect();

    for issue in &disproven_issues {
        let suggestion = issue.suggestion.as_ref().unwrap();
        assert!(
            suggestion.example.is_some(),
            "DISPROVEN suggestions should include an FTL example snippet"
        );
        let example = suggestion.example.as_ref().unwrap();
        assert!(
            example.contains("contract"),
            "example should contain 'contract': {}",
            example
        );
    }
}

// ===========================================================================
// 7. JSON serialization roundtrip
// ===========================================================================

#[test]
fn feedback_serializes_to_valid_json() {
    let input = std::fs::read_to_string("testdata/concurrency.ftl").unwrap();
    let fb = feedback_for(&input);

    let json = serde_json::to_string_pretty(&fb).expect("feedback should serialize to JSON");

    // Verify it parses back
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("serialized JSON should be valid");

    // Check top-level structure
    assert!(value.get("status").is_some());
    assert!(value.get("summary").is_some());
    assert!(value.get("issues").is_some());
    assert!(value.get("iteration_hint").is_some());

    // Status should be SCREAMING_SNAKE_CASE
    let status = value["status"].as_str().unwrap();
    assert!(
        status == "PASS" || status == "FIXABLE" || status == "FATAL",
        "status should be PASS/FIXABLE/FATAL, got: {}",
        status
    );
}

#[test]
fn feedback_pass_serialization() {
    let input = std::fs::read_to_string("testdata/hello_world.ftl").unwrap();
    let fb = feedback_for(&input);

    let json = serde_json::to_string(&fb).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(value["status"], "PASS");
    assert_eq!(value["issues"].as_array().unwrap().len(), 0);
    assert_eq!(value["iteration_hint"]["estimated_fixes"], 0);
}

// ===========================================================================
// 8. Edge cases
// ===========================================================================

#[test]
fn minimal_program_pass_feedback() {
    let input = std::fs::read_to_string("testdata/minimal.ftl").unwrap();
    let fb = feedback_for(&input);

    assert_eq!(fb.status, FeedbackStatus::Pass);
    assert!(fb.issues.is_empty());
}

#[test]
fn empty_input_fatal_feedback() {
    let fb = feedback_for("");
    assert_eq!(fb.status, FeedbackStatus::Fatal);
    assert!(!fb.issues.is_empty());
    // Empty input parses as an empty program but fails structural validation
    assert!(
        fb.issues[0].category == IssueCategory::ParseError
            || fb.issues[0].category == IssueCategory::StructuralValidation,
        "first issue should be parse or structural, got: {:?}",
        fb.issues[0].category
    );
}

#[test]
fn feedback_deduplicates_priority_order() {
    // Create feedback with duplicate node references
    let ve1 = ValidationError {
        error_code: 6002,
        node_id: "R:b2".to_string(),
        violation: "cycle".to_string(),
        message: "Region cycle detected".to_string(),
        suggestion: None,
    };
    let ve2 = ValidationError {
        error_code: 6002,
        node_id: "R:b2".to_string(),
        violation: "cycle".to_string(),
        message: "Region cycle detected (related)".to_string(),
        suggestion: None,
    };

    let fb = generate_feedback(&[], &[ve1, ve2], &[]);

    // Priority order should not have duplicates
    let mut seen = std::collections::HashSet::new();
    for id in &fb.iteration_hint.priority_order {
        assert!(
            seen.insert(id.clone()),
            "priority_order should not contain duplicates, found: {}",
            id
        );
    }
}
