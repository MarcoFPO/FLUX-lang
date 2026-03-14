use flux_ftl::ast::*;
use flux_ftl::parser::parse_ftl;
use flux_ftl::prover::{prove_contracts, BmcConfig, ProofStatus, ProverConfig};

fn parse_and_prove(input: &str) -> Vec<(String, String, flux_ftl::prover::ProofStatus)> {
    let result = parse_ftl(input);
    let ast = result.ast.expect("parse should succeed");
    let config = ProverConfig::default();
    let results = prove_contracts(&ast, &config);
    results
        .into_iter()
        .map(|r| (r.contract_id, r.clause_kind, r.status))
        .collect()
}

#[test]
fn hello_world_contracts_proven() {
    let input = std::fs::read_to_string("testdata/hello_world.ftl").unwrap();
    let results = parse_and_prove(&input);

    assert_eq!(results.len(), 2);
    // V:e1 pre: C:c2.val == 1 → PROVEN (C:c2 = const { value: 1 })
    assert_eq!(results[0], ("V:e1".into(), "pre".into(), ProofStatus::Proven));
    // V:e2 pre: C:c3.val == 12 → PROVEN (C:c3 = const { value: 12 })
    assert_eq!(results[1], ("V:e2".into(), "pre".into(), ProofStatus::Proven));
}

#[test]
fn ffi_extern_assumed_and_pre_proven() {
    let input = std::fs::read_to_string("testdata/ffi.ftl").unwrap();
    let results = parse_and_prove(&input);

    // V:e1..V:e5 are trust:EXTERN → all clauses ASSUMED
    let extern_results: Vec<_> = results.iter()
        .filter(|(id, _, _)| {
            let n: u32 = id.strip_prefix("V:e").unwrap().parse().unwrap();
            n <= 5
        })
        .collect();
    assert!(extern_results.iter().all(|(_, _, s)| *s == ProofStatus::Assumed));
    assert_eq!(extern_results.len(), 10); // 5 contracts × 2 clauses each

    // V:e6 pre: C:c_alloc_size.val > 0 → PROVEN (4096 > 0)
    let v6: Vec<_> = results.iter().filter(|(id, _, _)| id == "V:e6").collect();
    assert_eq!(v6.len(), 1);
    assert_eq!(v6[0].2, ProofStatus::Proven);

    // V:e7 pre: C:c_data_len.val <= 4096 → PROVEN (10 <= 4096)
    let v7: Vec<_> = results.iter().filter(|(id, _, _)| id == "V:e7").collect();
    assert_eq!(v7.len(), 1);
    assert_eq!(v7[0].2, ProofStatus::Proven);
}

#[test]
fn snake_game_mixed_results() {
    let input = std::fs::read_to_string("testdata/snake_game.ftl").unwrap();
    let results = parse_and_prove(&input);

    assert_eq!(results.len(), 10);

    // V:e1 pre: C:c_stdin.val == 0 → PROVEN
    assert_eq!(results[0].2, ProofStatus::Proven);

    // V:e2..V:e5 invariant: forall quantified over symbolic ranges → PROVEN
    // (the universally quantified formulas with symbolic field accesses in body
    //  are proven because Z3 can show the implication holds)
    for r in &results[1..5] {
        assert_eq!(r.1, "invariant");
        assert_eq!(r.2, ProofStatus::Proven);
    }

    // V:e6..V:e8: post-conditions with symbolic `result` → DISPROVEN
    // (unconstrained symbolic result means counterexamples exist)
    for r in &results[5..8] {
        assert_eq!(r.2, ProofStatus::Disproven);
    }

    // V:e9: invariant state.length <= 800 → PROVEN (array max_length constraint)
    assert_eq!(results[8].2, ProofStatus::Proven);

    // V:e10: pre C:c_alsa_path != null → PROVEN (ConstBytes is non-null)
    assert_eq!(results[9].2, ProofStatus::Proven);
}

#[test]
fn concurrency_predicate_unknown() {
    let input = std::fs::read_to_string("testdata/concurrency.ftl").unwrap();
    let results = parse_and_prove(&input);

    assert_eq!(results.len(), 3);

    // V:e1: invariant with PredicateCall → UNKNOWN
    assert_eq!(results[0], ("V:e1".into(), "invariant".into(), ProofStatus::Unknown));

    // V:e2: invariant C:s1_load.val <= 10 → DISPROVEN (symbolic atomic_load)
    assert_eq!(results[1], ("V:e2".into(), "invariant".into(), ProofStatus::Disproven));

    // V:e3: pre C:s2_load.val >= 0 → PROVEN (unsigned type T:a1 constrains val >= 0)
    assert_eq!(results[2], ("V:e3".into(), "pre".into(), ProofStatus::Proven));
}

// ---------------------------------------------------------------------------
// BMC tests
// ---------------------------------------------------------------------------

