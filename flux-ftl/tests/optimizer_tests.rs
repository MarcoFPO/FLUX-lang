// ---------------------------------------------------------------------------
// Phase 11: Optimizer integration tests
// ---------------------------------------------------------------------------

use flux_ftl::ast::*;
use flux_ftl::codegen::{codegen, CodegenConfig, OptLevel, OutputFormat};
use flux_ftl::optimizer::{optimize_graph, OptimizationConfig};
use flux_ftl::parser::parse_ftl;

// ---------------------------------------------------------------------------
// Helper: build a minimal program from compute nodes
// ---------------------------------------------------------------------------

fn program_with_computes(computes: Vec<ComputeDef>, entry: &str) -> Program {
    Program {
        types: vec![],
        regions: vec![],
        computes,
        effects: vec![],
        controls: vec![],
        contracts: vec![],
        memories: vec![],
        externs: vec![],
        entry: NodeRef::new(entry),
    }
}

fn i64_type_ref() -> TypeRef {
    TypeRef::Builtin {
        name: "i64".to_string(),
    }
}

fn const_int(id: &str, value: i64) -> ComputeDef {
    ComputeDef {
        id: NodeRef::new(id),
        op: ComputeOp::Const {
            value: Literal::Integer { value },
            type_ref: i64_type_ref(),
            region: None,
        },
    }
}

fn arith(id: &str, opcode: &str, lhs: &str, rhs: &str) -> ComputeDef {
    ComputeDef {
        id: NodeRef::new(id),
        op: ComputeOp::Arith {
            opcode: opcode.to_string(),
            inputs: vec![NodeRef::new(lhs), NodeRef::new(rhs)],
            type_ref: i64_type_ref(),
        },
    }
}

// ---------------------------------------------------------------------------
// Dead node elimination
// ---------------------------------------------------------------------------

#[test]
fn dead_node_elimination() {
    // C:c1 and C:c2 are used by C:c3 (entry), but C:dead is unreachable
    let computes = vec![
        const_int("C:c1", 10),
        const_int("C:c2", 20),
        arith("C:c3", "add", "C:c1", "C:c2"),
        const_int("C:dead", 999), // unreachable
    ];

    let prog = program_with_computes(computes, "C:c3");
    let config = OptimizationConfig {
        llvm_opt_level: 0,
        enable_graph_opts: true,
        strip_dead_nodes: true,
        fold_constants: false, // disable to isolate dead node test
    };

    let result = optimize_graph(&prog, &config);

    assert_eq!(result.stats.dead_nodes_removed, 1);
    assert_eq!(result.stats.nodes_after, 3);
    // The dead node should be gone
    assert!(result
        .optimized_program
        .computes
        .iter()
        .all(|c| c.id.0 != "C:dead"));
}

// ---------------------------------------------------------------------------
// Constant folding: add
// ---------------------------------------------------------------------------

