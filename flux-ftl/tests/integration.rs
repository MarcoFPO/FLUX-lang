use flux_ftl::parser::parse_ftl;
use flux_ftl::error::Status;
use flux_ftl::ast::*;

// ===========================================================================
// Helper
// ===========================================================================

/// Assert that parsing succeeded and return the AST.
fn parse_ok(input: &str) -> Program {
    let result = parse_ftl(input);
    assert!(
        matches!(result.status, Status::Ok),
        "expected Status::Ok, got errors: {:?}",
        result.errors
    );
    result.ast.expect("ast should be Some on Ok status")
}

/// Assert that parsing failed.
fn parse_err(input: &str) {
    let result = parse_ftl(input);
    assert!(
        matches!(result.status, Status::Error),
        "expected Status::Error but got Ok"
    );
    assert!(result.ast.is_none());
    assert!(!result.errors.is_empty());
}

// ===========================================================================
// 1. Erfolgreiche Parses der Testdateien
// ===========================================================================

#[test]
fn parse_hello_world() {
    let input = include_str!("../testdata/hello_world.ftl");
    let ast = parse_ok(input);

    assert_eq!(ast.types.len(), 3, "hello_world: 3 type defs (a1, a2, a3)");
    assert_eq!(ast.regions.len(), 1, "hello_world: 1 region (b1)");
    assert_eq!(ast.computes.len(), 5, "hello_world: 5 computes (c1..c5)");
    assert_eq!(ast.effects.len(), 3, "hello_world: 3 effects (d1, d2, d3)");
    assert_eq!(ast.controls.len(), 3, "hello_world: 3 controls (f1, f2, f3)");
    assert_eq!(ast.contracts.len(), 2, "hello_world: 2 contracts (e1, e2)");
    assert_eq!(ast.memories.len(), 0, "hello_world: 0 memory nodes");
    assert_eq!(ast.externs.len(), 0, "hello_world: 0 extern nodes");
    assert_eq!(ast.entry.as_str(), "K:f1");
}

#[test]
fn parse_minimal() {
    let input = include_str!("../testdata/minimal.ftl");
    let ast = parse_ok(input);

    assert_eq!(ast.types.len(), 1);
    assert_eq!(ast.regions.len(), 0);
    assert_eq!(ast.computes.len(), 1);
    assert_eq!(ast.effects.len(), 1);
    assert_eq!(ast.controls.len(), 1);
    assert_eq!(ast.contracts.len(), 0);
    assert_eq!(ast.memories.len(), 0);
    assert_eq!(ast.externs.len(), 0);
    assert_eq!(ast.entry.as_str(), "K:f1");
}

#[test]
fn parse_snake_game() {
    let input = include_str!("../testdata/snake_game.ftl");
    let ast = parse_ok(input);

    assert_eq!(ast.types.len(), 14);
    assert_eq!(ast.regions.len(), 4);
    assert_eq!(ast.computes.len(), 46);
    assert_eq!(ast.effects.len(), 14);
    assert_eq!(ast.controls.len(), 25);
    assert_eq!(ast.contracts.len(), 10);
    assert_eq!(ast.memories.len(), 11);
    assert_eq!(ast.externs.len(), 0);
    assert_eq!(ast.entry.as_str(), "K:f_main");
}

#[test]
fn parse_concurrency() {
    let input = include_str!("../testdata/concurrency.ftl");
    let ast = parse_ok(input);

    assert_eq!(ast.types.len(), 6);
    assert_eq!(ast.regions.len(), 2);
    assert_eq!(ast.computes.len(), 16);
    assert_eq!(ast.effects.len(), 2);
    assert_eq!(ast.controls.len(), 9);
    assert_eq!(ast.contracts.len(), 3);
    assert_eq!(ast.memories.len(), 4);
    assert_eq!(ast.externs.len(), 0);
    assert_eq!(ast.entry.as_str(), "K:f_main");
}

#[test]
fn parse_ffi() {
    let input = include_str!("../testdata/ffi.ftl");
    let ast = parse_ok(input);

    assert_eq!(ast.types.len(), 10);
    assert_eq!(ast.regions.len(), 2);
    assert_eq!(ast.computes.len(), 10);
    assert_eq!(ast.effects.len(), 10);
    assert_eq!(ast.controls.len(), 11);
    // ffi.ftl has 7 V-nodes, but some have multiple clauses (trust + assume + post)
    // V:e1 has trust + assume + post = 2 ContractDefs (assume, post)
    // V:e2 has trust + assume + post = 2
    // V:e3 has trust + assume + post = 2
    // V:e4 has trust + assume + post = 2
    // V:e5 has trust + assume + post = 2
    // V:e6 has pre = 1
    // V:e7 has pre = 1
    // Total = 12
    assert_eq!(ast.contracts.len(), 12);
    assert_eq!(ast.memories.len(), 0);
    assert_eq!(ast.externs.len(), 6);
    assert_eq!(ast.entry.as_str(), "K:f_main");
}

// ===========================================================================
// 1b. Entry-point und spezifische Node-Inhalte
// ===========================================================================