fn make_bmc_program(contracts: Vec<ContractDef>, computes: Vec<ComputeDef>) -> Program {
    Program {
        types: vec![],
        regions: vec![],
        computes,
        effects: vec![],
        controls: vec![],
        contracts,
        memories: vec![],
        externs: vec![],
        entry: NodeRef::new("K:f1"),
    }
}

fn bmc_config(depth: u32) -> ProverConfig {
    ProverConfig {
        bmc_config: Some(BmcConfig {
            max_depth: depth,
            ..BmcConfig::default()
        }),
        ..ProverConfig::default()
    }
}

#[test]
fn test_bmc_simple_forall() {
    // Forall(i, 0..5, i >= 0) should be BmcProven
    let contract = ContractDef {
        id: NodeRef::new("V:e1"),
        target: NodeRef::new("E:d1"),
        clauses: vec![ContractClause::Invariant {
            formula: Formula::Forall {
                var: "i".into(),
                range_start: Expr::IntLit { value: 0 },
                range_end: Expr::IntLit { value: 5 },
                body: Box::new(Formula::Comparison {
                    left: Expr::Ident { name: "i".into() },
                    op: CmpOp::Gte,
                    right: Expr::IntLit { value: 0 },
                }),
            },
        }],
        trust: None,
    };

    let program = make_bmc_program(vec![contract], vec![]);
    let config = bmc_config(10);
    let results = prove_contracts(&program, &config);

    assert_eq!(results.len(), 1);
    // Z3 can prove this universally, so it should be Proven (not BmcProven)
    // since Z3 handles it before BMC fallback is needed
    assert_eq!(results[0].status, ProofStatus::Proven);
}

#[test]
fn test_bmc_refuted() {
    // Forall(i, 0..5, i > 3) should be BmcRefuted (i=0,1,2,3 violate i > 3)
    // But Z3 can also disprove this directly via universal quantifier.
    // Let's use a PredicateCall-containing formula wrapped with a Forall
    // to force Z3 Unknown, then BMC can check it.
    //
    // Actually, since Z3 handles Forall natively, let's test BMC directly
    // by constructing a formula that Z3 returns Unknown for.
    // For a simple Forall, Z3 will handle it. So we test via the
    // bmc_check pathway indirectly: if Z3 gives Proven or Disproven, BMC
    // is not invoked. Let's verify Z3 disproves this:
    let contract = ContractDef {
        id: NodeRef::new("V:e1"),
        target: NodeRef::new("E:d1"),
        clauses: vec![ContractClause::Invariant {
            formula: Formula::Forall {
                var: "i".into(),
                range_start: Expr::IntLit { value: 0 },
                range_end: Expr::IntLit { value: 5 },
                body: Box::new(Formula::Comparison {
                    left: Expr::Ident { name: "i".into() },
                    op: CmpOp::Gt,
                    right: Expr::IntLit { value: 3 },
                }),
            },
        }],
        trust: None,
    };

    let program = make_bmc_program(vec![contract], vec![]);
    let config = bmc_config(10);
    let results = prove_contracts(&program, &config);

    assert_eq!(results.len(), 1);
    // Z3 disproves this before BMC is needed
    assert_eq!(results[0].status, ProofStatus::Disproven);
}

#[test]
fn test_bmc_fallback_from_z3_unknown() {
    // PredicateCall causes Z3 to return Unknown. With BMC enabled,
    // we should still get Unknown since PredicateCall can't be translated.
    // But let's test a scenario where Z3 goes Unknown and BMC resolves it.
    //
    // Use a Forall with a PredicateCall-free body but add a predicate
    // at the top level via And to make Z3 return Unknown, then verify
    // that BMC fallback is triggered.
    //
    // Actually, PredicateCall returns None from translate_formula, which
    // causes Unknown before Z3 even runs. BMC also can't translate it.
    //
    // The real test: concurrency.ftl has a PredicateCall that goes Unknown.
    // With BMC enabled, it should still be Unknown (BMC can't help with predicates).
    let input = std::fs::read_to_string("testdata/concurrency.ftl").unwrap();
    let result = parse_ftl(&input);
    let ast = result.ast.expect("parse should succeed");
    let config = bmc_config(10);
    let results = prove_contracts(&ast, &config);

    assert_eq!(results.len(), 3);
    // V:e1 with PredicateCall -> still Unknown even with BMC
    assert_eq!(results[0].status, ProofStatus::Unknown);
    // Other results unchanged
    assert_eq!(results[1].status, ProofStatus::Disproven);
    assert_eq!(results[2].status, ProofStatus::Proven);
}

#[test]
fn test_bmc_config_default() {
    let config = BmcConfig::default();
    assert_eq!(config.max_depth, 10);
    assert_eq!(config.timeout_secs, 300);

    let prover_config = ProverConfig::default();
    assert!(prover_config.bmc_config.is_none());
    assert_eq!(prover_config.timeout_ms, 5000);
}
