# FLUX — AI-Native Computation Substrate

**FLUX** ist eine Ausfuehrungsarchitektur, bei der KI-Systeme direkt binaere Computation-Graphen erzeugen, die formal verifiziert und zu optimalem Maschinencode kompiliert werden.

**Kein Text. Keine Namen. Keine menschlichen Hilfskonstrukte. Totale Korrektheit.**

## Design-Axiome

```
1. Compile-Zeit ist irrelevant     → Exhaustive Verifikation, Superoptimierung
2. Lesbarkeit ist irrelevant       → Kein Text, keine Namen, keine Kommentare
3. Menschliche Kompensationen      → Kein Debug, kein Exception-Handling,
   werden nicht benoetigt            keine defensive Programmierung
4. Performance der Codegenerierung → Beliebig viele LLM-Iterationen,
   ist sekundaer                     beliebig tiefe Analyse
5. Kreativitaet ist erwuenscht     → KI soll neuartige Loesungen ERFINDEN,
                                     nicht nur bekannte Muster reproduzieren
```

## Architektur

```
Anforderung (natuerliche Sprache)
    │
KI-Generator (LLM, unbegrenzte Iterationen)
    │
    ▼  Binaerer FLUX-Graph (kein JSON, kein Text)
    │
Validator (Struktur + Typen + Effekte + Regionen)
    │  FAIL → zurueck zum LLM (keine Begrenzung)
    ▼
Contract Prover (Z3/CVC5/Lean, KEIN Timeout)
    │  ALLE Contracts muessen PROVEN sein
    │  Unbewiesener Contract = Graph UNGUELTIG
    ▼
Superoptimizer (exhaustive Instruktionssuche)
    │  Findet optimale Sequenz pro Target
    ▼
MLIR → LLVM → nativer Maschinencode
    │
┌───┴────┬──────────┬──────────┐
ARM64   x86-64    RISC-V     WASM
```

## Node-Typen (7, reduziert von 11)

| Node | Funktion |
|------|----------|
| **C-Node** | Reine Berechnung (ADD, MUL, CONST, ...) |
| **E-Node** | Seiteneffekt mit exakt 2 Ausgaengen (success + failure) |
| **K-Node** | Kontrollfluss: Seq, Par, Branch, Loop |
| **V-Node** | Contract — MUSS bewiesen werden, sonst Graph ungueltig |
| **T-Node** | Typ mit Constraints |
| **M-Node** | Speicheroperation (Region-gebunden) |
| **R-Node** | Speicher-Lifetime (Arena) |

Entfernt: D-Node (Debug), H-Node (Hints), P-Node (Module), F-Node (Fehlerbehandlung).

## Kernprinzipien

**Totale Korrektheit:** Jedes Binary ist formal verifiziert. Null Runtime-Checks. Kein Overhead.

**Explorative Synthese:** KI erzeugt nicht einen Graph, sondern Hunderte. Korrektheit ist der Filter, Kreativitaet ist der Generator. Genetische Evolution auf Graph-Ebene findet Loesungen die kein Mensch erfinden wuerde.

**Superoptimierung:** Compile-Zeit irrelevant → exhaustive Suche nach der kuerzesten / schnellsten Instruktionssequenz pro Plattform. Besser als handgeschriebener Assembler.

**Content-Addressiert:** Keine Variablennamen. Identitaet = BLAKE3-Hash des Inhalts. Gleiche Berechnung = gleicher Hash = automatische Deduplizierung.

**Wachsende Wissensbasis:** Jedes akzeptierte Binary erweitert ein Graph Repository. Neuartigkeits-Metriken verhindern Stagnation. Das System wird ueber Zeit kreativer.

## Dokumentation

- **[FLUX v3 Spezifikation](docs/FLUX-v3-SPEC.md)** — Aktuelle Spezifikation (radikal reduziert)
- **[FLUX v2 Spezifikation](docs/FLUX-v2-SPEC.md)** — Vorherige Version (mit menschlichen Konzessionen)
- **[Expertenanalyse](docs/ANALYSIS.md)** — Bewertung durch 3 spezialisierte Agenten
- **[Hello World Simulation](docs/SIMULATION-hello-world.md)** — Pipeline von Anforderung bis Maschinencode
- **[Snake Game Simulation](docs/SIMULATION-snake-game.md)** — Komplexes Beispiel mit Sound

## Beispiele

- [`examples/hello-world.flux.json`](examples/hello-world.flux.json) — Hello World (v2 JSON-Format)
- [`examples/snake-game.flux.json`](examples/snake-game.flux.json) — Snake Game (v2 JSON-Format)

*Hinweis: v3 verwendet kein JSON mehr. Die Beispiele zeigen das v2-Format zur Veranschaulichung.*

## Anforderungstypen

```
UEBERSETZE   "Sortiere mit Mergesort"         → Direkte Synthese (1 Graph)
OPTIMIERE    "Sortiere moeglichst schnell"    → Pareto-Selektion (viele Varianten)
ERFINDE      "Verbessere sort(), erfinde Neues"→ Explorative Synthese + Evolution
ENTDECKE     "Finde Berechnung mit Eigenschaft X" → Offene Suche im Graphen-Raum
```

## Vergleich v2 → v3

```
Aspekt              v2                         v3
──────────────────────────────────────────────────────────────
Node-Typen          11                         7
Zwischenformat      JSON (menschenlesbar)      Binaer
Variablennamen      Ja                         Nein (Content-Hash)
SMT Timeout         5 Sekunden                 Kein Timeout
Unbewiesene Contr.  Runtime-Check              Graph UNGUELTIG
LLM-Iterationen     Max 3                      Unbegrenzt
Compile-Zeit        ~2 Sekunden                Minuten bis Stunden
Debug-Support       Ja (D-Node + Trace)        Keiner
Optimierung         LLVM -O3                   Superoptimizer
Runtime-Checks      0-N pro Binary             EXAKT 0
Korrektheitsgarantie Teilweise                 Total
Kreativitaet        Keine                      Explorative Synthese
Varianten           1 pro Anforderung          50-10000 Kandidaten
Wissensbasis        Keine                      Wachsendes Graph Repository
```

## Lizenz

MIT