#[test]
fn hello_world_entry_is_seq() {
    let input = include_str!("../testdata/hello_world.ftl");
    let ast = parse_ok(input);

    let entry_ctrl = ast.controls.iter().find(|c| c.id.as_str() == "K:f1").unwrap();
    match &entry_ctrl.op {
        ControlOp::Seq { steps } => {
            assert_eq!(steps.len(), 1);
            assert_eq!(steps[0].as_str(), "E:d1");
        }
        other => panic!("expected Seq, got {:?}", other),
    }
}

#[test]
fn hello_world_const_bytes() {
    let input = include_str!("../testdata/hello_world.ftl");
    let ast = parse_ok(input);

    let c1 = ast.computes.iter().find(|c| c.id.as_str() == "C:c1").unwrap();
    match &c1.op {
        ComputeOp::ConstBytes { value, region, .. } => {
            assert_eq!(value, b"Hello World\n");
            assert_eq!(region.as_str(), "R:b1");
        }
        other => panic!("expected ConstBytes, got {:?}", other),
    }
}

// ===========================================================================
// 2. Fehlerfaelle
// ===========================================================================

#[test]
fn error_empty_input() {
    // Empty input has no entry and no nodes. The grammar allows program = SOI ~ statement* ~ EOI
    // so an empty input actually parses successfully (0 statements, default entry K:main).
    let result = parse_ftl("");
    assert!(matches!(result.status, Status::Ok));
    let ast = result.ast.unwrap();
    assert_eq!(ast.types.len(), 0);
    // Default entry when none specified
    assert_eq!(ast.entry.as_str(), "K:main");
}

#[test]
fn error_unknown_prefix() {
    let input = "Z:z1 = something { value: 42 }\n";
    parse_err(input);
}

#[test]
fn const_missing_type_parsed_as_generic() {
    // const without type: is caught by compute_generic_op as a generic compute,
    // not as a parse error. The grammar's catch-all handles it.
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 42 }
K:f1 = seq { steps: [C:c1] }
entry: K:f1
"#;
    let ast = parse_ok(input);
    match &ast.computes[0].op {
        ComputeOp::Generic { name, .. } => {
            assert_eq!(name, "const");
        }
        other => panic!("expected Generic catch-all, got {:?}", other),
    }
}

#[test]
fn error_invalid_type_body() {
    // A type body that doesn't match any known type kind
    let input = "T:a1 = magic { stuff: 42 }\nentry: K:main\n";
    parse_err(input);
}

#[test]
fn error_unclosed_braces() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true
C:c1 = const { value: 0, type: T:a1 }
entry: K:f1
"#;
    parse_err(input);
}

#[test]
fn error_unclosed_bracket() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
K:f1 = seq { steps: [C:c1 }
entry: K:f1
"#;
    parse_err(input);
}

#[test]
fn missing_entry_defaults_to_k_main() {
    // The parser defaults to K:main when no entry is specified
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
"#;
    let ast = parse_ok(input);
    assert_eq!(ast.entry.as_str(), "K:main");
}

#[test]
fn duplicate_entry_uses_last() {
    // Two entry: definitions -- the second one should overwrite the first.
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
K:f1 = seq { steps: [C:c1] }
K:f2 = seq { steps: [C:c1] }
entry: K:f1
entry: K:f2
"#;
    let ast = parse_ok(input);
    assert_eq!(ast.entry.as_str(), "K:f2");
}

// ===========================================================================
// 3. Einzelne Konstrukte — T-Nodes
// ===========================================================================

#[test]
fn t_node_integer() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
entry: K:main
"#;
    let ast = parse_ok(input);
    assert_eq!(ast.types.len(), 1);
    match &ast.types[0].body {
        TypeBody::Integer { bits, signed } => {
            assert_eq!(*bits, 32);
            assert!(*signed);
        }
        other => panic!("expected Integer, got {:?}", other),
    }
}

#[test]
fn t_node_integer_unsigned_default() {
    // signed is optional, defaults to false
    let input = r#"
T:a1 = integer { bits: 64 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.types[0].body {
        TypeBody::Integer { bits, signed } => {
            assert_eq!(*bits, 64);
            assert!(!*signed);
        }
        other => panic!("expected Integer, got {:?}", other),
    }
}

#[test]
fn t_node_float() {
    let input = r#"
T:a1 = float { bits: 64 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.types[0].body {
        TypeBody::Float { bits } => assert_eq!(*bits, 64),
        other => panic!("expected Float, got {:?}", other),
    }
}

#[test]
fn t_node_boolean() {
    let input = "T:a1 = boolean\nentry: K:main\n";
    let ast = parse_ok(input);
    assert!(matches!(ast.types[0].body, TypeBody::Boolean));
}

#[test]
fn t_node_unit() {
    let input = "T:a1 = unit\nentry: K:main\n";
    let ast = parse_ok(input);
    assert!(matches!(ast.types[0].body, TypeBody::Unit));
}

#[test]
fn t_node_struct_with_layout() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
T:a2 = struct { fields: [x: T:a1, y: T:a1], layout: PACKED }
entry: K:main
"#;
    let ast = parse_ok(input);
    let t = &ast.types[1];
    assert_eq!(t.id.as_str(), "T:a2");
    match &t.body {
        TypeBody::Struct { fields, layout } => {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].name, "x");
            assert_eq!(fields[1].name, "y");
            assert!(matches!(layout, Layout::Packed));
        }
        other => panic!("expected Struct, got {:?}", other),
    }
}

