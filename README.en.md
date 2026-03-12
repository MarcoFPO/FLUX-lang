<p align="center">
  <img src="assets/logo.gif" alt="FLUX Validator Logo" width="400">
</p>

<p align="center">
  <a href="README.md">DE</a> |
  <strong>EN</strong> |
  <a href="README.fr.md">FR</a> |
  <a href="README.es.md">ES</a> |
  <a href="README.ja.md">JA</a> |
  <a href="README.zh.md">ZH</a>
</p>

# FLUX — AI-Native Computation Substrate

**FLUX** is an execution architecture where AI systems (LLMs) generate computation graphs in FTL (FLUX Text Language), which are formally verified and compiled to optimal machine code.

**LLM generates FTL text. System compiles to binary. Formally verified. Optimal.**

## Design Axioms

```
1. Compile time is irrelevant       → Exhaustive verification, superoptimization
2. Human readability is irrelevant   → LLM works with FTL (structured text),
                                       system compiles to binary
3. Human compensations               → No debug, no exception handling,
   are not needed                      no defensive programming
4. Code generation performance       → Unlimited LLM iterations,
   is secondary                        unlimited depth of analysis
5. Creativity is desired             → AI should INVENT novel solutions,
                                       not just reproduce known patterns
6. Pragmatism in verification        → Tiered prover strategy with timeouts,
                                       undecidable → escalation, not infinite loops
```

## Architecture

```
Requirement (natural language, out of scope)
    │
LLM (the programmer — replaces the human)
    │  FTL (FLUX Text Language) — structured text
    ▼
FLUX System
    ├─ FTL Compiler (Text → Binary + BLAKE3 hashes)
    ├─ Validator (Structure + Types + Effects + Regions)
    │    FAIL → JSON feedback to LLM (with suggestions)
    ├─ Contract Prover (tiered: Z3 60s → BMC 5m → Lean)
    │    DISPROVEN → Counterexample to LLM
    │    UNDECIDABLE → Hint to LLM or incubation
    ├─ Pool / Evolution (for INVENT/DISCOVER)
    │    Fitness feedback to LLM (relative metrics)
    ├─ Superoptimizer (3-tier: LLVM + MLIR + STOKE)
    │    Hot paths optimal, rest LLVM -O3 quality
    └─ MLIR → LLVM → native machine code
    │
┌───┴────┬──────────┬──────────┐
ARM64   x86-64    RISC-V     WASM
```

## Node Types

| Node | Function |
|------|----------|
| **C-Node** | Pure computation (ADD, MUL, CONST, ...) |
| **E-Node** | Side effect with exactly 2 outputs (success + failure) |
| **K-Node** | Control flow: Seq, Par, Branch, Loop |
| **V-Node** | Contract (SMT-LIB2) — MUST be proven for compilation |
| **T-Node** | Type: Integer, Float, Struct, Array, Variant, Fn, Opaque |
| **M-Node** | Memory operation (region-bound) |
| **R-Node** | Memory lifetime (arena) |


## Core Principles

**LLM as Programmer:** The LLM replaces the human programmer. It delivers FTL text (no binary, no hashes). The system compiles FTL to binary graphs, computes BLAKE3 hashes, and returns JSON feedback.

**Total Correctness:** Every compiled binary is formally verified. Zero runtime checks. Contracts are proven through a tiered prover strategy (Z3 → BMC → Lean).

**Explorative Synthesis:** The AI generates not one graph, but hundreds. Correctness is the filter, creativity is the generator. The genetic algorithm (GA) is the primary innovation engine; the LLM provides initialization and targeted repairs.

**Superoptimization:** 3-tier (LLVM -O3 → MLIR-level → STOKE). Hot paths better than hand-written assembly. Realistic: 5-20% overall improvement over pure LLVM -O3.

**Content-Addressed:** No variable names. Identity = BLAKE3 hash of content (computed by the system). Same computation = same hash = automatic deduplication.

**Biological Mutation Model:** Faulty graphs are isolated in an incubation zone for further development. A mutation on a mutation can turn something "bad" into something "special". Only the final binary must be provably correct — the path there may lead through errors.

## Documentation

- **[FLUX v3 Specification](docs/FLUX-v3-SPEC.md)** — Current specification (18 sections)
- **[FLUX v2 Specification](docs/FLUX-v2-SPEC.md)** — Previous version (with human concessions)
- **[Expert Analysis](docs/ANALYSIS.md)** — Evaluation by 3 specialized agents (Round 2)
- **[Hello World Simulation](docs/SIMULATION-hello-world.md)** — Pipeline from requirement to machine code
- **[Snake Game Simulation](docs/SIMULATION-snake-game.md)** — Complex example with sound

## Examples

- [`examples/hello-world.flux.json`](examples/hello-world.flux.json) — Hello World (v2 JSON format)
- [`examples/snake-game.flux.json`](examples/snake-game.flux.json) — Snake Game (v2 JSON format)

*Note: v3 uses FTL (FLUX Text Language) instead of JSON. The examples show the v2 format.*

## Requirement Types

```
TRANSLATE    "Sort with mergesort"              → Direct synthesis (1 graph)
OPTIMIZE     "Sort as fast as possible"         → Pareto selection (many variants)
INVENT       "Improve sort(), invent something" → Explorative synthesis + evolution
DISCOVER     "Find computation with property X" → Open search in graph space
```


## License

MIT

## Acknowledgments
- Bea for the logo
- Gerd for the inspiration
- Michi for the comments
