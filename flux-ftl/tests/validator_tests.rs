use flux_ftl::ast::*;
use flux_ftl::error::Status;
use flux_ftl::parser::parse_ftl;
use flux_ftl::region_checker::{check_regions, RegionError};

// ===========================================================================
// Helpers
// ===========================================================================

/// Parse FTL source into a Program, panicking on parse failure.
fn must_parse(input: &str) -> Program {
    let result = parse_ftl(input);
    assert!(
        matches!(result.status, Status::Ok),
        "expected successful parse, got errors: {:?}",
        result.errors
    );
    result.ast.expect("ast should be Some on Ok status")
}

/// Assert that no region errors are present for the given program.
fn assert_no_region_errors(program: &Program, label: &str) {
    let errors = check_regions(program);
    assert!(
        errors.is_empty(),
        "{label}: expected 0 region errors, got {}: {errors:?}",
        errors.len()
    );
}

/// Assert that at least one error with the given code is present.
fn assert_has_error(errors: &[RegionError], code: u32) {
    assert!(
        errors.iter().any(|e| e.error_code == code),
        "expected error code {code} in: {errors:?}"
    );
}

/// Assert that no error with the given code is present.
fn assert_no_error(errors: &[RegionError], code: u32) {
    assert!(
        !errors.iter().any(|e| e.error_code == code),
        "did NOT expect error code {code} in: {errors:?}"
    );
}

// ===========================================================================
// Correct testdata files — 0 region errors expected
// ===========================================================================

#[test]
fn region_check_hello_world() {
    let input = include_str!("../testdata/hello_world.ftl");
    let ast = must_parse(input);
    assert_no_region_errors(&ast, "hello_world.ftl");
}

#[test]
fn region_check_snake_game() {
    let input = include_str!("../testdata/snake_game.ftl");
    let ast = must_parse(input);
    assert_no_region_errors(&ast, "snake_game.ftl");
}

#[test]
fn region_check_minimal() {
    let input = include_str!("../testdata/minimal.ftl");
    let ast = must_parse(input);
    assert_no_region_errors(&ast, "minimal.ftl");
}

#[test]
fn region_check_concurrency() {
    let input = include_str!("../testdata/concurrency.ftl");
    let ast = must_parse(input);
    assert_no_region_errors(&ast, "concurrency.ftl");
}

#[test]
fn region_check_ffi() {
    let input = include_str!("../testdata/ffi.ftl");
    let ast = must_parse(input);
    assert_no_region_errors(&ast, "ffi.ftl");
}

// ===========================================================================
// Error testdata files — specific region errors expected
// ===========================================================================

#[test]
fn region_error_file_no_parent() {
    let input = include_str!("../testdata/errors/region_no_parent.ftl");
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert_has_error(&errors, 6004); // SCOPED_WITHOUT_PARENT
}

#[test]
fn region_error_file_cycle() {
    let input = include_str!("../testdata/errors/region_cycle.ftl");
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert_has_error(&errors, 6002); // REGION_CYCLE
}

#[test]
fn region_error_file_escape() {
    let input = include_str!("../testdata/errors/region_escape.ftl");
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert_has_error(&errors, 6006); // REGION_ESCAPE
}

// ===========================================================================
// Inline FTL — scoped region without parent (6004)
// ===========================================================================

#[test]
fn inline_scoped_without_parent() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
R:b1 = region { lifetime: scoped }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert_has_error(&errors, 6004);
}

// ===========================================================================
// Inline FTL — static region with parent (6003)
// ===========================================================================

#[test]
fn inline_static_with_parent() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
R:b1 = region { lifetime: static }
R:b2 = region { lifetime: static, parent: R:b1 }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert_has_error(&errors, 6003);
}

// ===========================================================================
// Inline FTL — region cycle (6002)
// ===========================================================================

#[test]
fn inline_region_cycle() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
R:b1 = region { lifetime: scoped, parent: R:b2 }
R:b2 = region { lifetime: scoped, parent: R:b1 }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert_has_error(&errors, 6002);
}

// ===========================================================================
// Inline FTL — parent not found (6001)
// ===========================================================================

#[test]
fn inline_parent_not_found() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b99 }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert_has_error(&errors, 6001);
}

// ===========================================================================
// Inline FTL — M:alloc with non-existent region (6005)
// ===========================================================================