#[test]
fn t_node_struct_default_layout() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
T:a2 = struct { fields: [x: T:a1] }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.types[1].body {
        TypeBody::Struct { layout, .. } => {
            assert!(matches!(layout, Layout::Optimal));
        }
        other => panic!("expected Struct, got {:?}", other),
    }
}

#[test]
fn t_node_array_with_constraint() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
T:a2 = array { element: T:a1, max_length: 100, constraint: result >= 0 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.types[1].body {
        TypeBody::Array { element, max_length, constraint } => {
            assert!(matches!(element, TypeRef::Id { .. }));
            assert_eq!(*max_length, 100);
            assert!(constraint.is_some());
        }
        other => panic!("expected Array, got {:?}", other),
    }
}

#[test]
fn t_node_array_without_constraint() {
    let input = r#"
T:a1 = array { element: u8, max_length: 256 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.types[0].body {
        TypeBody::Array { element, max_length, constraint } => {
            match element {
                TypeRef::Builtin { name } => assert_eq!(name, "u8"),
                other => panic!("expected Builtin, got {:?}", other),
            }
            assert_eq!(*max_length, 256);
            assert!(constraint.is_none());
        }
        other => panic!("expected Array, got {:?}", other),
    }
}

#[test]
fn t_node_variant() {
    let input = r#"
T:a1 = unit
T:a2 = variant { cases: [UP: T:a1, DOWN: T:a1, LEFT: T:a1, RIGHT: T:a1] }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.types[1].body {
        TypeBody::Variant { cases } => {
            assert_eq!(cases.len(), 4);
            assert_eq!(cases[0].tag, "UP");
            assert_eq!(cases[1].tag, "DOWN");
            assert_eq!(cases[2].tag, "LEFT");
            assert_eq!(cases[3].tag, "RIGHT");
        }
        other => panic!("expected Variant, got {:?}", other),
    }
}

#[test]
fn t_node_fn_type() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
T:a2 = fn { params: [T:a1, T:a1], result: T:a1, effects: [IO, MEM] }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.types[1].body {
        TypeBody::Fn { params, result, effects } => {
            assert_eq!(params.len(), 2);
            assert!(matches!(result.as_ref(), TypeRef::Id { .. }));
            assert_eq!(effects.len(), 2);
            assert_eq!(effects[0], "IO");
            assert_eq!(effects[1], "MEM");
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn t_node_fn_type_no_effects() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
T:a2 = fn { params: [T:a1], result: T:a1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.types[1].body {
        TypeBody::Fn { effects, .. } => {
            assert!(effects.is_empty());
        }
        other => panic!("expected Fn, got {:?}", other),
    }
}

#[test]
fn t_node_opaque() {
    let input = r#"
T:a1 = opaque { size: 216, align: 8 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.types[0].body {
        TypeBody::Opaque { size, align } => {
            assert_eq!(*size, 216);
            assert_eq!(*align, 8);
        }
        other => panic!("expected Opaque, got {:?}", other),
    }
}

// ===========================================================================
// 3. Einzelne Konstrukte — R-Node
// ===========================================================================

#[test]
fn r_node_static() {
    let input = "R:b1 = region { lifetime: static }\nentry: K:main\n";
    let ast = parse_ok(input);
    assert_eq!(ast.regions.len(), 1);
    assert!(matches!(ast.regions[0].lifetime, Lifetime::Static));
    assert!(ast.regions[0].parent.is_none());
}

#[test]
fn r_node_scoped_with_parent() {
    let input = r#"
R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    assert_eq!(ast.regions.len(), 2);
    assert!(matches!(ast.regions[1].lifetime, Lifetime::Scoped));
    assert_eq!(ast.regions[1].parent.as_ref().unwrap().as_str(), "R:b1");
}

// ===========================================================================
// 3. Einzelne Konstrukte — C-Node
// ===========================================================================

#[test]
fn c_node_const_integer() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 42, type: T:a1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    let c = &ast.computes[0];
    assert_eq!(c.id.as_str(), "C:c1");
    match &c.op {
        ComputeOp::Const { value, type_ref, region } => {
            assert!(matches!(value, Literal::Integer { value: 42 }));
            assert!(matches!(type_ref, TypeRef::Id { .. }));
            assert!(region.is_none());
        }
        other => panic!("expected Const, got {:?}", other),
    }
}

#[test]
fn c_node_const_negative() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: -1, type: T:a1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.computes[0].op {
        ComputeOp::Const { value, .. } => {
            assert!(matches!(value, Literal::Integer { value: -1 }));
        }
        other => panic!("expected Const, got {:?}", other),
    }
}

#[test]
fn c_node_const_bool() {
    let input = r#"
T:a1 = boolean
C:c1 = const { value: true, type: T:a1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.computes[0].op {
        ComputeOp::Const { value, .. } => {
            assert!(matches!(value, Literal::Bool { value: true }));
        }
        other => panic!("expected Const, got {:?}", other),
    }
}

#[test]
fn c_node_arith() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 1, type: T:a1 }
C:c2 = const { value: 2, type: T:a1 }
C:c3 = add { inputs: [C:c1, C:c2], type: T:a1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    let c3 = &ast.computes[2];
    assert_eq!(c3.id.as_str(), "C:c3");
    match &c3.op {
        ComputeOp::Arith { opcode, inputs, .. } => {
            assert_eq!(opcode, "add");
            assert_eq!(inputs.len(), 2);
            assert_eq!(inputs[0].as_str(), "C:c1");
            assert_eq!(inputs[1].as_str(), "C:c2");
        }
        other => panic!("expected Arith, got {:?}", other),
    }
}

#[test]
fn c_node_call_pure() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
C:c2 = call_pure { target: "my_func", inputs: [C:c1], type: T:a1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.computes[1].op {
        ComputeOp::CallPure { target, inputs, .. } => {
            assert_eq!(target, "my_func");
            assert_eq!(inputs.len(), 1);
        }
        other => panic!("expected CallPure, got {:?}", other),
    }
}

#[test]
fn c_node_atomic_load() {
    let input = r#"
T:a1 = integer { bits: 64, signed: false }
R:b1 = region { lifetime: static }
M:g1 = alloc { type: T:a1, region: R:b1 }
C:c1 = atomic_load { source: M:g1, order: ACQUIRE, type: T:a1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.computes[0].op {
        ComputeOp::AtomicLoad { source, order, .. } => {
            assert_eq!(source.as_str(), "M:g1");
            assert!(matches!(order, MemoryOrder::Acquire));
        }
        other => panic!("expected AtomicLoad, got {:?}", other),
    }
}

#[test]
fn c_node_atomic_store() {
    let input = r#"
T:a1 = integer { bits: 64, signed: false }
R:b1 = region { lifetime: static }
M:g1 = alloc { type: T:a1, region: R:b1 }
C:c0 = const { value: 0, type: T:a1 }
C:c1 = atomic_store { target: M:g1, value: C:c0, order: SEQ_CST }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.computes[1].op {
        ComputeOp::AtomicStore { target, value, order } => {
            assert_eq!(target.as_str(), "M:g1");
            assert_eq!(value.as_str(), "C:c0");
            assert!(matches!(order, MemoryOrder::SeqCst));
        }
        other => panic!("expected AtomicStore, got {:?}", other),
    }
}

#[test]
fn c_node_atomic_cas() {
    let input = r#"
T:a1 = integer { bits: 64, signed: false }
R:b1 = region { lifetime: static }
M:g1 = alloc { type: T:a1, region: R:b1 }
C:c0 = const { value: 0, type: T:a1 }
C:c1 = const { value: 1, type: T:a1 }
K:f_ok = seq { steps: [C:c0] }
K:f_fail = seq { steps: [C:c1] }
C:c2 = atomic_cas { target: M:g1, expected: C:c0, desired: C:c1, order: SEQ_CST, success: K:f_ok, failure: K:f_fail }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.computes[2].op {
        ComputeOp::AtomicCas { target, expected, desired, order, success, failure } => {
            assert_eq!(target.as_str(), "M:g1");
            assert_eq!(expected.as_str(), "C:c0");
            assert_eq!(desired.as_str(), "C:c1");
            assert!(matches!(order, MemoryOrder::SeqCst));
            assert_eq!(success.as_str(), "K:f_ok");
            assert_eq!(failure.as_str(), "K:f_fail");
        }
        other => panic!("expected AtomicCas, got {:?}", other),
    }
}

#[test]
fn c_node_generic_op() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
C:c2 = bhaskara_approx { inputs: [C:c1], type: T:a1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.computes[1].op {
        ComputeOp::Generic { name, inputs, .. } => {
            assert_eq!(name, "bhaskara_approx");
            assert_eq!(inputs.len(), 1);
        }
        other => panic!("expected Generic, got {:?}", other),
    }
}

// ===========================================================================
// 3. Einzelne Konstrukte — E-Node
// ===========================================================================

#[test]
fn e_node_syscall_exit_no_success_failure() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
T:a2 = unit
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a2, effects: [PROC] }
entry: K:main
"#;
    let ast = parse_ok(input);
    assert_eq!(ast.effects.len(), 1);
    match &ast.effects[0].op {
        EffectOp::Syscall { name, inputs, effects, success, failure, .. } => {
            assert_eq!(name, "syscall_exit");
            assert_eq!(inputs.len(), 1);
            assert_eq!(effects, &["PROC"]);
            assert!(success.is_none());
            assert!(failure.is_none());
        }
        other => panic!("expected Syscall, got {:?}", other),
    }
}

#[test]
fn e_node_syscall_write_with_success_failure() {
    let input = r#"
T:a1 = integer { bits: 64, signed: false }
C:c1 = const { value: 1, type: T:a1 }
C:c2 = const { value: 0, type: T:a1 }
C:c3 = const { value: 5, type: T:a1 }
K:f_ok = seq { steps: [C:c1] }
K:f_fail = seq { steps: [C:c2] }
E:d1 = syscall_write { inputs: [C:c1, C:c2, C:c3], type: T:a1, effects: [IO], success: K:f_ok, failure: K:f_fail }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.effects[0].op {
        EffectOp::Syscall { name, inputs, success, failure, .. } => {
            assert_eq!(name, "syscall_write");
            assert_eq!(inputs.len(), 3);
            assert_eq!(success.as_ref().unwrap().as_str(), "K:f_ok");
            assert_eq!(failure.as_ref().unwrap().as_str(), "K:f_fail");
        }
        other => panic!("expected Syscall, got {:?}", other),
    }
}

#[test]
fn e_node_call_extern() {
    let input = r#"
T:a1 = integer { bits: 64, signed: false }
T:a2 = unit
X:ext1 = extern { name: "malloc", abi: C, params: [T:a1], result: T:a1, effects: [MEM] }
C:c1 = const { value: 4096, type: T:a1 }
K:f_ok = seq { steps: [C:c1] }
K:f_fail = seq { steps: [C:c1] }
E:d1 = call_extern { target: X:ext1, inputs: [C:c1], type: T:a1, effects: [MEM], success: K:f_ok, failure: K:f_fail }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.effects[0].op {
        EffectOp::CallExtern { target, inputs, effects, success, failure, .. } => {
            assert_eq!(target.as_str(), "X:ext1");
            assert_eq!(inputs.len(), 1);
            assert_eq!(effects, &["MEM"]);
            assert_eq!(success.as_str(), "K:f_ok");
            assert_eq!(failure.as_str(), "K:f_fail");
        }
        other => panic!("expected CallExtern, got {:?}", other),
    }
}

// ===========================================================================
// 3. Einzelne Konstrukte — K-Node
// ===========================================================================

#[test]
fn k_node_seq() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
C:c2 = const { value: 1, type: T:a1 }
K:f1 = seq { steps: [C:c1, C:c2] }
entry: K:f1
"#;
    let ast = parse_ok(input);
    match &ast.controls[0].op {
        ControlOp::Seq { steps } => {
            assert_eq!(steps.len(), 2);
        }
        other => panic!("expected Seq, got {:?}", other),
    }
}

