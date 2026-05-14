---
name: FLUX-lang Projektübersicht
description: KI-native Programmiersprache FLUX — Konzept, GitHub, Status
type: project
---

- **GitHub:** `https://github.com/MarcoFPO/FLUX-lang` (public, Branch: `main`)
- **Konzept:** KI erzeugt typisierte Computation-Graphen (DAG) → MLIR → LLVM → nativer Maschinencode
- **Status:** Konzept/Forschung, kein Prototyp
- **Docs:** `docs/FLUX-v2-SPEC.md`, `docs/ANALYSIS.md`, Simulationen (Hello World, Snake Game)
- **Beispiele:** `examples/hello-world.flux.json`, `examples/snake-game.flux.json`

**Why:** FLUX ist eine Sprache speziell für LLMs als Code-Generatoren.
**How to apply:** Immer den DAG-First-Ansatz im Kopf behalten — keine imperative Syntax, sondern Graph-Knoten.
