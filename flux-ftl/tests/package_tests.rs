// ---------------------------------------------------------------------------
// Package system integration tests — multi-file FTL with imports
// ---------------------------------------------------------------------------

use std::path::Path;

use flux_ftl::parser::parse_ftl;
use flux_ftl::pipeline;

#[test]
fn import_resolution_success() {
    let source = std::fs::read_to_string("testdata/module_main.ftl")
        .expect("failed to read module_main.ftl");
    let parse_result = parse_ftl(&source);
    let ast = parse_result.ast.expect("parse should succeed");

    assert_eq!(ast.imports.len(), 1);
    assert_eq!(ast.imports[0], "module_a.ftl");

    let base_path = Path::new("testdata/module_main.ftl");
    let resolved = pipeline::resolve_imports(&ast, base_path)
        .expect("import resolution should succeed");

    // After resolution, imports should be cleared
    assert!(resolved.imports.is_empty());

    // The merged program should contain nodes from both files
    assert!(
        resolved.types.iter().any(|t| t.id.as_str() == "T:shared_int"),
        "should contain T:shared_int from module_a"
    );
    assert!(
        resolved.computes.iter().any(|c| c.id.as_str() == "C:shared_zero"),
        "should contain C:shared_zero from module_a"
    );
    assert!(
        resolved.effects.iter().any(|e| e.id.as_str() == "E:exit"),
        "should contain E:exit from module_main"
    );

    // Full check should pass on the resolved program
    let result = pipeline::run_check_program(resolved);
    assert_eq!(
        result.status,
        pipeline::FullStatus::Ok,
        "check on resolved program should pass: {:?}",
        result.validation_errors
    );
}

#[test]
fn circular_import_error() {
    let source = std::fs::read_to_string("testdata/module_circular_a.ftl")
        .expect("failed to read module_circular_a.ftl");
    let parse_result = parse_ftl(&source);
    let ast = parse_result.ast.expect("parse should succeed");

    let base_path = Path::new("testdata/module_circular_a.ftl");
    let result = pipeline::resolve_imports(&ast, base_path);

    assert!(result.is_err(), "circular import should produce an error");
    let errs = result.unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("circular")),
        "error should mention 'circular': {:?}",
        errs
    );
}

#[test]
fn missing_file_import_error() {
    let source = std::fs::read_to_string("testdata/module_missing_import.ftl")
        .expect("failed to read module_missing_import.ftl");
    let parse_result = parse_ftl(&source);
    let ast = parse_result.ast.expect("parse should succeed");

    let base_path = Path::new("testdata/module_missing_import.ftl");
    let result = pipeline::resolve_imports(&ast, base_path);

    assert!(result.is_err(), "missing file import should produce an error");
    let errs = result.unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("nonexistent_module.ftl")),
        "error should mention the missing file: {:?}",
        errs
    );
}

#[test]
fn duplicate_node_id_error() {
    let source = std::fs::read_to_string("testdata/module_duplicate.ftl")
        .expect("failed to read module_duplicate.ftl");
    let parse_result = parse_ftl(&source);
    let ast = parse_result.ast.expect("parse should succeed");

    let base_path = Path::new("testdata/module_duplicate.ftl");
    let result = pipeline::resolve_imports(&ast, base_path);

    assert!(result.is_err(), "duplicate node IDs should produce an error");
    let errs = result.unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("duplicate") && e.contains("T:shared_int")),
        "error should mention 'duplicate' and 'T:shared_int': {:?}",
        errs
    );
}

#[test]
fn program_without_imports_unchanged() {
    let source = std::fs::read_to_string("testdata/minimal.ftl")
        .expect("failed to read minimal.ftl");
    let parse_result = parse_ftl(&source);
    let ast = parse_result.ast.expect("parse should succeed");

    // No imports
    assert!(ast.imports.is_empty());

    // resolve_imports should still work (no-op)
    let base_path = Path::new("testdata/minimal.ftl");
    let resolved = pipeline::resolve_imports(&ast, base_path)
        .expect("should succeed with no imports");

    assert_eq!(resolved.types.len(), ast.types.len());
    assert_eq!(resolved.computes.len(), ast.computes.len());
}

#[test]
fn parse_import_syntax() {
    let source = r#"
import "types.ftl"
import "utils.ftl"

T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
K:main = seq { steps: [C:c1] }
entry: K:main
"#;
    let parse_result = parse_ftl(source);
    let ast = parse_result.ast.expect("parse should succeed");
    assert_eq!(ast.imports.len(), 2);
    assert_eq!(ast.imports[0], "types.ftl");
    assert_eq!(ast.imports[1], "utils.ftl");
}
