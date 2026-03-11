# FLUX — AI-Native Computation Substrate

**FLUX** ist ein Konzept fuer eine neuartige Ausfuehrungsarchitektur, die **nicht fuer Menschen, sondern fuer KI-Systeme** optimiert ist. Statt Quelltext zu erzeugen, generiert eine KI direkt einen typisierten Computation-Graphen (DAG), der ueber MLIR/LLVM zu nativem Maschinencode fuer jede Zielplattform kompiliert wird.

## Kernidee

```
Anforderung (natuerliche Sprache)
        |
   KI erzeugt strukturierten Graph (JSON → FLUX DAG)
        |
   Validator + SMT-Prover (formale Verifikation)
        |
   FLUX → MLIR → LLVM IR
        |
   ┌────┴────┬──────────┬──────────┐
   ARM64    x86-64    RISC-V     WASM
   nativer Maschinencode pro Plattform
```

**Kein Quelltext. Kein Parser. Keine Syntaxfehler.**
Ein Graph — viele Plattformen — beweisbar korrekt.

## Was FLUX anders macht

| Aspekt | Klassisch (C, Rust, Python) | FLUX |
|--------|---------------------------|------|
| Primaere Darstellung | Text (Quellcode) | Typisierter DAG |
| Erzeugt von | Mensch (+ KI als Assistent) | KI direkt |
| Seiteneffekte | Implizit, versteckt | Explizit im Graph (C-Node vs. E-Node) |
| Fehlerbehandlung | Exceptions / panic | F-Nodes als Datenflusspfade |
| Speicherverwaltung | GC / malloc / Ownership | Region-basiert (Arena, deterministisch) |
| Formale Verifikation | Optional, externes Tool | Eingebaut (V-Nodes + SMT-Solver) |
| Hardware-Ziel | Compiler-spezifisch | Ein Graph → alle Plattformen via LLVM |
| Tests | Separates Artefakt | Automatisch aus Contracts abgeleitet |

## Architektur

```
┌─────────────────────────────────────────────────────┐
│  KI-GENERATOR (LLM + Structured Output)             │
│  Iterative Korrekturschleife mit Validator-Feedback  │
└───────────────────┬─────────────────────────────────┘
                    │  FLUX Computation Graph
┌───────────────────▼─────────────────────────────────┐
│  VALIDATOR + PROVER                                  │
│  Struktur │ Typen │ Effekte │ Regionen │ SMT-Solver │
└───────────────────┬─────────────────────────────────┘
                    │  Verifizierter Graph
┌───────────────────▼─────────────────────────────────┐
│  FLUX MLIR-Dialekt → LLVM IR                        │
└───────┬───────────┬───────────┬─────────────────────┘
   ┌────▼────┐ ┌────▼────┐ ┌───▼──────┐
   │ AArch64 │ │ x86-64  │ │ RISC-V64 │  ...
   │ NEON    │ │ AVX-512 │ │ RVV      │
   └─────────┘ └─────────┘ └──────────┘
```

## Node-Typen

| Node | Funktion | Beispiel |
|------|----------|---------|
| **C-Node** | Reine Berechnung | `ADD`, `MUL`, `SIN` |
| **E-Node** | Seiteneffekt (IO) | `SYSCALL_WRITE`, `DB.query` |
| **K-Node** | Komposition | `SEQ`, `PAR`, `BRANCH`, `LOOP`, `FINALLY` |
| **F-Node** | Fehlerbehandlung | Recovery-Pfad bei Syscall-Fehler |
| **V-Node** | Contract (Pre/Post) | `score >= 0`, `array.length <= max` |
| **M-Node** | Speicheroperation | `ALLOC`, `LOAD`, `STORE` |
| **R-Node** | Speicherregion | Arena mit Lifetime-Scope |
| **T-Node** | Typ-Definition | `Int32`, `Array<Float64>` mit Constraints |
| **H-Node** | Hardware-Hint | `PREFER_VECTOR`, `CACHE_LINE_ALIGN` |
| **P-Node** | Modul (Package) | Typisiertes Interface, Content-addressiert |
| **D-Node** | Debug-Mapping | Node-ID → Anforderung + KI-Session |

## Speichermodell: Region-basiert

Kein Garbage Collector. Kein manuelles `free`. Deterministische Arena-Allokation:

```
R-Node #game (Lifetime: Spielschleife)
  └── R-Node #frame (Lifetime: 1 Tick, ~150ms)
        └── Framebuffer, temporaere Daten
            → am Ende des Ticks automatisch freigegeben (bulk free)
```

## Fehlerbehandlung: Kein Exception-Overhead

Fehler sind normale Datenflusspfade im Graph — kein Stack-Unwinding, kein `try/catch`:

```
E-Node write() ──ok──→ weiter
       │
       └──fail──→ F-Node ──recovery──→ Alternative
```

Kompiliert zu einem einfachen Branch. Null Overhead im Erfolgsfall.

## Dokumentation

- **[FLUX v2 Spezifikation](docs/FLUX-v2-SPEC.md)** — Vollstaendiges technisches Konzept
- **[Expertenanalyse](docs/ANALYSIS.md)** — Bewertung durch 3 spezialisierte Agenten
- **[Hello World Simulation](docs/SIMULATION-hello-world.md)** — Komplette Pipeline von Anforderung bis Maschinencode
- **[Snake Game Simulation](docs/SIMULATION-snake-game.md)** — Komplexes Beispiel mit Sound, Input, Game Loop

## Beispiele

- [`examples/hello-world.flux.json`](examples/hello-world.flux.json) — Minimales "Hello World"
- [`examples/snake-game.flux.json`](examples/snake-game.flux.json) — Snake mit Terminal-Rendering und ALSA-Sound

## Status

**Phase: Konzept / Forschung**

Dies ist ein Forschungsprojekt. Die naechsten Schritte:

1. FLUX Binary Format formal spezifizieren
2. Validator-Prototyp (Rust)
3. FLUX als MLIR-Dialekt implementieren
4. KI-Generierungspipeline mit Structured Output
5. Erstes Target: einfache Arithmetik → x86-64 Binary

## Technische Grundlagen

FLUX baut auf existierenden Forschungsergebnissen auf:

| Konzept | Herkunft |
|---------|----------|
| Graph-basierte IR | Sea of Nodes (1995), RVSDG |
| Multi-Level IR | MLIR (Google/LLVM) |
| Content-Addressierung | Unison, Nix, Git |
| Effect-Tracking | Koka (algebraische Effekte), RVSDG |
| Formale Verifikation | CompCert, CakeML |
| Region-basierter Speicher | MLKit, Rust Lifetimes |

**Genuint neu**: KI-native Graph-Generierung + Kombination aller Features in einem kohaerenten System.

## Lizenz

MIT
