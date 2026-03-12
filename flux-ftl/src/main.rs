use std::io::Read;

use serde::Serialize;

use flux_ftl::parser::parse_ftl;
use flux_ftl::error::Status;
use flux_ftl::validator::validate;
use flux_ftl::type_checker::check_types_and_effects;
use flux_ftl::region_checker::check_regions;

#[derive(Debug, Serialize)]
struct FullResult {
    status: FullStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    ast: Option<flux_ftl::ast::Program>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    parse_errors: Vec<flux_ftl::error::ParseError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    validation_errors: Vec<GenericError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum FullStatus {
    Ok,
    ParseError,
    ValidationFail,
}

#[derive(Debug, Serialize)]
struct GenericError {
    error_code: u32,
    node_id: String,
    violation: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<String>,
}

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();

    let parse_result = parse_ftl(&input);

    let ast = match parse_result.status {
        Status::Ok => parse_result.ast.unwrap(),
        Status::Error => {
            let out = FullResult {
                status: FullStatus::ParseError,
                ast: None,
                parse_errors: parse_result.errors,
                validation_errors: Vec::new(),
            };
            println!("{}", serde_json::to_string(&out).unwrap());
            return;
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

    let has_fatal = validation_errors.iter().any(|e| e.error_code < 2000 || e.error_code >= 3000);

    let out = FullResult {
        status: if has_fatal { FullStatus::ValidationFail } else { FullStatus::Ok },
        ast: Some(ast),
        parse_errors: Vec::new(),
        validation_errors,
    };

    println!("{}", serde_json::to_string(&out).unwrap());
}
