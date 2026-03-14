// Phase 19: Performance Benchmarks for the FLUX FTL Pipeline
//
// Measures each pipeline stage independently as well as the full pipeline:
//   Parse → Validate → Prove → Compile → Optimize

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::fs;
use std::time::Duration;

use flux_ftl::compiler::compile;
use flux_ftl::optimizer::{optimize_graph, OptimizationConfig};
use flux_ftl::parser::parse_ftl;
use flux_ftl::prover::{prove_contracts, ProverConfig};
use flux_ftl::validator::validate;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load an FTL source file from the testdata directory.
fn load_testdata(name: &str) -> String {
    fs::read_to_string(format!("testdata/{}.ftl", name))
        .unwrap_or_else(|e| panic!("failed to load testdata/{}.ftl: {}", name, e))
}

/// The testdata programs used across benchmarks.
const TEST_PROGRAMS: &[&str] = &["hello_world", "ffi", "snake_game", "concurrency"];

// ---------------------------------------------------------------------------
// Stage 1: Parser
// ---------------------------------------------------------------------------

fn bench_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser");

    for &name in TEST_PROGRAMS {
        let input = load_testdata(name);
        group.bench_with_input(BenchmarkId::new("parse_ftl", name), &input, |b, input| {
            b.iter(|| parse_ftl(black_box(input)));
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Stage 2: Validator
// ---------------------------------------------------------------------------

fn bench_validator(c: &mut Criterion) {
    let mut group = c.benchmark_group("validator");

    for &name in TEST_PROGRAMS {
        let input = load_testdata(name);
        let result = parse_ftl(&input);
        if let Some(ref ast) = result.ast {
            let program = ast.clone();
            group.bench_with_input(
                BenchmarkId::new("validate", name),
                &program,
                |b, program| {
                    b.iter(|| validate(black_box(program)));
                },
            );
        }
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Stage 3: Prover (Z3-based — reduced sample size)
// ---------------------------------------------------------------------------

fn bench_prover(c: &mut Criterion) {
    let mut group = c.benchmark_group("prover");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));

    let config = ProverConfig::default();

    for &name in TEST_PROGRAMS {
        let input = load_testdata(name);
        let result = parse_ftl(&input);
        if let Some(ref ast) = result.ast {
            let val = validate(ast);
            if val.valid {
                let program = ast.clone();
                group.bench_with_input(
                    BenchmarkId::new("prove_contracts", name),
                    &program,
                    |b, program| {
                        b.iter(|| prove_contracts(black_box(program), black_box(&config)));
                    },
                );
            }
        }
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Stage 4: Compiler
// ---------------------------------------------------------------------------

fn bench_compiler(c: &mut Criterion) {
    let mut group = c.benchmark_group("compiler");

    for &name in TEST_PROGRAMS {
        let input = load_testdata(name);
        let result = parse_ftl(&input);
        if let Some(ref ast) = result.ast {
            let program = ast.clone();
            group.bench_with_input(
                BenchmarkId::new("compile", name),
                &program,
                |b, program| {
                    b.iter(|| compile(black_box(program)));
                },
            );
        }
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Stage 5: Optimizer
// ---------------------------------------------------------------------------

fn bench_optimizer(c: &mut Criterion) {
    let mut group = c.benchmark_group("optimizer");

    let config = OptimizationConfig::default();

    for &name in TEST_PROGRAMS {
        let input = load_testdata(name);
        let result = parse_ftl(&input);
        if let Some(ref ast) = result.ast {
            let program = ast.clone();
            group.bench_with_input(
                BenchmarkId::new("optimize_graph", name),
                &program,
                |b, program| {
                    b.iter(|| optimize_graph(black_box(program), black_box(&config)));
                },
            );
        }
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Full Pipeline: Parse → Validate → Prove → Compile → Optimize
// ---------------------------------------------------------------------------

fn bench_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_pipeline");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(15));

    let prover_config = ProverConfig::default();
    let opt_config = OptimizationConfig::default();

    for &name in TEST_PROGRAMS {
        let input = load_testdata(name);
        group.bench_with_input(
            BenchmarkId::new("pipeline", name),
            &input,
            |b, input| {
                b.iter(|| {
                    // Stage 1: Parse
                    let result = parse_ftl(black_box(input));
                    let ast = match result.ast {
                        Some(ref ast) => ast,
                        None => return,
                    };

                    // Stage 2: Validate
                    let val = validate(ast);
                    if !val.valid {
                        return;
                    }

                    // Stage 3: Prove
                    let _proofs = prove_contracts(ast, &prover_config);

                    // Stage 4: Compile
                    let _compiled = compile(ast);

                    // Stage 5: Optimize
                    let _optimized = optimize_graph(ast, &opt_config);
                });
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion harness
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_parser,
    bench_validator,
    bench_prover,
    bench_compiler,
    bench_optimizer,
    bench_full_pipeline,
);
criterion_main!(benches);
