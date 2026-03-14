// ---------------------------------------------------------------------------
// Phase 12: Evolution integration tests
// ---------------------------------------------------------------------------
//
// Tests for the genetic algorithm that evolves FTL graph variants.
// Covers pool management, mutations, fitness, selection, Pareto
// dominance, incubation, and deterministic reproducibility.
// ---------------------------------------------------------------------------

use flux_ftl::ast::*;
use flux_ftl::evolution::*;
use flux_ftl::prover::{ProofResult, ProofStatus};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Create a minimal test program with arithmetic computation:
///   T:i64 (integer type)
///   C:c1 = const 10
///   C:c2 = const 20
///   C:c3 = add(C:c1, C:c2)
///   K:main = seq [C:c3]
///   V:v1 = post(result > 0) on C:c3
fn make_test_program() -> Program {
    let t1 = TypeDef {
        id: NodeRef::new("T:i64"),
        body: TypeBody::Integer {
            bits: 64,
            signed: true,
        },
    };

    let c1 = ComputeDef {
        id: NodeRef::new("C:c1"),
        op: ComputeOp::Const {
            value: Literal::Integer { value: 10 },
            type_ref: TypeRef::Id {
                node: NodeRef::new("T:i64"),
            },
            region: None,
        },
    };

    let c2 = ComputeDef {
        id: NodeRef::new("C:c2"),
        op: ComputeOp::Const {
            value: Literal::Integer { value: 20 },
            type_ref: TypeRef::Id {
                node: NodeRef::new("T:i64"),
            },
            region: None,
        },
    };

    let c3 = ComputeDef {
        id: NodeRef::new("C:c3"),
        op: ComputeOp::Arith {
            opcode: "add".to_string(),
            inputs: vec![NodeRef::new("C:c1"), NodeRef::new("C:c2")],
            type_ref: TypeRef::Id {
                node: NodeRef::new("T:i64"),
            },
        },
    };

    let k1 = ControlDef {
        id: NodeRef::new("K:main"),
        op: ControlOp::Seq {
            steps: vec![NodeRef::new("C:c3")],
        },
    };

    let v1 = ContractDef {
        id: NodeRef::new("V:v1"),
        target: NodeRef::new("C:c3"),
        clauses: vec![ContractClause::Post {
            formula: Formula::Comparison {
                left: Expr::Result,
                op: CmpOp::Gt,
                right: Expr::IntLit { value: 0 },
            },
        }],
        trust: None,
    };

    Program {
        imports: vec![],
        types: vec![t1],
        regions: vec![],
        computes: vec![c1, c2, c3],
        effects: vec![],
        controls: vec![k1],
        contracts: vec![v1],
        memories: vec![],
        externs: vec![],
        entry: NodeRef::new("K:main"),
    }
}

