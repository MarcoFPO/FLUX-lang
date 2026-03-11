# FLUX Konzept — Expertenanalyse

Drei spezialisierte Agenten haben das FLUX-Konzept unabhaengig analysiert.

## Bewertungen

| Perspektive | Agent | Note | Kernaussage |
|---|---|---|---|
| **Architektur** | Backend-Architekt | **6/10** | Solide Ideen, kritische Luecken (Speichermodell, Runtime, Module, Fehlerbehandlung) |
| **KI-Generierbarkeit** | AI-Engineer | **4/10** | Syntaxfehler-Eliminierung loest nur 2-5% der Probleme; LLMs koennen keine komplexen DAGs zuverlaessig erzeugen |
| **Neuartigkeit** | Forschungsvergleich | **Evolutionaer+** | Einzelfeatures existieren alle — Kombination + KI-native Generierung ist genuint neu |

## Gemeinsame Kritikpunkte

### 1. Bootstrapping-Problem
Kein Trainingskorpus, keine Toolchain, keine Validierung — Henne-Ei-Problem.

### 2. LLMs erzeugen Sequenzen, keine Graphen
Structured Output loest Syntax, nicht Semantik. Graph-Kohaerenz ueber viele Nodes ist ein offenes Problem.

### 3. SMT-Verifikation skaliert nicht
Triviale Constraints: funktioniert. Reale Programme: NP-hart bis unentscheidbar.

### 4. Fehlende Konzeptteile (v1)
- Speichermodell
- Laufzeitsystem
- Modulsystem
- Fehlerbehandlung
- Debugging
- Migrationspfad (FFI)

**Alle Luecken wurden in FLUX v2 adressiert.**

## Vergleich mit existierenden Systemen

| FLUX-Feature | Existiert in |
|---|---|
| Graph-basierte IR | Sea of Nodes (1995), RVSDG, MLIR |
| Multi-Target | LLVM IR, MLIR, Thorin, WASM/Cranelift |
| Content-Addressierung | Unison (Sprache), Nix (Builds) |
| Effect-Tracking | RVSDG (State-Edges), Koka (algebraische Effekte) |
| Formale Verifikation | CompCert, CakeML |
| **KI erzeugt Graphen direkt** | **Nichts — genuint neu** |
| **Gesamtkomposition** | **Kein System vereint alle Features** |

## Empfehlungen (umgesetzt in v2)

1. ~~Eigene Backends~~ → LLVM via MLIR (existierende Infrastruktur)
2. ~~"Kein Parser"~~ → Ehrlich: JSON → Validator → Graph
3. ~~Kein Speichermodell~~ → Region-basiert, deterministisch
4. ~~Keine Fehlerbehandlung~~ → F-Nodes als Graph-Kanten
5. ~~SMT beweist alles~~ → Timeout-Strategie, entscheidbare Fragmente
6. ~~KI direkt perfekt~~ → Iterative Korrekturschleife mit Feedback