#[test]
fn k_node_branch() {
    let input = r#"
T:a1 = boolean
C:c1 = const { value: true, type: T:a1 }
K:f_true = seq { steps: [C:c1] }
K:f_false = seq { steps: [C:c1] }
K:f1 = branch { condition: C:c1, true: K:f_true, false: K:f_false }
entry: K:f1
"#;
    let ast = parse_ok(input);
    let branch = ast.controls.iter().find(|c| c.id.as_str() == "K:f1").unwrap();
    match &branch.op {
        ControlOp::Branch { condition, true_branch, false_branch } => {
            assert_eq!(condition.as_str(), "C:c1");
            assert_eq!(true_branch.as_str(), "K:f_true");
            assert_eq!(false_branch.as_str(), "K:f_false");
        }
        other => panic!("expected Branch, got {:?}", other),
    }
}

#[test]
fn k_node_loop() {
    let input = r#"
T:a1 = boolean
T:a2 = integer { bits: 32, signed: true }
C:c1 = const { value: true, type: T:a1 }
C:c_state = const { value: 0, type: T:a2 }
K:f_body = seq { steps: [C:c1] }
K:f1 = loop { condition: C:c1, body: K:f_body, state: C:c_state, state_type: T:a2 }
entry: K:f1
"#;
    let ast = parse_ok(input);
    let lp = ast.controls.iter().find(|c| c.id.as_str() == "K:f1").unwrap();
    match &lp.op {
        ControlOp::Loop { condition, body, state, state_type } => {
            assert_eq!(condition.as_str(), "C:c1");
            assert_eq!(body.as_str(), "K:f_body");
            assert_eq!(state.as_str(), "C:c_state");
            assert!(matches!(state_type, TypeRef::Id { .. }));
        }
        other => panic!("expected Loop, got {:?}", other),
    }
}