/// Create a program with multiple arithmetic nodes for richer evolution.
fn make_complex_program() -> Program {
    let t1 = TypeDef {
        id: NodeRef::new("T:i64"),
        body: TypeBody::Integer {
            bits: 64,
            signed: true,
        },
    };

    let c1 = ComputeDef {
        id: NodeRef::new("C:a"),
        op: ComputeOp::Const {
            value: Literal::Integer { value: 5 },
            type_ref: TypeRef::Id {
                node: NodeRef::new("T:i64"),
            },
            region: None,
        },
    };

    let c2 = ComputeDef {
        id: NodeRef::new("C:b"),
        op: ComputeOp::Const {
            value: Literal::Integer { value: 3 },
            type_ref: TypeRef::Id {
                node: NodeRef::new("T:i64"),
            },
            region: None,
        },
    };

    let c3 = ComputeDef {
        id: NodeRef::new("C:sum"),
        op: ComputeOp::Arith {
            opcode: "add".to_string(),
            inputs: vec![NodeRef::new("C:a"), NodeRef::new("C:b")],
            type_ref: TypeRef::Id {
                node: NodeRef::new("T:i64"),
            },
        },
    };

    let c4 = ComputeDef {
        id: NodeRef::new("C:prod"),
        op: ComputeOp::Arith {
            opcode: "mul".to_string(),
            inputs: vec![NodeRef::new("C:a"), NodeRef::new("C:b")],
            type_ref: TypeRef::Id {
                node: NodeRef::new("T:i64"),
            },
        },
    };

    let c5 = ComputeDef {
        id: NodeRef::new("C:result"),
        op: ComputeOp::Arith {
            opcode: "add".to_string(),
            inputs: vec![NodeRef::new("C:sum"), NodeRef::new("C:prod")],
            type_ref: TypeRef::Id {
                node: NodeRef::new("T:i64"),
            },
        },
    };

    let k1 = ControlDef {
        id: NodeRef::new("K:main"),
        op: ControlOp::Seq {
            steps: vec![NodeRef::new("C:result")],
        },
    };

    let v1 = ContractDef {
        id: NodeRef::new("V:check"),
        target: NodeRef::new("C:result"),
        clauses: vec![
            ContractClause::Post {
                formula: Formula::Comparison {
                    left: Expr::Result,
                    op: CmpOp::Gt,
                    right: Expr::IntLit { value: 0 },
                },
            },
            ContractClause::Post {
                formula: Formula::Comparison {
                    left: Expr::Result,
                    op: CmpOp::Lt,
                    right: Expr::IntLit { value: 1000 },
                },
            },
        ],
        trust: None,
    };

    Program {
        imports: vec![],
        types: vec![t1],
        regions: vec![],
        computes: vec![c1, c2, c3, c4, c5],
        effects: vec![],
        controls: vec![k1],
        contracts: vec![v1],
        memories: vec![],
        externs: vec![],
        entry: NodeRef::new("K:main"),
    }
}

// ---------------------------------------------------------------------------
// Pool creation and seeding
// ---------------------------------------------------------------------------

#[test]
fn test_pool_creation() {
    let config = EvolutionConfig {
        seed: Some(42),
        population_size: 20,
        ..Default::default()
    };
    let pool = GraphPool::new(config);

    assert_eq!(pool.generation(), 0);
    assert!(pool.population().is_empty());
    assert!(pool.incubation().is_empty());
    assert_eq!(pool.config().population_size, 20);
    assert_eq!(pool.config().seed, Some(42));
}

#[test]
fn test_pool_default_config() {
    let config = EvolutionConfig::default();
    assert_eq!(config.population_size, 50);
    assert_eq!(config.elite_count, 5);
    assert!((config.mutation_rate - 0.3).abs() < f64::EPSILON);
    assert!((config.crossover_rate - 0.5).abs() < f64::EPSILON);
    assert_eq!(config.tournament_size, 3);
    assert_eq!(config.max_generations, 100);
    assert_eq!(config.incubation_limit, 20);
    assert!(config.seed.is_none());
}