#[test]
fn inline_invalid_alloc_region_ref() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
R:b1 = region { lifetime: static }
M:g1 = alloc { type: T:a1, region: R:b99 }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert_has_error(&errors, 6005);
}

// ===========================================================================
// Inline FTL — C:const_bytes with non-existent region (6005)
// ===========================================================================

#[test]
fn inline_invalid_const_bytes_region_ref() {
    let input = r#"
T:a1 = array { element: u8, max_length: 4 }
T:a2 = integer { bits: 32, signed: true }
R:b1 = region { lifetime: static }
C:c1 = const_bytes { value: [72,101], type: T:a1, region: R:b99 }
C:c2 = const { value: 0, type: T:a2 }
E:d1 = syscall_exit { inputs: [C:c2], type: T:a2, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert_has_error(&errors, 6005);
}

// ===========================================================================
// Inline FTL — valid nested regions (no errors)
// ===========================================================================

#[test]
fn inline_valid_three_level_nesting() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b1 }
R:b3 = region { lifetime: scoped, parent: R:b2 }
M:g1 = alloc { type: T:a1, region: R:b3 }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert!(errors.is_empty(), "valid 3-level nesting should have no errors: {errors:?}");
}

// ===========================================================================
// Inline FTL — store within same region is fine (no escape)
// ===========================================================================

#[test]
fn inline_store_same_region_ok() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b1 }
M:g1 = alloc { type: T:a1, region: R:b2 }
M:g2 = alloc { type: T:a1, region: R:b2 }
C:c0 = const { value: 0, type: T:a1 }
M:g3 = store { target: M:g1, index: C:c0, value: M:g2 }
E:d1 = syscall_exit { inputs: [C:c0], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert_no_error(&errors, 6006);
}

// ===========================================================================
// Inline FTL — store from outer to inner region is OK
// ===========================================================================

#[test]
fn inline_store_outer_to_inner_ok() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b1 }
M:g1 = alloc { type: T:a1, region: R:b2 }
M:g2 = alloc { type: T:a1, region: R:b1 }
C:c0 = const { value: 0, type: T:a1 }
M:g3 = store { target: M:g1, index: C:c0, value: M:g2 }
E:d1 = syscall_exit { inputs: [C:c0], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert_no_error(&errors, 6006);
}

// ===========================================================================
// Inline FTL — store from inner to outer region is an ESCAPE
// ===========================================================================

#[test]
fn inline_store_inner_to_outer_escape() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b1 }
M:g1 = alloc { type: T:a1, region: R:b1 }
M:g2 = alloc { type: T:a1, region: R:b2 }
C:c0 = const { value: 0, type: T:a1 }
M:g3 = store { target: M:g1, index: C:c0, value: M:g2 }
E:d1 = syscall_exit { inputs: [C:c0], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert_has_error(&errors, 6006);
}

// ===========================================================================
// Error serialization — RegionError serializes to JSON
// ===========================================================================

#[test]
fn region_error_serializes_to_json() {
    let err = flux_ftl::region_checker::RegionError {
        error_code: 6004,
        node_id: "R:b2".into(),
        violation: "SCOPED_WITHOUT_PARENT".into(),
        message: "Scoped region R:b2 must have a parent".into(),
        suggestion: Some("Add a parent field".into()),
    };

    let json = serde_json::to_string(&err).expect("should serialize");
    assert!(json.contains("6004"));
    assert!(json.contains("SCOPED_WITHOUT_PARENT"));
}

// ===========================================================================
// Multiple errors in one program
// ===========================================================================

#[test]
fn multiple_errors_combined() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
R:b1 = region { lifetime: static, parent: R:b2 }
R:b2 = region { lifetime: scoped }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
    let ast = must_parse(input);
    let errors = check_regions(&ast);

    // R:b1 is static with parent -> 6003
    assert_has_error(&errors, 6003);
    // R:b2 is scoped without parent -> 6004
    assert_has_error(&errors, 6004);
}

// ===========================================================================
// Empty program (no regions) — no errors
// ===========================================================================

#[test]
fn no_regions_no_errors() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
"#;
    let ast = must_parse(input);
    let errors = check_regions(&ast);
    assert!(errors.is_empty(), "no regions should yield no errors: {errors:?}");
}