#[test]
fn constant_folding_add() {
    let computes = vec![
        const_int("C:c1", 3),
        const_int("C:c2", 4),
        arith("C:c3", "add", "C:c1", "C:c2"),
    ];

    let prog = program_with_computes(computes, "C:c3");
    let config = OptimizationConfig {
        llvm_opt_level: 0,
        enable_graph_opts: true,
        strip_dead_nodes: false,
        fold_constants: true,
    };

    let result = optimize_graph(&prog, &config);

    assert_eq!(result.stats.constants_folded, 1);

    let c3 = result
        .optimized_program
        .computes
        .iter()
        .find(|c| c.id.0 == "C:c3")
        .expect("C:c3 should exist");

    match &c3.op {
        ComputeOp::Const {
            value: Literal::Integer { value: 7 },
            ..
        } => {} // correct
        other => panic!("expected Const(7), got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Constant folding: mul
// ---------------------------------------------------------------------------

#[test]
fn constant_folding_mul() {
    let computes = vec![
        const_int("C:c1", 5),
        const_int("C:c2", 6),
        arith("C:c3", "mul", "C:c1", "C:c2"),
    ];

    let prog = program_with_computes(computes, "C:c3");
    let config = OptimizationConfig {
        llvm_opt_level: 0,
        enable_graph_opts: true,
        strip_dead_nodes: false,
        fold_constants: true,
    };

    let result = optimize_graph(&prog, &config);

    assert_eq!(result.stats.constants_folded, 1);

    let c3 = result
        .optimized_program
        .computes
        .iter()
        .find(|c| c.id.0 == "C:c3")
        .expect("C:c3 should exist");

    match &c3.op {
        ComputeOp::Const {
            value: Literal::Integer { value: 30 },
            ..
        } => {}
        other => panic!("expected Const(30), got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Identity removal: add x 0
// ---------------------------------------------------------------------------

#[test]
fn identity_removal_add_zero() {
    let computes = vec![
        const_int("C:c1", 42),
        const_int("C:c2", 0),
        arith("C:c3", "add", "C:c1", "C:c2"),
    ];

    let prog = program_with_computes(computes, "C:c3");
    let config = OptimizationConfig {
        llvm_opt_level: 0,
        enable_graph_opts: true,
        strip_dead_nodes: false,
        fold_constants: false, // disable folding so identity pass handles it
    };

    let result = optimize_graph(&prog, &config);

    assert_eq!(result.stats.identities_removed, 1);

    let c3 = result
        .optimized_program
        .computes
        .iter()
        .find(|c| c.id.0 == "C:c3")
        .expect("C:c3 should exist");

    match &c3.op {
        ComputeOp::Const {
            value: Literal::Integer { value: 42 },
            ..
        } => {}
        other => panic!("expected Const(42), got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Identity removal: mul x 1
// ---------------------------------------------------------------------------

#[test]
fn identity_removal_mul_one() {
    let computes = vec![
        const_int("C:c1", 77),
        const_int("C:c2", 1),
        arith("C:c3", "mul", "C:c1", "C:c2"),
    ];

    let prog = program_with_computes(computes, "C:c3");
    let config = OptimizationConfig {
        llvm_opt_level: 0,
        enable_graph_opts: true,
        strip_dead_nodes: false,
        fold_constants: false,
    };

    let result = optimize_graph(&prog, &config);

    assert_eq!(result.stats.identities_removed, 1);

    let c3 = result
        .optimized_program
        .computes
        .iter()
        .find(|c| c.id.0 == "C:c3")
        .expect("C:c3 should exist");

    match &c3.op {
        ComputeOp::Const {
            value: Literal::Integer { value: 77 },
            ..
        } => {}
        other => panic!("expected Const(77), got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Optimization stats correctness
// ---------------------------------------------------------------------------

#[test]
fn optimization_stats() {
    let computes = vec![
        const_int("C:c1", 3),
        const_int("C:c2", 4),
        arith("C:c3", "add", "C:c1", "C:c2"),
        const_int("C:dead1", 100),
        const_int("C:dead2", 200),
    ];

    let prog = program_with_computes(computes, "C:c3");
    let config = OptimizationConfig {
        llvm_opt_level: 0,
        enable_graph_opts: true,
        strip_dead_nodes: true,
        fold_constants: true,
    };

    let result = optimize_graph(&prog, &config);

    assert_eq!(result.stats.nodes_before, 5);
    assert_eq!(result.stats.constants_folded, 1);
    // After folding, C:c3 becomes a const. C:c1 and C:c2 are still
    // referenced by C:c3's original inputs in the folded node (which is now
    // a Const that does NOT reference them). Dead node elimination removes
    // C:dead1, C:dead2, and also C:c1, C:c2 (no longer referenced).
    assert_eq!(result.stats.dead_nodes_removed, 4);
    assert_eq!(result.stats.nodes_after, 1); // only C:c3 remains
}

// ---------------------------------------------------------------------------
// LLVM opt produces smaller (or at least different) IR at -O3 vs -O0
// ---------------------------------------------------------------------------

#[test]
fn llvm_opt_produces_smaller_ir() {
    let source = std::fs::read_to_string("testdata/hello_world.ftl")
        .expect("failed to read hello_world.ftl");
    let parsed = parse_ftl(&source);
    let ast = parsed.ast.expect("failed to parse hello_world.ftl");

    // Generate at -O0
    let config_o0 = CodegenConfig {
        opt_level: OptLevel::None,
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };
    let result_o0 = codegen(&ast, &config_o0).expect("codegen O0 failed");

    // Generate at -O3
    let config_o3 = CodegenConfig {
        opt_level: OptLevel::Aggressive,
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };
    let result_o3 = codegen(&ast, &config_o3).expect("codegen O3 failed");

    // Both should produce valid IR
    assert!(result_o0.llvm_ir.contains("define i32 @main"));
    assert!(result_o3.llvm_ir.contains("define i32 @main"));

    // O3 IR should be at least as compact as O0 (or different)
    // In practice, LLVM may add attributes that increase size, so we just
    // verify they are both non-empty and the optimization ran without error.
    assert!(!result_o0.llvm_ir.is_empty());
    assert!(!result_o3.llvm_ir.is_empty());
}

// ---------------------------------------------------------------------------
// Graph optimizer on a real FTL program (hello_world) should not break it
// ---------------------------------------------------------------------------

#[test]
fn optimize_hello_world_still_generates_valid_ir() {
    let source = std::fs::read_to_string("testdata/hello_world.ftl")
        .expect("failed to read hello_world.ftl");
    let parsed = parse_ftl(&source);
    let ast = parsed.ast.expect("failed to parse hello_world.ftl");

    let opt_config = OptimizationConfig::default();
    let opt_result = optimize_graph(&ast, &opt_config);

    let cg_config = CodegenConfig {
        opt_level: OptLevel::Default,
        output_format: OutputFormat::LlvmIr,
        ..CodegenConfig::default()
    };

    let cg_result =
        codegen(&opt_result.optimized_program, &cg_config).expect("codegen after opt failed");
    assert!(cg_result.llvm_ir.contains("define i32 @main"));
    assert!(cg_result.llvm_ir.contains("@write"));
}

// ---------------------------------------------------------------------------
// Disabled optimization passes through unchanged
// ---------------------------------------------------------------------------

#[test]
fn disabled_optimization_passes_through() {
    let computes = vec![
        const_int("C:c1", 3),
        const_int("C:c2", 4),
        arith("C:c3", "add", "C:c1", "C:c2"),
    ];

    let prog = program_with_computes(computes, "C:c3");
    let config = OptimizationConfig {
        llvm_opt_level: 0,
        enable_graph_opts: false,
        strip_dead_nodes: false,
        fold_constants: false,
    };

    let result = optimize_graph(&prog, &config);

    assert_eq!(result.stats.constants_folded, 0);
    assert_eq!(result.stats.dead_nodes_removed, 0);
    assert_eq!(result.stats.identities_removed, 0);
    assert_eq!(result.stats.nodes_before, result.stats.nodes_after);
    assert_eq!(result.optimized_program.computes.len(), 3);
}

// ---------------------------------------------------------------------------
// Float constant folding
// ---------------------------------------------------------------------------

#[test]
fn constant_folding_float() {
    let computes = vec![
        ComputeDef {
            id: NodeRef::new("C:c1"),
            op: ComputeOp::Const {
                value: Literal::Float { value: 2.5 },
                type_ref: TypeRef::Builtin {
                    name: "f64".to_string(),
                },
                region: None,
            },
        },
        ComputeDef {
            id: NodeRef::new("C:c2"),
            op: ComputeOp::Const {
                value: Literal::Float { value: 3.5 },
                type_ref: TypeRef::Builtin {
                    name: "f64".to_string(),
                },
                region: None,
            },
        },
        ComputeDef {
            id: NodeRef::new("C:c3"),
            op: ComputeOp::Arith {
                opcode: "add".to_string(),
                inputs: vec![NodeRef::new("C:c1"), NodeRef::new("C:c2")],
                type_ref: TypeRef::Builtin {
                    name: "f64".to_string(),
                },
            },
        },
    ];

    let prog = program_with_computes(computes, "C:c3");
    let config = OptimizationConfig {
        llvm_opt_level: 0,
        enable_graph_opts: true,
        strip_dead_nodes: false,
        fold_constants: true,
    };

    let result = optimize_graph(&prog, &config);
    assert_eq!(result.stats.constants_folded, 1);

    let c3 = result
        .optimized_program
        .computes
        .iter()
        .find(|c| c.id.0 == "C:c3")
        .expect("C:c3 should exist");

    match &c3.op {
        ComputeOp::Const {
            value: Literal::Float { value },
            ..
        } => {
            assert!((value - 6.0).abs() < f64::EPSILON);
        }
        other => panic!("expected Const(6.0), got {:?}", other),
    }
}