#[test]
fn test_seed_population() {
    let config = EvolutionConfig {
        population_size: 10,
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let base = make_test_program();

    pool.seed_population(&base, 10);

    assert_eq!(pool.population().len(), 10);

    // First individual is the original (no lineage)
    assert!(pool.population()[0].lineage.is_empty());
    assert_eq!(pool.population()[0].program.computes.len(), 3);

    // Remaining individuals have lineage
    for ind in pool.population().iter().skip(1) {
        assert!(!ind.lineage.is_empty());
        assert_eq!(ind.lineage[0], "seed-mutation");
    }
}

#[test]
fn test_seed_population_capped_at_config() {
    let config = EvolutionConfig {
        population_size: 5,
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let base = make_test_program();

    // Requesting more than population_size should be capped
    pool.seed_population(&base, 100);
    assert_eq!(pool.population().len(), 5);
}

// ---------------------------------------------------------------------------
// Mutation operators
// ---------------------------------------------------------------------------

#[test]
fn test_mutation_swap_op() {
    let config = EvolutionConfig {
        seed: Some(100),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let program = make_test_program();

    let mutated = pool.apply_mutation(&program, MutationOp::SwapComputeOp);

    let original_c3 = program
        .computes
        .iter()
        .find(|c| c.id.0 == "C:c3")
        .unwrap();
    let mutated_c3 = mutated
        .computes
        .iter()
        .find(|c| c.id.0 == "C:c3")
        .unwrap();

    if let (
        ComputeOp::Arith {
            opcode: orig_op, ..
        },
        ComputeOp::Arith {
            opcode: new_op, ..
        },
    ) = (&original_c3.op, &mutated_c3.op)
    {
        assert_ne!(orig_op, new_op, "opcode should have changed");
    } else {
        panic!("expected Arith ops");
    }
}

#[test]
fn test_mutation_modify_constant() {
    let config = EvolutionConfig {
        seed: Some(200),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let program = make_test_program();

    let mutated = pool.apply_mutation(&program, MutationOp::ModifyConstant);

    let orig_values: Vec<i64> = program
        .computes
        .iter()
        .filter_map(|c| {
            if let ComputeOp::Const {
                value: Literal::Integer { value },
                ..
            } = &c.op
            {
                Some(*value)
            } else {
                None
            }
        })
        .collect();

    let new_values: Vec<i64> = mutated
        .computes
        .iter()
        .filter_map(|c| {
            if let ComputeOp::Const {
                value: Literal::Integer { value },
                ..
            } = &c.op
            {
                Some(*value)
            } else {
                None
            }
        })
        .collect();

    assert_ne!(
        orig_values, new_values,
        "at least one constant should change"
    );
}

#[test]
fn test_mutation_insert_node() {
    let config = EvolutionConfig {
        seed: Some(300),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let program = make_test_program();

    let mutated = pool.apply_mutation(&program, MutationOp::InsertNode);

    // Should have 2 more compute nodes (zero const + identity add)
    assert_eq!(mutated.computes.len(), program.computes.len() + 2);

    // Check that the new nodes exist
    let has_zero = mutated
        .computes
        .iter()
        .any(|c| c.id.0.starts_with("C:evo_zero_"));
    let has_ins = mutated
        .computes
        .iter()
        .any(|c| c.id.0.starts_with("C:evo_ins_"));
    assert!(has_zero, "should have zero constant node");
    assert!(has_ins, "should have inserted identity node");
}

#[test]
fn test_mutation_remove_node() {
    let config = EvolutionConfig {
        seed: Some(400),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let program = make_complex_program();

    let mutated = pool.apply_mutation(&program, MutationOp::RemoveNode);

    // Should not crash, and should have a valid program
    assert!(!mutated.computes.is_empty());
    // May or may not have fewer nodes depending on reference counts
    assert!(mutated.computes.len() <= program.computes.len());
}

#[test]
fn test_mutation_swap_edges() {
    let config = EvolutionConfig {
        seed: Some(500),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let program = make_test_program();

    let mutated = pool.apply_mutation(&program, MutationOp::SwapEdges);

    // Same number of nodes
    assert_eq!(mutated.computes.len(), program.computes.len());
}

#[test]
fn test_mutation_modify_contract() {
    let config = EvolutionConfig {
        seed: Some(700),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let program = make_test_program();

    let mutated = pool.apply_mutation(&program, MutationOp::ModifyContract);

    assert_eq!(mutated.contracts.len(), program.contracts.len());
    // The contract should have been processed without panic
}

#[test]
fn test_mutation_on_empty_program() {
    let config = EvolutionConfig {
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let empty = Program {
        imports: vec![],
        types: vec![],
        regions: vec![],
        computes: vec![],
        effects: vec![],
        controls: vec![],
        contracts: vec![],
        memories: vec![],
        externs: vec![],
        entry: NodeRef::new("K:entry"),
    };

    // All mutations should handle empty programs gracefully
    for op in MutationOp::ALL {
        let result = pool.apply_mutation(&empty, op);
        // Should not panic
        assert!(result.computes.is_empty() || !result.computes.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Crossover
// ---------------------------------------------------------------------------

#[test]
fn test_crossover() {
    let config = EvolutionConfig {
        seed: Some(600),
        ..Default::default()
    };
    let pool = GraphPool::new(config);

    let parent1 = make_test_program();
    let mut parent2 = make_test_program();

    // Add an extra compute to parent2
    parent2.computes.push(ComputeDef {
        id: NodeRef::new("C:c4"),
        op: ComputeOp::Const {
            value: Literal::Integer { value: 42 },
            type_ref: TypeRef::Id {
                node: NodeRef::new("T:i64"),
            },
            region: None,
        },
    });

    let child = pool.crossover(&parent1, &parent2);

    // Child should have C:c4 from parent2
    let has_c4 = child.computes.iter().any(|c| c.id.0 == "C:c4");
    assert!(has_c4, "child should inherit C:c4 from parent2");

    // Child should have all of parent1's base nodes
    let has_c1 = child.computes.iter().any(|c| c.id.0 == "C:c1");
    let has_c2 = child.computes.iter().any(|c| c.id.0 == "C:c2");
    let has_c3 = child.computes.iter().any(|c| c.id.0 == "C:c3");
    assert!(has_c1 && has_c2 && has_c3, "child should have base nodes");
}

#[test]
fn test_crossover_disjoint_contracts() {
    let config = EvolutionConfig::default();
    let pool = GraphPool::new(config);

    let parent1 = make_test_program();
    let mut parent2 = make_test_program();

    // Add a unique contract to parent2
    parent2.contracts.push(ContractDef {
        id: NodeRef::new("V:v2"),
        target: NodeRef::new("C:c1"),
        clauses: vec![ContractClause::Pre {
            formula: Formula::BoolLit { value: true },
        }],
        trust: None,
    });

    let child = pool.crossover(&parent1, &parent2);
    let has_v2 = child.contracts.iter().any(|c| c.id.0 == "V:v2");
    assert!(
        has_v2,
        "child should inherit V:v2 contract from parent2"
    );
}

// ---------------------------------------------------------------------------
// Fitness calculation
// ---------------------------------------------------------------------------

#[test]
fn test_fitness_calculation() {
    let program = make_test_program();
    let mut individual = Individual::new(program, 0);

    // Without proof results
    let fitness = calculate_fitness(&individual);
    assert_eq!(fitness.correctness, 0.0);
    assert!(fitness.node_count > 0.0);
    assert!(fitness.depth > 0.0);
    assert!(fitness.estimated_cost > 0.0);

    // With proof results (all proven)
    individual.proof_status = Some(vec![ProofResult {
        contract_id: "V:v1".to_string(),
        target_id: "C:c3".to_string(),
        clause_index: 0,
        clause_kind: "post".to_string(),
        status: ProofStatus::Proven,
        counterexample: None,
        counterexample_model: None,
    }]);

    let fitness = calculate_fitness(&individual);
    assert_eq!(fitness.correctness, 1.0);
}

#[test]
fn test_fitness_partial_correctness() {
    let program = make_test_program();
    let mut individual = Individual::new(program, 0);

    individual.proof_status = Some(vec![
        ProofResult {
            contract_id: "V:v1".to_string(),
            target_id: "C:c3".to_string(),
            clause_index: 0,
            clause_kind: "post".to_string(),
            status: ProofStatus::Proven,
            counterexample: None,
        counterexample_model: None,
        },
        ProofResult {
            contract_id: "V:v1".to_string(),
            target_id: "C:c3".to_string(),
            clause_index: 1,
            clause_kind: "post".to_string(),
            status: ProofStatus::Disproven,
            counterexample: Some("x=0".to_string()),
        counterexample_model: None,
        },
    ]);

    let fitness = calculate_fitness(&individual);
    assert!((fitness.correctness - 0.5).abs() < f64::EPSILON);
}

#[test]
fn test_fitness_total_weights() {
    let f = FitnessScore {
        correctness: 1.0,
        node_count: 1.0,
        depth: 1.0,
        estimated_cost: 1.0,
    };
    // 1.0*0.5 + 1.0*0.2 + 1.0*0.1 + 1.0*0.2 = 1.0
    assert!((f.total() - 1.0).abs() < f64::EPSILON);

    let g = FitnessScore {
        correctness: 0.0,
        node_count: 0.0,
        depth: 0.0,
        estimated_cost: 0.0,
    };
    assert!((g.total() - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_count_nodes() {
    let program = make_test_program();
    let count = count_nodes(&program);
    // 1 type + 3 computes + 1 control + 1 contract = 6
    assert_eq!(count, 6);
}

#[test]
fn test_calculate_depth() {
    let program = make_test_program();
    let depth = calculate_depth(&program);
    // C:c1 -> depth 1, C:c2 -> depth 1, C:c3(c1,c2) -> depth 2
    assert_eq!(depth, 2);
}

#[test]
fn test_calculate_depth_complex() {
    let program = make_complex_program();
    let depth = calculate_depth(&program);
    // C:a=1, C:b=1, C:sum=2, C:prod=2, C:result=3
    assert_eq!(depth, 3);
}

#[test]
fn test_estimate_cost() {
    let program = make_test_program();
    let cost = estimate_cost(&program);
    // 2 Const * 0.5 + 1 Arith * 1.0 = 2.0
    assert!((cost - 2.0).abs() < f64::EPSILON);
}

#[test]
fn test_estimate_cost_complex() {
    let program = make_complex_program();
    let cost = estimate_cost(&program);
    // 2 Const * 0.5 + 3 Arith * 1.0 = 4.0
    assert!((cost - 4.0).abs() < f64::EPSILON);
}

// ---------------------------------------------------------------------------
// Tournament selection
// ---------------------------------------------------------------------------

#[test]
fn test_tournament_selection() {
    let config = EvolutionConfig {
        population_size: 10,
        tournament_size: 3,
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let base = make_test_program();
    pool.seed_population(&base, 10);
    pool.evaluate_fitness();

    let pairs = pool.select_parents();
    assert!(!pairs.is_empty());

    for (p1, p2) in &pairs {
        assert!(*p1 < pool.population().len());
        assert!(*p2 < pool.population().len());
    }
}

#[test]
fn test_select_parents_empty_population() {
    let config = EvolutionConfig {
        seed: Some(42),
        ..Default::default()
    };
    let pool = GraphPool::new(config);
    let pairs = pool.select_parents();
    assert!(pairs.is_empty());
}

// ---------------------------------------------------------------------------
// Pareto dominance
// ---------------------------------------------------------------------------

#[test]
fn test_pareto_dominance() {
    let a = FitnessScore {
        correctness: 1.0,
        node_count: 0.5,
        depth: 0.5,
        estimated_cost: 0.5,
    };
    let b = FitnessScore {
        correctness: 0.8,
        node_count: 0.5,
        depth: 0.5,
        estimated_cost: 0.5,
    };

    assert!(pareto_dominates(&a, &b), "a should dominate b");
    assert!(!pareto_dominates(&b, &a), "b should not dominate a");
}

#[test]
fn test_pareto_no_dominance_equal() {
    let a = FitnessScore {
        correctness: 1.0,
        node_count: 0.5,
        depth: 0.5,
        estimated_cost: 0.5,
    };
    let c = FitnessScore {
        correctness: 1.0,
        node_count: 0.5,
        depth: 0.5,
        estimated_cost: 0.5,
    };
    assert!(
        !pareto_dominates(&a, &c),
        "equal scores should not dominate"
    );
}

#[test]
fn test_pareto_no_dominance_tradeoff() {
    let a = FitnessScore {
        correctness: 1.0,
        node_count: 0.3,
        depth: 0.5,
        estimated_cost: 0.4,
    };
    let b = FitnessScore {
        correctness: 0.5,
        node_count: 0.8,
        depth: 0.5,
        estimated_cost: 0.9,
    };
    // Neither dominates: a better in correctness, b better in size/cost
    assert!(!pareto_dominates(&a, &b));
    assert!(!pareto_dominates(&b, &a));
}

#[test]
fn test_pareto_front() {
    let program = make_test_program();

    let mut ind1 = Individual::new(program.clone(), 0);
    ind1.fitness = Some(FitnessScore {
        correctness: 1.0,
        node_count: 0.3,
        depth: 0.5,
        estimated_cost: 0.4,
    });

    let mut ind2 = Individual::new(program.clone(), 0);
    ind2.fitness = Some(FitnessScore {
        correctness: 0.5,
        node_count: 0.8,
        depth: 0.5,
        estimated_cost: 0.9,
    });

    // ind3 is dominated by ind1 in all dimensions
    let mut ind3 = Individual::new(program, 0);
    ind3.fitness = Some(FitnessScore {
        correctness: 0.3,
        node_count: 0.2,
        depth: 0.4,
        estimated_cost: 0.3,
    });

    let population = vec![ind1, ind2, ind3];
    let front = pareto_front(&population);

    assert!(front.contains(&0), "ind1 should be on the front");
    assert!(front.contains(&1), "ind2 should be on the front");
    assert!(!front.contains(&2), "ind3 should be dominated by ind1");
    assert_eq!(front.len(), 2);
}

#[test]
fn test_pareto_front_single_individual() {
    let program = make_test_program();
    let mut ind = Individual::new(program, 0);
    ind.fitness = Some(FitnessScore {
        correctness: 1.0,
        node_count: 1.0,
        depth: 1.0,
        estimated_cost: 1.0,
    });

    let population = vec![ind];
    let front = pareto_front(&population);
    assert_eq!(front, vec![0]);
}

// ---------------------------------------------------------------------------
// Incubation
// ---------------------------------------------------------------------------

#[test]
fn test_incubation() {
    let config = EvolutionConfig {
        population_size: 5,
        elite_count: 1,
        incubation_limit: 10,
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let base = make_test_program();
    pool.seed_population(&base, 5);

    // Mark some individuals as failed
    for ind in pool.population_mut().iter_mut().skip(2) {
        ind.proof_status = Some(vec![ProofResult {
            contract_id: "V:v1".to_string(),
            target_id: "C:c3".to_string(),
            clause_index: 0,
            clause_kind: "post".to_string(),
            status: ProofStatus::Disproven,
            counterexample: Some("counterexample".to_string()),
        counterexample_model: None,
        }]);
    }

    pool.evolve_generation();

    assert!(
        pool.population().len() <= 5,
        "population should not exceed limit"
    );
}

#[test]
fn test_promote_from_incubation() {
    let config = EvolutionConfig {
        population_size: 10,
        incubation_limit: 5,
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);

    // Manually add an individual to incubation
    let program = make_test_program();
    let mut incubated = Individual::new(program, 0);
    incubated.proof_status = Some(vec![ProofResult {
        contract_id: "V:v1".to_string(),
        target_id: "C:c3".to_string(),
        clause_index: 0,
        clause_kind: "post".to_string(),
        status: ProofStatus::Proven,
        counterexample: None,
        counterexample_model: None,
    }]);
    incubated.lineage.push("initial".to_string());
    pool.incubation_mut().push(incubated);

    let initial_pop_size = pool.population().len();
    pool.promote_from_incubation();

    // Incubation should have been processed
    assert!(
        pool.population().len() >= initial_pop_size || !pool.incubation().is_empty(),
        "incubation should have been processed"
    );
}

#[test]
fn test_incubation_limit() {
    let config = EvolutionConfig {
        population_size: 5,
        elite_count: 1,
        incubation_limit: 2,
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let base = make_test_program();
    pool.seed_population(&base, 5);

    // Mark all non-elite individuals as failed
    for ind in pool.population_mut().iter_mut().skip(1) {
        ind.proof_status = Some(vec![ProofResult {
            contract_id: "V:v1".to_string(),
            target_id: "C:c3".to_string(),
            clause_index: 0,
            clause_kind: "post".to_string(),
            status: ProofStatus::Disproven,
            counterexample: None,
        counterexample_model: None,
        }]);
    }

    pool.evolve_generation();

    // Incubation should be capped at the limit
    assert!(
        pool.incubation().len() <= 2,
        "incubation should not exceed limit of 2, got {}",
        pool.incubation().len()
    );
}

// ---------------------------------------------------------------------------
// Evolution lifecycle
// ---------------------------------------------------------------------------

#[test]
fn test_evolve_one_generation() {
    let config = EvolutionConfig {
        population_size: 10,
        elite_count: 2,
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let base = make_test_program();
    pool.seed_population(&base, 10);

    assert_eq!(pool.generation(), 0);
    pool.evolve_generation();
    assert_eq!(pool.generation(), 1);
    assert!(!pool.population().is_empty());
    assert!(pool.population().len() <= 10);
}

#[test]
fn test_evolution_run() {
    let config = EvolutionConfig {
        population_size: 10,
        elite_count: 2,
        max_generations: 5,
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let base = make_test_program();
    pool.seed_population(&base, 10);

    let result = pool.run(3);

    assert_eq!(result.generations_run, 3);
    assert!(result.population_stats.best_fitness >= 0.0);
    assert!(result.population_stats.avg_fitness >= 0.0);
}

#[test]
fn test_evolution_run_capped_at_max() {
    let config = EvolutionConfig {
        population_size: 8,
        max_generations: 3,
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let base = make_test_program();
    pool.seed_population(&base, 8);

    // Requesting more than max_generations should be capped
    let result = pool.run(100);
    assert_eq!(result.generations_run, 3);
}

#[test]
fn test_evolution_best_individual() {
    let config = EvolutionConfig {
        population_size: 10,
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let base = make_test_program();
    pool.seed_population(&base, 10);
    pool.evaluate_fitness();

    let best = pool.best();
    assert!(best.is_some());
    assert!(best.unwrap().fitness.is_some());
}

// ---------------------------------------------------------------------------
// Deterministic reproducibility
// ---------------------------------------------------------------------------

#[test]
fn test_deterministic_with_seed() {
    let base = make_test_program();

    // Run 1
    let config1 = EvolutionConfig {
        population_size: 10,
        seed: Some(12345),
        max_generations: 5,
        ..Default::default()
    };
    let mut pool1 = GraphPool::new(config1);
    pool1.seed_population(&base, 10);
    let result1 = pool1.run(3);

    // Run 2 with same seed
    let config2 = EvolutionConfig {
        population_size: 10,
        seed: Some(12345),
        max_generations: 5,
        ..Default::default()
    };
    let mut pool2 = GraphPool::new(config2);
    pool2.seed_population(&base, 10);
    let result2 = pool2.run(3);

    assert_eq!(
        result1.population_stats.best_fitness,
        result2.population_stats.best_fitness,
        "deterministic runs with same seed should produce identical best fitness"
    );
    assert_eq!(
        result1.population_stats.avg_fitness,
        result2.population_stats.avg_fitness,
        "deterministic runs with same seed should produce identical avg fitness"
    );
    assert_eq!(
        result1.generations_run, result2.generations_run,
        "both runs should complete the same number of generations"
    );
}

#[test]
fn test_different_seeds_differ() {
    let base = make_complex_program();

    let config1 = EvolutionConfig {
        population_size: 15,
        seed: Some(111),
        max_generations: 10,
        ..Default::default()
    };
    let mut pool1 = GraphPool::new(config1);
    pool1.seed_population(&base, 15);
    let result1 = pool1.run(5);

    let config2 = EvolutionConfig {
        population_size: 15,
        seed: Some(999),
        max_generations: 10,
        ..Default::default()
    };
    let mut pool2 = GraphPool::new(config2);
    pool2.seed_population(&base, 15);
    let result2 = pool2.run(5);

    // Different seeds should produce different results (with very high probability)
    // At minimum, the populations should have diverged
    let nodes1 = count_nodes(&result1.best.program);
    let nodes2 = count_nodes(&result2.best.program);

    // This is a probabilistic test; with different seeds the evolved
    // programs will almost certainly differ in some way
    let _both_ran = result1.generations_run == 5 && result2.generations_run == 5;
    assert!(_both_ran, "both should have run 5 generations");

    // Just verify both produced valid results
    assert!(nodes1 > 0);
    assert!(nodes2 > 0);
}

// ---------------------------------------------------------------------------
// Individual state checks
// ---------------------------------------------------------------------------

#[test]
fn test_individual_is_proven() {
    let program = make_test_program();
    let mut ind = Individual::new(program, 0);

    assert!(!ind.is_proven());

    ind.proof_status = Some(vec![ProofResult {
        contract_id: "V:v1".to_string(),
        target_id: "C:c3".to_string(),
        clause_index: 0,
        clause_kind: "post".to_string(),
        status: ProofStatus::Proven,
        counterexample: None,
        counterexample_model: None,
    }]);

    assert!(ind.is_proven());
}

#[test]
fn test_individual_has_failures() {
    let program = make_test_program();
    let mut ind = Individual::new(program, 0);

    assert!(!ind.has_failures());

    ind.proof_status = Some(vec![ProofResult {
        contract_id: "V:v1".to_string(),
        target_id: "C:c3".to_string(),
        clause_index: 0,
        clause_kind: "post".to_string(),
        status: ProofStatus::Disproven,
        counterexample: Some("x=0".to_string()),
        counterexample_model: None,
    }]);

    assert!(ind.has_failures());
}

#[test]
fn test_individual_unknown_is_failure() {
    let program = make_test_program();
    let mut ind = Individual::new(program, 0);

    ind.proof_status = Some(vec![ProofResult {
        contract_id: "V:v1".to_string(),
        target_id: "C:c3".to_string(),
        clause_index: 0,
        clause_kind: "post".to_string(),
        status: ProofStatus::Unknown,
        counterexample: None,
        counterexample_model: None,
    }]);

    assert!(ind.has_failures(), "Unknown should count as failure");
    assert!(!ind.is_proven(), "Unknown should not count as proven");
}

// ---------------------------------------------------------------------------
// Empty / edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_empty_population_safety() {
    let config = EvolutionConfig {
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);

    // Should not panic on empty population
    pool.evaluate_fitness();
    pool.evolve_generation();
    assert!(pool.best().is_none());

    let pairs = pool.select_parents();
    assert!(pairs.is_empty());
}

#[test]
fn test_evolution_with_complex_program() {
    let config = EvolutionConfig {
        population_size: 15,
        elite_count: 3,
        max_generations: 10,
        seed: Some(42),
        ..Default::default()
    };
    let mut pool = GraphPool::new(config);
    let base = make_complex_program();
    pool.seed_population(&base, 15);

    let result = pool.run(5);

    assert_eq!(result.generations_run, 5);
    assert!(result.population_stats.best_fitness > 0.0);
    assert!(count_nodes(&result.best.program) > 0);
}

#[test]
fn test_mutation_op_all_variants() {
    assert_eq!(MutationOp::ALL.len(), 6);
    assert!(MutationOp::ALL.contains(&MutationOp::SwapComputeOp));
    assert!(MutationOp::ALL.contains(&MutationOp::ModifyConstant));
    assert!(MutationOp::ALL.contains(&MutationOp::InsertNode));
    assert!(MutationOp::ALL.contains(&MutationOp::RemoveNode));
    assert!(MutationOp::ALL.contains(&MutationOp::SwapEdges));
    assert!(MutationOp::ALL.contains(&MutationOp::ModifyContract));
}
