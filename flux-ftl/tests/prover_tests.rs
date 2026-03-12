use flux_ftl::parser::parse_ftl;
use flux_ftl::prover::{prove_contracts, ProofStatus, ProverConfig};

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

    // V:e9: invariant with symbolic state.length → DISPROVEN
    assert_eq!(results[8].2, ProofStatus::Disproven);

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

    // V:e3: pre C:s2_load.val >= 0 → DISPROVEN (symbolic atomic_load)
    assert_eq!(results[2], ("V:e3".into(), "pre".into(), ProofStatus::Disproven));
}