#[test]
fn k_node_par_with_memory_order() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
K:f_prod = seq { steps: [C:c1] }
K:f_cons = seq { steps: [C:c1] }
K:f1 = par { branches: [K:f_prod, K:f_cons], sync: BARRIER, memory_order: ACQUIRE_RELEASE }
entry: K:f1
"#;
    let ast = parse_ok(input);
    let par = ast.controls.iter().find(|c| c.id.as_str() == "K:f1").unwrap();
    match &par.op {
        ControlOp::Par { branches, sync, memory_order } => {
            assert_eq!(branches.len(), 2);
            assert!(matches!(sync, SyncMode::Barrier));
            assert!(matches!(memory_order.as_ref().unwrap(), MemoryOrder::AcquireRelease));
        }
        other => panic!("expected Par, got {:?}", other),
    }
}

#[test]
fn k_node_par_no_memory_order() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
K:f_a = seq { steps: [C:c1] }
K:f_b = seq { steps: [C:c1] }
K:f1 = par { branches: [K:f_a, K:f_b], sync: NONE }
entry: K:f1
"#;
    let ast = parse_ok(input);
    let par = ast.controls.iter().find(|c| c.id.as_str() == "K:f1").unwrap();
    match &par.op {
        ControlOp::Par { sync, memory_order, .. } => {
            assert!(matches!(sync, SyncMode::None));
            assert!(memory_order.is_none());
        }
        other => panic!("expected Par, got {:?}", other),
    }
}

