use std::path::PathBuf;

use flux_ftl::compiler::{self, CompileMetadata, NodeKind};
use flux_ftl::error::Status;
use flux_ftl::parser::parse_ftl;

// ===========================================================================
// Helper
// ===========================================================================

fn parse_ok(input: &str) -> flux_ftl::ast::Program {
    let result = parse_ftl(input);
    assert!(
        matches!(result.status, Status::Ok),
        "expected Status::Ok, got errors: {:?}",
        result.errors,
    );
    result.ast.expect("ast should be Some on Ok status")
}

fn compile_ok(input: &str) -> compiler::CompiledGraph {
    let ast = parse_ok(input);
    compiler::compile(&ast).expect("compilation should succeed")
}

// ===========================================================================
// 1. hello_world compiles and produces a deterministic hash
// ===========================================================================

#[test]
fn hello_world_compiles() {
    let input = include_str!("../testdata/hello_world.ftl");
    let graph = compile_ok(input);

    // hello_world has: 3 types, 1 region, 5 computes, 3 effects, 3 controls, 2 contracts = 17
    assert!(graph.metadata.total_nodes > 0, "should have nodes");
    assert!(graph.metadata.unique_nodes > 0, "should have unique nodes");
    assert!(
        graph.metadata.unique_nodes <= graph.metadata.total_nodes,
        "unique <= total",
    );
    assert_eq!(graph.metadata.entry_id, "K:f1");

    // Entry hash should be non-zero.
    assert_ne!(graph.entry_hash, [0u8; 32], "entry hash should be non-zero");
}

// ===========================================================================
// 2. ffi compiles with more nodes, dedup works
// ===========================================================================

#[test]
fn ffi_compiles() {
    let input = include_str!("../testdata/ffi.ftl");
    let graph = compile_ok(input);

    // The FFI program has many nodes.
    assert!(graph.metadata.total_nodes > 10, "FFI should have many nodes");
    assert!(graph.metadata.unique_nodes > 0, "should have unique nodes");

    // Check that extern nodes exist.
    let extern_nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Extern)
        .collect();
    assert!(!extern_nodes.is_empty(), "should have extern nodes");

    assert_eq!(graph.metadata.entry_id, "K:f_main");
}

// ===========================================================================
// 3. Determinism — same input produces same hashes
// ===========================================================================

#[test]
fn determinism_hello_world() {
    let input = include_str!("../testdata/hello_world.ftl");

    let graph1 = compile_ok(input);
    let graph2 = compile_ok(input);

    assert_eq!(
        graph1.entry_hash, graph2.entry_hash,
        "same input must produce same entry hash",
    );
    assert_eq!(
        graph1.nodes.len(),
        graph2.nodes.len(),
        "same input must produce same node count",
    );

    // All node hashes should match.
    for (n1, n2) in graph1.nodes.iter().zip(graph2.nodes.iter()) {
        assert_eq!(n1.hash, n2.hash, "node hashes must be identical for '{}'", n1.original_id);
        assert_eq!(n1.original_id, n2.original_id);
    }
}

#[test]
fn determinism_ffi() {
    let input = include_str!("../testdata/ffi.ftl");

    let graph1 = compile_ok(input);
    let graph2 = compile_ok(input);

    assert_eq!(graph1.entry_hash, graph2.entry_hash);
    assert_eq!(graph1.nodes.len(), graph2.nodes.len());
}

// ===========================================================================
// 4. Binary write/read roundtrip
// ===========================================================================

#[test]
fn binary_roundtrip_hello_world() {
    let input = include_str!("../testdata/hello_world.ftl");
    let graph = compile_ok(input);

    let tmp_path = PathBuf::from("/tmp/flux_test_hello_world.flux.bin");
    compiler::write_binary(&graph, &tmp_path).expect("write should succeed");

    let loaded = compiler::read_binary(&tmp_path).expect("read should succeed");

    assert_eq!(graph.entry_hash, loaded.entry_hash, "entry hash roundtrip");
    assert_eq!(graph.nodes.len(), loaded.nodes.len(), "node count roundtrip");

    for (orig, loaded_node) in graph.nodes.iter().zip(loaded.nodes.iter()) {
        assert_eq!(orig.hash, loaded_node.hash, "hash roundtrip for '{}'", orig.original_id);
        assert_eq!(orig.kind, loaded_node.kind, "kind roundtrip for '{}'", orig.original_id);
        assert_eq!(orig.original_id, loaded_node.original_id, "id roundtrip");
        assert_eq!(orig.data, loaded_node.data, "data roundtrip for '{}'", orig.original_id);
        assert_eq!(orig.refs, loaded_node.refs, "refs roundtrip for '{}'", orig.original_id);
    }

    // Cleanup.
    let _ = std::fs::remove_file(&tmp_path);
}

