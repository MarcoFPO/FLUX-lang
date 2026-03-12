<p align="center">
  <img src="assets/logo.gif" alt="FLUX Validator Logo" width="400">
</p>

# FLUX — AI-Native Computation Substrate

**FLUX** ist eine Ausfuehrungsarchitektur, bei der KI-Systeme (LLMs) Computation-Graphen in FTL (FLUX Text Language) erzeugen, die formal verifiziert und zu optimalem Maschinencode kompiliert werden.

**LLM erzeugt FTL-Text. System kompiliert zu Binaer. Formal verifiziert. Optimal.**

## Design-Axiome

```
1. Compile-Zeit ist irrelevant     → Exhaustive Verifikation, Superoptimierung
2. Menschliche Lesbarkeit irrelevant → LLM arbeitet mit FTL (strukturierter Text),
                                       System kompiliert zu Binaer
3. Menschliche Kompensationen      → Kein Debug, kein Exception-Handling,
   werden nicht benoetigt            keine defensive Programmierung
4. Performance der Codegenerierung → Beliebig viele LLM-Iterationen,
   ist sekundaer                     beliebig tiefe Analyse
5. Kreativitaet ist erwuenscht     → KI soll neuartige Loesungen ERFINDEN,
                                     nicht nur bekannte Muster reproduzieren
6. Pragmatismus bei Verifikation   → Gestaffelte Prover-Strategie mit Timeouts,
                                     Unentscheidbares → Eskalation, nicht Endlosschleife
```

## Architektur

```
Anforderung (natuerliche Sprache, out of scope)
    │
LLM (der Programmierer — ersetzt den Menschen)
    │  FTL (FLUX Text Language) — strukturierter Text
    ▼
FLUX-System
    ├─ FTL-Compiler (Text → Binaer + BLAKE3-Hashes)
    ├─ Validator (Struktur + Typen + Effekte + Regionen)
    │    FAIL → JSON-Feedback ans LLM (mit Suggestions)
    ├─ Contract Prover (gestaffelt: Z3 60s → BMC 5m → Lean)
    │    DISPROVEN → Counterexample ans LLM
    │    UNDECIDABLE → Hint ans LLM oder Inkubation
    ├─ Pool / Evolution (bei ERFINDE/ENTDECKE)
    │    Fitness-Feedback ans LLM (relative Metriken)
    ├─ Superoptimizer (3-stufig: LLVM + MLIR + STOKE)
    │    Hot Paths optimal, Rest LLVM -O3 Qualitaet
    └─ MLIR → LLVM → nativer Maschinencode
    │
┌───┴────┬──────────┬──────────┐
ARM64   x86-64    RISC-V     WASM
```

## Node-Typen

| Node | Funktion |
|------|----------|
| **C-Node** | Reine Berechnung (ADD, MUL, CONST, ...) |
| **E-Node** | Seiteneffekt mit exakt 2 Ausgaengen (success + failure) |
| **K-Node** | Kontrollfluss: Seq, Par, Branch, Loop |
| **V-Node** | Contract (SMT-LIB2) — MUSS bewiesen werden fuer Compilation |
| **T-Node** | Typ: Integer, Float, Struct, Array, Variant, Fn, Opaque |
| **M-Node** | Speicheroperation (Region-gebunden) |
| **R-Node** | Speicher-Lifetime (Arena) |


## Kernprinzipien

**LLM als Programmierer:** Das LLM ersetzt den menschlichen Programmierer. Es liefert FTL-Text (kein Binaer, keine Hashes). Das System kompiliert FTL zu binaeren Graphen, berechnet BLAKE3-Hashes und gibt JSON-Feedback zurueck.

**Totale Korrektheit:** Jedes kompilierte Binary ist formal verifiziert. Null Runtime-Checks. Contracts werden durch gestaffelte Prover-Strategie bewiesen (Z3 → BMC → Lean).

**Explorative Synthese:** KI erzeugt nicht einen Graph, sondern Hunderte. Korrektheit ist der Filter, Kreativitaet ist der Generator. Der genetische Algorithmus (GA) ist der primaere Innovationsmotor, das LLM liefert Initialisierung und gezielte Reparaturen.

**Superoptimierung:** 3-stufig (LLVM -O3 → MLIR-Level → STOKE). Hot Paths besser als handgeschriebener Assembler. Realistisch: 5-20% Gesamtverbesserung ueber reines LLVM -O3.

**Content-Addressiert:** Keine Variablennamen. Identitaet = BLAKE3-Hash des Inhalts (vom System berechnet). Gleiche Berechnung = gleicher Hash = automatische Deduplizierung.

**Biologisches Mutations-Modell:** Fehlerhafte Graphen werden in einer Inkubations-Zone isoliert weiterentwickelt. Eine Mutation auf eine Mutation kann etwas "Schlechtes" zu etwas "Besonderem" machen. Nur das fertige Binary muss bewiesen korrekt sein — der Weg dorthin darf durch Fehler fuehren.

## Dokumentation

- **[FLUX v3 Spezifikation](docs/FLUX-v3-SPEC.md)** — Aktuelle Spezifikation (18 Sektionen)
- **[FLUX v2 Spezifikation](docs/FLUX-v2-SPEC.md)** — Vorherige Version (mit menschlichen Konzessionen)
- **[Expertenanalyse](docs/ANALYSIS.md)** — Bewertung durch 3 spezialisierte Agenten (Runde 2)
- **[Hello World Simulation](docs/SIMULATION-hello-world.md)** — Pipeline von Anforderung bis Maschinencode
- **[Snake Game Simulation](docs/SIMULATION-snake-game.md)** — Komplexes Beispiel mit Sound

## Beispiele

- [`examples/hello-world.flux.json`](examples/hello-world.flux.json) — Hello World (v2 JSON-Format)
- [`examples/snake-game.flux.json`](examples/snake-game.flux.json) — Snake Game (v2 JSON-Format)

*Hinweis: v3 verwendet FTL (FLUX Text Language) statt JSON. Die Beispiele zeigen das v2-Format.*

## Anforderungstypen

```
UEBERSETZE   "Sortiere mit Mergesort"         → Direkte Synthese (1 Graph)
OPTIMIERE    "Sortiere moeglichst schnell"    → Pareto-Selektion (viele Varianten)
ERFINDE      "Verbessere sort(), erfinde Neues"→ Explorative Synthese + Evolution
ENTDECKE     "Finde Berechnung mit Eigenschaft X" → Offene Suche im Graphen-Raum
```


## Lizenz

MIT