// ===========================================================================
// 3. Einzelne Konstrukte — V-Node (Contract)
// ===========================================================================

#[test]
fn v_node_contract_pre() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 1, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
V:e1 = contract { target: E:d1, pre: C:c1.val == 1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    assert_eq!(ast.contracts.len(), 1);
    let v = &ast.contracts[0];
    assert_eq!(v.id.as_str(), "V:e1");
    assert_eq!(v.target.as_str(), "E:d1");
    assert!(matches!(v.clause, ContractClause::Pre { .. }));
    assert!(v.trust.is_none());
}

#[test]
fn v_node_contract_trust_extern() {
    let input = r#"
T:a1 = integer { bits: 64, signed: false }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
V:e1 = contract { target: E:d1, trust: EXTERN, assume: true, post: result != 0 }
entry: K:main
"#;
    let ast = parse_ok(input);
    // trust + assume + post => 2 ContractDefs (assume, post), both with trust: EXTERN
    assert_eq!(ast.contracts.len(), 2);
    assert!(matches!(ast.contracts[0].trust.as_ref().unwrap(), TrustLevel::Extern));
    assert!(matches!(ast.contracts[0].clause, ContractClause::Assume { .. }));
    assert!(matches!(ast.contracts[1].clause, ContractClause::Post { .. }));
    assert!(matches!(ast.contracts[1].trust.as_ref().unwrap(), TrustLevel::Extern));
}

// ===========================================================================
// 3. Einzelne Konstrukte — M-Node (Memory)
// ===========================================================================