#[test]
fn binary_roundtrip_ffi() {
    let input = include_str!("../testdata/ffi.ftl");
    let graph = compile_ok(input);

    let tmp_path = PathBuf::from("/tmp/flux_test_ffi.flux.bin");
    compiler::write_binary(&graph, &tmp_path).expect("write should succeed");

    let loaded = compiler::read_binary(&tmp_path).expect("read should succeed");

    assert_eq!(graph.entry_hash, loaded.entry_hash);
    assert_eq!(graph.nodes.len(), loaded.nodes.len());

    for (orig, loaded_node) in graph.nodes.iter().zip(loaded.nodes.iter()) {
        assert_eq!(orig.hash, loaded_node.hash);
        assert_eq!(orig.data, loaded_node.data);
    }

    let _ = std::fs::remove_file(&tmp_path);
}

// ===========================================================================
// 5. Invalid binary format handling
// ===========================================================================

#[test]
fn read_invalid_magic() {
    let tmp_path = PathBuf::from("/tmp/flux_test_bad_magic.flux.bin");
    std::fs::write(&tmp_path, b"NOTFLUX").expect("write test file");

    let result = compiler::read_binary(&tmp_path);
    assert!(result.is_err(), "should fail on bad magic");

    let _ = std::fs::remove_file(&tmp_path);
}

#[test]
fn read_truncated_file() {
    let tmp_path = PathBuf::from("/tmp/flux_test_truncated.flux.bin");
    // Just the magic, missing everything else.
    std::fs::write(&tmp_path, b"FLUX").expect("write test file");

    let result = compiler::read_binary(&tmp_path);
    assert!(result.is_err(), "should fail on truncated file");

    let _ = std::fs::remove_file(&tmp_path);
}

// ===========================================================================
// 6. CompileMetadata conversion
// ===========================================================================

#[test]
fn compile_metadata_from_graph() {
    let input = include_str!("../testdata/hello_world.ftl");
    let graph = compile_ok(input);

    let meta = CompileMetadata::from(&graph);
    assert_eq!(meta.total_nodes, graph.metadata.total_nodes);
    assert_eq!(meta.unique_nodes, graph.metadata.unique_nodes);
    assert_eq!(meta.entry_hash.len(), 64, "hex-encoded hash should be 64 chars");
}

// ===========================================================================
// 7. All testdata files compile successfully
// ===========================================================================

#[test]
fn minimal_compiles() {
    let input = include_str!("../testdata/minimal.ftl");
    let graph = compile_ok(input);
    assert!(graph.metadata.unique_nodes > 0);
}

#[test]
fn concurrency_compiles() {
    let input = include_str!("../testdata/concurrency.ftl");
    let graph = compile_ok(input);
    assert!(graph.metadata.unique_nodes > 0);
}

#[test]
fn snake_game_compiles() {
    let input = include_str!("../testdata/snake_game.ftl");
    let graph = compile_ok(input);
    assert!(graph.metadata.unique_nodes > 0);
}

// ===========================================================================
// 8. Node kinds are correctly assigned
// ===========================================================================

#[test]
fn node_kinds_hello_world() {
    let input = include_str!("../testdata/hello_world.ftl");
    let graph = compile_ok(input);

    let type_count = graph.nodes.iter().filter(|n| n.kind == NodeKind::Type).count();
    let region_count = graph.nodes.iter().filter(|n| n.kind == NodeKind::Region).count();
    let compute_count = graph.nodes.iter().filter(|n| n.kind == NodeKind::Compute).count();
    let effect_count = graph.nodes.iter().filter(|n| n.kind == NodeKind::Effect).count();
    let control_count = graph.nodes.iter().filter(|n| n.kind == NodeKind::Control).count();
    let contract_count = graph.nodes.iter().filter(|n| n.kind == NodeKind::Contract).count();

    assert_eq!(type_count, 3, "3 type nodes");
    assert_eq!(region_count, 1, "1 region node");
    // C:c2 (const 1) and C:c5 (const 1) are identical content and get
    // deduplicated, so we expect 4 unique compute nodes instead of 5.
    assert_eq!(compute_count, 4, "4 unique compute nodes (one dedup pair)");
    assert_eq!(effect_count, 3, "3 effect nodes");
    // Controls may be deduplicated if two have identical content+refs.
    assert!(control_count >= 2, "at least 2 control nodes");
    assert_eq!(contract_count, 2, "2 contract nodes");
}

// ===========================================================================
// 9. Content addressing: different content = different hash
// ===========================================================================

#[test]
fn different_programs_different_hashes() {
    let hello = include_str!("../testdata/hello_world.ftl");
    let ffi = include_str!("../testdata/ffi.ftl");

    let graph1 = compile_ok(hello);
    let graph2 = compile_ok(ffi);

    assert_ne!(
        graph1.entry_hash, graph2.entry_hash,
        "different programs should have different entry hashes",
    );
}