#[test]
fn m_node_alloc() {
    let input = r#"
T:a1 = integer { bits: 64, signed: false }
R:b1 = region { lifetime: static }
M:g1 = alloc { type: T:a1, region: R:b1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    assert_eq!(ast.memories.len(), 1);
    match &ast.memories[0].op {
        MemoryOp::Alloc { type_ref, region } => {
            assert!(matches!(type_ref, TypeRef::Id { .. }));
            assert_eq!(region.as_str(), "R:b1");
        }
        other => panic!("expected Alloc, got {:?}", other),
    }
}

#[test]
fn m_node_load() {
    let input = r#"
T:a1 = integer { bits: 64, signed: false }
R:b1 = region { lifetime: static }
M:g1 = alloc { type: T:a1, region: R:b1 }
C:c0 = const { value: 0, type: T:a1 }
M:g2 = load { source: M:g1, index: C:c0, type: T:a1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    let m = ast.memories.iter().find(|m| m.id.as_str() == "M:g2").unwrap();
    match &m.op {
        MemoryOp::Load { source, index, type_ref } => {
            assert_eq!(source.as_str(), "M:g1");
            assert_eq!(index.as_str(), "C:c0");
            assert!(matches!(type_ref, TypeRef::Id { .. }));
        }
        other => panic!("expected Load, got {:?}", other),
    }
}

#[test]
fn m_node_store() {
    let input = r#"
T:a1 = integer { bits: 64, signed: false }
R:b1 = region { lifetime: static }
M:g1 = alloc { type: T:a1, region: R:b1 }
C:c0 = const { value: 0, type: T:a1 }
C:c1 = const { value: 42, type: T:a1 }
M:g2 = store { target: M:g1, index: C:c0, value: C:c1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    let m = ast.memories.iter().find(|m| m.id.as_str() == "M:g2").unwrap();
    match &m.op {
        MemoryOp::Store { target, index, value } => {
            assert_eq!(target.as_str(), "M:g1");
            assert_eq!(index.as_str(), "C:c0");
            assert_eq!(value.as_str(), "C:c1");
        }
        other => panic!("expected Store, got {:?}", other),
    }
}

// ===========================================================================
// 3. Einzelne Konstrukte — X-Node (Extern)
// ===========================================================================

#[test]
fn x_node_extern_c_abi() {
    let input = r#"
T:a1 = integer { bits: 64, signed: false }
T:a2 = unit
X:ext1 = extern { name: "malloc", abi: C, params: [T:a1], result: T:a1, effects: [MEM] }
entry: K:main
"#;
    let ast = parse_ok(input);
    assert_eq!(ast.externs.len(), 1);
    let x = &ast.externs[0];
    assert_eq!(x.id.as_str(), "X:ext1");
    assert_eq!(x.name, "malloc");
    assert!(matches!(x.abi, Abi::C));
    assert_eq!(x.params.len(), 1);
    assert_eq!(x.effects, vec!["MEM"]);
}

#[test]
fn x_node_extern_system_v() {
    let input = r#"
T:a1 = integer { bits: 64, signed: false }
X:ext1 = extern { name: "my_func", abi: SYSTEM_V, params: [T:a1, T:a1], result: T:a1, effects: [IO, MEM] }
entry: K:main
"#;
    let ast = parse_ok(input);
    let x = &ast.externs[0];
    assert!(matches!(x.abi, Abi::SystemV));
    assert_eq!(x.params.len(), 2);
    assert_eq!(x.effects, vec!["IO", "MEM"]);
}

#[test]
fn x_node_extern_no_effects() {
    let input = r#"
T:a1 = integer { bits: 64, signed: false }
X:ext1 = extern { name: "pure_fn", abi: C, params: [T:a1], result: T:a1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    assert!(ast.externs[0].effects.is_empty());
}

// ===========================================================================
// 4. Formeln
// ===========================================================================

#[test]
fn formula_simple_comparison_eq() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 1, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
V:e1 = contract { target: E:d1, pre: C:c1.val == 1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.contracts[0].clause {
        ContractClause::Pre { formula } => match formula {
            Formula::Comparison { left, op, right } => {
                assert!(matches!(op, CmpOp::Eq));
                // left is C:c1.val (field access)
                match left {
                    Expr::FieldAccess { node, fields } => {
                        assert_eq!(node.as_str(), "C:c1");
                        assert_eq!(fields, &["val"]);
                    }
                    other => panic!("expected FieldAccess, got {:?}", other),
                }
                // right is integer literal 1
                assert!(matches!(right, Expr::IntLit { value: 1 }));
            }
            other => panic!("expected Comparison, got {:?}", other),
        },
        other => panic!("expected Pre, got {:?}", other),
    }
}

#[test]
fn formula_and_or() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
C:c2 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
V:e1 = contract { target: E:d1, pre: C:c1.val > 0 AND C:c2.val < 100 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.contracts[0].clause {
        ContractClause::Pre { formula } => {
            assert!(matches!(formula, Formula::And { .. }));
            if let Formula::And { left, right } = formula {
                assert!(matches!(left.as_ref(), Formula::Comparison { .. }));
                assert!(matches!(right.as_ref(), Formula::Comparison { .. }));
                if let Formula::Comparison { op, .. } = left.as_ref() {
                    assert!(matches!(op, CmpOp::Gt));
                }
                if let Formula::Comparison { op, .. } = right.as_ref() {
                    assert!(matches!(op, CmpOp::Lt));
                }
            }
        }
        other => panic!("expected Pre, got {:?}", other),
    }
}

#[test]
fn formula_or() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
V:e1 = contract { target: E:d1, post: result >= 0 OR result == -1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.contracts[0].clause {
        ContractClause::Post { formula } => {
            assert!(matches!(formula, Formula::Or { .. }));
        }
        other => panic!("expected Post, got {:?}", other),
    }
}

#[test]
fn formula_forall() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
T:a2 = array { element: T:a1, max_length: 100 }
C:c1 = const { value: 0, type: T:a1 }
C:c2 = const { value: 0, type: T:a2 }
K:f1 = seq { steps: [C:c1] }
V:e1 = contract { target: K:f1, invariant: forall i in 0..C:c1.val: C:c2.arr[i] >= 0 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.contracts[0].clause {
        ContractClause::Invariant { formula } => {
            assert!(matches!(formula, Formula::Forall { .. }));
            if let Formula::Forall { var, range_start, .. } = formula {
                assert_eq!(var, "i");
                assert!(matches!(range_start, Expr::IntLit { value: 0 }));
            }
        }
        other => panic!("expected Invariant, got {:?}", other),
    }
}

#[test]
fn formula_nested_parens_and_or() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
V:e1 = contract { target: E:d1, post: (result >= 0 AND result < 100) OR result == -1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.contracts[0].clause {
        ContractClause::Post { formula } => {
            // The top level is OR: (... AND ...) OR (result == -1)
            assert!(matches!(formula, Formula::Or { .. }));
        }
        other => panic!("expected Post, got {:?}", other),
    }
}

#[test]
fn formula_arithmetic() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
V:e1 = contract { target: E:d1, post: result == C:c1.val * 2 + 1 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.contracts[0].clause {
        ContractClause::Post { formula } => {
            assert!(matches!(formula, Formula::Comparison { .. }));
            if let Formula::Comparison { left, op, right } = formula {
                assert!(matches!(op, CmpOp::Eq));
                assert!(matches!(left, Expr::Result));
                // right should be BinOp(BinOp(C:c1.val * 2) + 1)
                match right {
                    Expr::BinOp { op: ArithBinOp::Add, right: r, left: l } => {
                        assert!(matches!(r.as_ref(), Expr::IntLit { value: 1 }));
                        assert!(matches!(l.as_ref(), Expr::BinOp { op: ArithBinOp::Mul, .. }));
                    }
                    other => panic!("expected BinOp Add, got {:?}", other),
                }
            }
        }
        other => panic!("expected Post, got {:?}", other),
    }
}

#[test]
fn formula_not() {
    let input = r#"
T:a1 = boolean
C:c1 = const { value: true, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
V:e1 = contract { target: E:d1, pre: NOT C:c1.val == true }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.contracts[0].clause {
        ContractClause::Pre { formula } => {
            // NOT (C:c1.val == true)
            assert!(matches!(formula, Formula::Not { .. }));
        }
        other => panic!("expected Pre, got {:?}", other),
    }
}

#[test]
fn formula_comparison_ops() {
    // Test all comparison operators
    let ops = [
        ("==", "Eq"),
        ("!=", "Neq"),
        ("<", "Lt"),
        ("<=", "Lte"),
        (">", "Gt"),
        (">=", "Gte"),
    ];

    for (op_str, _label) in &ops {
        let input = format!(
            r#"
T:a1 = integer {{ bits: 32, signed: true }}
C:c1 = const {{ value: 0, type: T:a1 }}
E:d1 = syscall_exit {{ inputs: [C:c1], type: T:a1, effects: [PROC] }}
V:e1 = contract {{ target: E:d1, pre: C:c1.val {} 0 }}
entry: K:main
"#,
            op_str
        );
        let ast = parse_ok(&input);
        match &ast.contracts[0].clause {
            ContractClause::Pre { formula } => {
                assert!(
                    matches!(formula, Formula::Comparison { .. }),
                    "operator {} did not produce Comparison",
                    op_str
                );
            }
            other => panic!("expected Pre for op {}, got {:?}", op_str, other),
        }
    }
}

#[test]
fn formula_result_special() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
V:e1 = contract { target: E:d1, post: result > 0 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.contracts[0].clause {
        ContractClause::Post { formula } => {
            if let Formula::Comparison { left, .. } = formula {
                assert!(matches!(left, Expr::Result));
            } else {
                panic!("expected Comparison, got {:?}", formula);
            }
        }
        other => panic!("expected Post, got {:?}", other),
    }
}

#[test]
fn formula_state_special() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: true, type: T:a1 }
K:f1 = seq { steps: [C:c1] }
V:e1 = contract { target: K:f1, invariant: state.length <= 800 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.contracts[0].clause {
        ContractClause::Invariant { formula } => {
            assert!(matches!(formula, Formula::Comparison { .. }));
            if let Formula::Comparison { left, op, right } = formula {
                // state.length is parsed as field_access
                assert!(matches!(left, Expr::FieldAccess { .. }));
                assert!(matches!(op, CmpOp::Lte));
                assert!(matches!(right, Expr::IntLit { value: 800 }));
            }
        }
        other => panic!("expected Invariant, got {:?}", other),
    }
}

// ===========================================================================
// 4b. Formeln — edge cases
// ===========================================================================

#[test]
fn formula_bool_literal_assume_true() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
V:e1 = contract { target: E:d1, assume: true }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.contracts[0].clause {
        ContractClause::Assume { formula } => {
            assert!(matches!(formula, Formula::BoolLit { value: true }));
        }
        other => panic!("expected Assume, got {:?}", other),
    }
}

#[test]
fn formula_field_access_result_dot_size() {
    let input = r#"
T:a1 = integer { bits: 32, signed: true }
C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
V:e1 = contract { target: E:d1, post: result.size <= 16384 }
entry: K:main
"#;
    let ast = parse_ok(input);
    match &ast.contracts[0].clause {
        ContractClause::Post { formula } => {
            assert!(matches!(formula, Formula::Comparison { .. }));
            if let Formula::Comparison { left, .. } = formula {
                match left {
                    Expr::FieldAccess { node, fields } => {
                        assert_eq!(node.as_str(), "result");
                        assert_eq!(fields, &["size"]);
                    }
                    other => panic!("expected FieldAccess, got {:?}", other),
                }
            }
        }
        other => panic!("expected Post, got {:?}", other),
    }
}

// ===========================================================================
// Additional: all builtin types parse correctly
// ===========================================================================

#[test]
fn builtin_types_in_array() {
    let builtins = ["u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f32", "f64", "bool", "unit"];
    for b in &builtins {
        let input = format!(
            "T:a1 = array {{ element: {}, max_length: 10 }}\nentry: K:main\n",
            b
        );
        let ast = parse_ok(&input);
        match &ast.types[0].body {
            TypeBody::Array { element, .. } => {
                match element {
                    TypeRef::Builtin { name } => assert_eq!(name, *b, "builtin type mismatch"),
                    other => panic!("expected Builtin for {}, got {:?}", b, other),
                }
            }
            other => panic!("expected Array for {}, got {:?}", b, other),
        }
    }
}

// ===========================================================================
// Roundtrip: parse and serialize to JSON
// ===========================================================================

#[test]
fn serialize_minimal_to_json() {
    let input = include_str!("../testdata/minimal.ftl");
    let result = parse_ftl(input);
    let json = serde_json::to_string(&result).expect("serialization failed");
    assert!(json.contains("\"status\":\"OK\""));
    assert!(json.contains("\"entry\""));
}
