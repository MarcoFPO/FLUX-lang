# FLUX v3 — Radikale Reduktion

## Design-Axiome

```
1. Compile-Zeit ist irrelevant           → Exhaustive Verifikation, Superoptimierung
2. Lesbarkeit ist irrelevant             → Kein Text, keine Namen, keine Kommentare
3. Menschliche Hilfskonstrukte entfallen → Kein Debug, kein Exception-Handling,
                                           keine defensive Programmierung,
                                           keine Abstraktion zur Verstaendlichkeit
4. Performance der Codegenerierung       → Beliebig viele LLM-Iterationen,
   ist sekundaer                           beliebig tiefe Analyse
5. Kreativitaet ist erwuenscht           → KI soll neuartige Loesungen ERFINDEN,
                                           nicht nur bekannte Muster reproduzieren
```

## Was gegenueber v2 wegfaellt

```
ENTFERNT          BEGRUENDUNG
──────────────────────────────────────────────────────────────────
D-Node (Debug)    Kein Mensch liest den Code. Tracing unnoetig.
H-Node (Hint)     Compile-Zeit unbegrenzt → Superoptimizer findet
                  optimale Instruktionen automatisch.
P-Node (Package)  Module existierten zur menschlichen Organisation.
                  KI arbeitet mit einem flachen Gesamt-Graph.
FINALLY           Menschliches Pattern fuer vergessliches Cleanup.
                  KI vergisst nicht — Region-Lifetime erzwingt Cleanup.
Variablennamen    Nur Content-Hashes. Namen sind Lesbarkeits-Artefakte.
Error Messages    Kein Mensch sieht sie. Fehler → Abort-Code (u8).
fault_policy      Keine "ignore" / "continue_without". Entweder der
                  Pfad ist korrekt oder er wird nicht erzeugt.
SMT Timeout       Kein Timeout. Prover laeuft so lange wie noetig.
                  Unbewiesene Contracts → Graph ist UNGUELTIG.
Iteration-Limit   Keine Begrenzung auf 3 Iterationen. LLM iteriert
                  bis der Graph korrekt ist oder aufgibt.
JSON Format       Menschenlesbares Zwischenformat unnoetig.
                  KI erzeugt direkt Binaer-Nodes.
```

## Was sich verschaerft

```
VERSCHAERFT       BEGRUENDUNG
──────────────────────────────────────────────────────────────────
V-Nodes           ALLE Contracts MUESSEN bewiesen werden.
                  Kein "Timeout → Runtime-Check" Fallback.
                  Unbewiesener Contract = ungueltiger Graph.
Superoptimierung  LLVM -O3 reicht nicht. Stochastic Superoptimization:
                  Exhaustive Suche nach optimaler Instruktionssequenz.
                  Kostet Minuten statt Millisekunden — irrelevant.
Exhaustive Test   Automatische Property-Tests aus ALLEN Contracts.
                  Nicht Stichproben, sondern Bounded Model Checking.
Totale Korrektheit Graph wird NICHT kompiliert wenn ein Contract
                  nicht bewiesen ist. Keine Kompromisse.
```


## 1. Architektur v3

```
Anforderung
    │
    ▼
KI-Generator (LLM, beliebig viele Iterationen)
    │
    ▼  Binaerer FLUX-Graph (kein JSON, kein Text)
    │
Validator (Struktur + Typen + Effekte + Regionen)
    │  FAIL → zurueck zum LLM (keine Iteration-Begrenzung)
    ▼
Contract Prover (Z3, KEIN Timeout)
    │  FAIL → zurueck zum LLM
    │  ALLE Contracts muessen PROVEN sein
    ▼
Superoptimizer (exhaustive Instruktionssuche)
    │  Zeit irrelevant — findet optimale Sequenz
    ▼
MLIR → LLVM → Maschinencode
```

## 2. Node-Typen (reduziert)

```
C-Node  (Compute)     Reine Berechnung
E-Node  (Effect)      Seiteneffekte (IO, Syscall)
K-Node  (Kontroll)    Seq | Par | Branch | Loop
V-Node  (Verify)      Contract — MUSS bewiesen werden
T-Node  (Type)        Typ mit Constraints
M-Node  (Memory)      ALLOC | LOAD | STORE (Region-gebunden)
R-Node  (Region)      Speicher-Lifetime
```

Entfernt: D-Node, H-Node, P-Node, F-Node.

**Warum kein F-Node:**
F-Nodes modellierten menschliche Fehlerbehandlungsmuster (try/catch, fallback, ignore).
Eine KI erzeugt stattdessen:
- Entweder einen Graph in dem der Fehlerfall **strukturell unmoeglich** ist
  (durch Contracts bewiesen)
- Oder alternative Datenflusspfade die **beide korrekt** sind
  (via K-Node BRANCH mit V-Node auf beiden Pfaden)

Fehler sind keine Sonderfaelle — sie sind normale Pfade im Graph, die denselben
Korrektheitsbeweis durchlaufen wie der Hauptpfad.


## 3. Binaeres Format v3

```
┌───────────────────────────────────────────┐
│ FLUX v3 Binary                             │
├───────────────────────────────────────────┤
│ magic:        0x464C5833 ("FLX3")          │
│ node_count:   u32                          │
│ type_count:   u16                          │
│ region_count: u16                          │
├───────────────────────────────────────────┤
│ Type Table    [T-Nodes]                    │
│ Region Table  [R-Nodes]                    │
│ Node Table    [C|E|K|V|M-Nodes]            │
│ Edge Table    [from, to, kind]             │
│   kind: Data | Control | Effect            │
│ Contract Table [V-Nodes: formula]          │
├───────────────────────────────────────────┤
│ BLAKE3 Content Hash                        │
└───────────────────────────────────────────┘

Entfernt gegenueber v2:
  - target_hint    (Superoptimizer waehlt pro Target)
  - Debug Table    (kein D-Node)
  - proven: bool   (alle Contracts muessen proven sein)
  - check_strategy (kein Runtime-Check Fallback)
  - Fault edges    (kein F-Node, kein Fault edge-type)
```


## 4. Identitaet: Content-Hash statt Namen

```
v2:  fn calculate_tax(income: PositiveInt, rate: Probability) → PositiveInt
v3:  C-Node blake3:7a2f... { type_in: [T:a3b1, T:c4d2], type_out: T:a3b1, op: MUL }

Keine Namen. Keine Aliase. Keine Kommentare.
Identitaet = BLAKE3(type_in + type_out + op + inputs)

Gleiche Berechnung → gleicher Hash → automatische Deduplizierung.
Kein Namespace-Management, kein Import-System, kein Namenskonflikt.
```


## 5. Speichermodell — Unveraendert, aber ohne Escape-Hatch

Region-basiert wie v2, aber:

```
v2: fault_policy: "ignore" bei OOM → weiter ohne Speicher
v3: OOM ist ein Contract-Versagen. Der Graph muss BEWEISEN
    dass der Speicherbedarf innerhalb der Region-Groesse liegt.

    V-Node: max_alloc(R:frame) <= R:frame.capacity

    Beweis scheitert → Graph ungueltig → LLM erzeugt neuen Graph
    mit korrektem Speicherbedarf.
```

Regeln (unveraendert):
1. Jedes M-Node gehoert zu genau einer R-Node
2. Referenzen nur in eigene oder aeussere Region
3. Regionen werden am Scope-Ende deterministisch freigegeben


## 6. Effekt-System — Verschaerft

```
v2: E-Node mit fault_policy: "ignore" | "abort" | "continue_without"
v3: E-Node ohne Policy. Jeder E-Node hat exakt ZWEI Ausgangskanten:

    E-Node (syscall_write)
        ├── success → C-Node (naechste Berechnung)
        └── failure → C-Node (alternativer Pfad)

    BEIDE Pfade muessen:
    - Typsicher sein
    - Vollstaendig durch V-Nodes abgedeckt sein
    - Zu einem terminierten Zustand fuehren

    Es gibt keine "ignorieren" oder "abbrechen" Option.
    Jeder Pfad ist ein vollwertiger, bewiesener Graph-Pfad.
```


## 7. Contract-System — Fehlertolerante Evolution

### Paradigmenwechsel: Quarantaene statt Hinrichtung

```
v2:                     v3-alt:                  v3-NEU:
─────────────────────────────────────────────────────────────────────
PROVEN   → 0 Overhead   PROVEN   → 0 Overhead    PROVEN   → 0 Overhead
DISPROVEN→ Feedback LLM  DISPROVEN→ VERWORFEN     DISPROVEN→ INKUBATION
TIMEOUT  → Runtime-Check TIMEOUT  → kein Timeout  TIMEOUT  → INKUBATION
```

Ein Graph mit verletztem Contract wird NICHT verworfen.
Er wird in die INKUBATIONS-ZONE verschoben.
Dort wird er weiter mutiert.
Eine Folge-Mutation kann den Fehler HEILEN — und dabei etwas
hervorbringen das BESSER ist als der fehlerfreie Ausgangszustand.

```
Der Weg durch den Fehler:

Graph A (korrekt, langsam)
    │
    │ Mutation M1
    ▼
Graph A' (FEHLERHAFT — Contract V3 verletzt)
    │
    │ In klassischem Modell: → VERWORFEN. Ende.
    │ In FLUX v3:            → INKUBATION. Weiter mutieren.
    │
    │ Mutation M2
    ▼
Graph A'' (KORREKT — alle Contracts erfuellt, UND 3x schneller!)
    │
    │ Die Kombination M1+M2 hat etwas erschaffen,
    │ das durch M2 ALLEIN nie entstanden waere.
    │ M1 war der "Fehler" der noetig war.
    │
    ▼
Ergebnis: Neuartiger Algorithmus, bewiesen korrekt, ueberlegen.
```

Biologische Parallele:
```
Sichelzelleanaemie:
  Mutation 1: Haemoglobin-Gen veraendert → rote Blutzellen sichelfoermig
              → SCHAEDLICH (Anaemie, Organschaeden)
              → In "perfektem Immunsystem" → eliminiert

  ABER: Sichelzellen sind resistent gegen Malaria.
  → In Malaria-Gebieten ist die "schadhafte" Mutation ein VORTEIL.
  → Heterozygote Traeger: milde Sichelzellen + Malaria-Resistenz = OPTIMAL.

  Haette das Immunsystem die erste Mutation sofort eliminiert,
  waere die Malaria-Resistenz NIE entstanden.

FLUX-Analogie:
  Mutation 1: Sort-Algorithmus verletzt Stabilitaets-Contract
              → FEHLERHAFT (instabile Sortierung)
              → In "perfektem Immunsystem" → eliminiert

  ABER: Die instabile Variante hat ein Cache-Zugriffsmuster
  das 5x schneller ist.

  Mutation 2: Repariert Stabilitaet durch Tie-Breaking auf Originalindex
              → KORREKT + 5x SCHNELLER als der Ausgangsgraph

  Der Umweg ueber den Fehler war der SCHLUESSEL.
```


### Drei Zustaende eines Graphen

```
┌─────────────────────────────────────────────────────────────────┐
│                                                                  │
│  GESUND (alle Contracts bewiesen)                               │
│  ═══════════════════════════════                                │
│  → Kann zu Binary kompiliert werden                             │
│  → Lebt in Elite-Zone oder Toleranz-Zone                       │
│  → Wird fuer Fitness bewertet                                   │
│  → Darf gekreuzt werden                                        │
│                                                                  │
│  INKUBIERT (mindestens 1 Contract verletzt oder unbewiesen)     │
│  ══════════════════════════════════════════════════════════      │
│  → Kann NICHT zu Binary kompiliert werden                       │
│  → Lebt in der Inkubations-Zone (isoliert)                     │
│  → Wird WEITER MUTIERT (mit erhoehter Rate)                    │
│  → Wird NICHT fuer Fitness bewertet (noch nicht lauffaehig)     │
│  → Traegt Markierung: welche Contracts verletzt sind            │
│  → Traegt Markierung: wie viele Generationen in Inkubation     │
│  → Kann durch Mutation GESUND werden → aufsteigen              │
│  → Wird nach N Generationen ohne Heilung entfernt              │
│                                                                  │
│  TOT (strukturell ungueltig)                                    │
│  ═══════════════════════════                                    │
│  → Validator FAIL (kein DAG, Typ-Fehler, Region-Escape)        │
│  → Kann NICHT repariert werden durch einfache Mutation          │
│  → Wird sofort verworfen                                       │
│  → EINZIGER Zustand der zur Eliminierung fuehrt                │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘

Unterscheidung: STRUKTURELL ungueltig vs. SEMANTISCH fehlerhaft

  STRUKTURELL ungueltig:          SEMANTISCH fehlerhaft:
  Zyklen im DAG                   Contract verletzt
  Typ-Mismatch an Kanten          Falsches Ergebnis
  Region-Escape                   Out-of-Bounds (unbewiesen)
  Fehlende Inputs                 Terminierung nicht beweisbar

  → TOT (nicht reparierbar        → INKUBIERT (reparierbar
    durch Punkt-Mutation)           durch Punkt-Mutation)
```


### Inkubations-Zone: Regeln

```
1. AUFNAHME
   Graph besteht Validator (strukturell ok)
   ABER: mindestens 1 V-Node ist DISPROVEN oder UNRESOLVED
   → Graph wird in Inkubations-Zone aufgenommen
   → Markierung: {verletzt: [V3, V7], generation: 0}

2. MUTATION
   Inkubierte Graphen werden mit ERHOEHTER RATE mutiert:
   - Normal: 30% Mutation pro Generation
   - Inkubiert: 60% Mutation pro Generation
   - Mutationen werden GEZIELT in der Naehe der verletzten
     Contracts angewendet (der Prover liefert Gegenbeispiele
     die als Hinweis dienen WO der Fehler liegt)

3. RE-EVALUATION
   Nach jeder Mutation:
   - Validator: strukturell ok? → weiter. Sonst → TOT.
   - Prover: Contracts pruefen.
     → Alle PROVEN: HEILUNG! → Aufstieg in Toleranz-Zone
     → Weniger Verletzungen als vorher: Fortschritt, weiter mutieren
     → Mehr Verletzungen: Rueckschritt, aber NICHT eliminieren
       (Rueckschritte koennen spaeter zu Spruengen fuehren)

4. ALTERUNG
   Inkubations-Zaehler steigt pro Generation.
   Nach MAX_INCUBATION Generationen ohne Heilung:
   → Graph wird entfernt (Ressourcen-Limit)
   → Aber: Graph wird im ARCHIV gespeichert (nicht im aktiven Pool)
   → Archivierte Graphen koennen spaeter wiederbelebt werden
     wenn neue Mutations-Strategien verfuegbar sind

5. HEILUNG
   Wenn ein inkubierter Graph alle Contracts erfuellt:
   → Sofortige Fitness-Bewertung
   → Aufstieg in Elite/Toleranz basierend auf Fitness
   → BONUS: Geheilte Graphen erhalten Neuartigkeits-Bonus
     (der Umweg durch den Fehler hat oft ungewoehnliche
      Strukturen hervorgebracht)
```


### Pool-Architektur (revidiert)

```
┌──────────────────────────────────────────────────────────────┐
│  POOL (1000-10000 Graphen)                                    │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ ELITE-ZONE (10%)                                       │  │
│  │ Bewiesen korrekt. Beste Fitness.                       │  │
│  │ Werden nie entfernt. Eltern fuer Kreuzung.             │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ TOLERANZ-ZONE (40%)                                    │  │
│  │ Bewiesen korrekt. Neutrale Drift.                      │  │
│  │ Kein Selektionsdruck. Akkumulation.                    │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ INKUBATIONS-ZONE (30%)                                 │  │
│  │ Contract-Verletzungen. Strukturell valide.             │  │
│  │ Erhoehte Mutationsrate. Gezielte Reparatur-Mutationen.│  │
│  │ Koennen durch Mutation HEILEN → Aufstieg.             │  │
│  │ Werden nach MAX_INCUBATION archiviert.                │  │
│  │                                                        │  │
│  │ Markierungen pro Graph:                                │  │
│  │   violated_contracts: [V3, V7]                         │  │
│  │   counterexamples: [{input: [5,3,1], expected: ...}]  │  │
│  │   incubation_gen: 47                                   │  │
│  │   healing_trend: improving / stagnating / regressing   │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ PRUEF-ZONE (20%)                                       │  │
│  │ Neue Mutationen und Kreuzungen.                        │  │
│  │ Validator → Prover → Routing:                          │  │
│  │   Alle Contracts proven → Toleranz/Elite               │  │
│  │   Manche Contracts verletzt → Inkubation               │  │
│  │   Strukturell ungueltig → TOT (verwerfen)             │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ ARCHIV (unbegrenzt, persistent)                        │  │
│  │ Entfernte Inkubations-Graphen.                         │  │
│  │ Nicht im aktiven Pool. Kein Ressourcenverbrauch.       │  │
│  │ Koennen wiederbelebt werden bei neuen Strategien.      │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘

Fluss:
  Pruef-Zone ──proven──→ Toleranz-Zone ──beste──→ Elite-Zone
      │                       │
      │──verletzt──→ Inkubations-Zone ──geheilt──→ Toleranz-Zone
      │                       │
      │──strukturell──→ TOT   │──timeout──→ Archiv
         ungueltig                          (wiederbelebbar)
```


### Kompilierungs-Gate

```
NUR Graphen aus Elite-Zone oder Toleranz-Zone koennen zu Binaries
kompiliert werden. Das garantiert:

  ✓ Jedes BINARY ist bewiesen korrekt
  ✗ Nicht jeder GRAPH im Pool ist korrekt (Inkubations-Zone)

Die Inkubations-Zone ist ein LABOR, kein Produktionssystem.
Fehlerhafte Graphen existieren, werden erforscht, koennen heilen.
Aber sie verlassen das Labor erst wenn sie gesund sind.

  Pool-Ebene:  Fehlertoleranz (Inkubation, Mutation, Heilung)
  Binary-Ebene: Totale Korrektheit (nur bewiesene Graphen)
```


### Prover-Strategie (revidiert)

```
Fuer GESUNDE Graphen (→ Binary):
  Phase 1: Z3 / CVC5 (automatische SMT-Solver)
  Phase 2: Bounded Model Checking (CBMC, KLEE)
  Phase 3: Symbolische Ausfuehrung
  Phase 4: Lean 4 / Coq (KI erzeugt Beweis)
  Phase 5: Exhaustive Enumeration
  → Alle Phasen, kein Timeout, Binary nur wenn PROVEN

Fuer INKUBIERTE Graphen (→ Diagnose):
  Nur Phase 1: Z3 schnell-Check (Timeout: 10s)
  Ziel: Nicht beweisen, sondern GEGENBEISPIELE finden
  Gegenbeispiele leiten gezielte Reparatur-Mutationen
  → Schnelle Diagnose, nicht vollstaendiger Beweis
```


## 8. Superoptimierung (statt LLVM -O3)

Compile-Zeit irrelevant → optimale Instruktionssequenz per Exhaustive Search.

```
Ablauf:

1. LLVM -O3 erzeugt Baseline-Maschinencode
2. Fuer jede Funktion < 64 Instruktionen:
   Stochastic Superoptimizer (STOKE-aehnlich):
   - Zufaellige Instruktionssequenzen generieren
   - Testen ob semantisch aequivalent (via SMT)
   - Kuerzeste / schnellste Sequenz waehlen
3. Fuer groessere Funktionen:
   LLVM -O3 + zusaetzliche MLIR-Passes

Ergebnis:
- Maschinencode der manuell geschriebenem Assembler
  UEBERLEGEN sein kann
- Instruktionskombinationen die kein Mensch finden wuerde
- Kostet Minuten pro Funktion — irrelevant
```

**Beispiel: sum_array**

```
LLVM -O3 (gut):                  Superoptimized (optimal):
  vxorpd    ymm0, ymm0, ymm0      vxorpd  ymm0, ymm0, ymm0
  mov       rcx, rsi               ; Loop komplett eliminiert
  shr       rcx, 2                 ; durch rekursives AVX-Falten:
.loop:                               vaddpd  ymm0, ymm0, [rdi]
  vaddpd    ymm0, ymm0, [rdi]       vaddpd  ymm0, ymm0, [rdi+32]
  add       rdi, 32                  ...
  dec       rcx                      ; Vollstaendig entrollt fuer
  jnz       .loop                    ; bekannte Array-Groessen
  ; horizontal sum                   vhaddpd xmm0, xmm0, xmm0
  vextractf128 xmm1, ymm0, 1        ret
  vaddpd    xmm0, xmm0, xmm1
  vhaddpd   xmm0, xmm0, xmm0      ; 40% weniger Instruktionen
  ret                               ; 0 Branches
```


## 9. Explorative Synthese — KI als Erfinder

### Das Problem mit reiner Uebersetzung

```
Bisheriges Modell:
  Anforderung → bekanntes Muster → Graph → Binary

  "Sortiere ein Array"  → Mergesort → Graph → Binary

  Die KI REPRODUZIERT. Sie erfindet nichts.
  Das ist eine Schreibmaschine, kein Ingenieur.
```

### Das neue Modell: Korrektheit als Filter, Kreativitaet als Generator

```
Neues Modell:
  Anforderung → VIELE Varianten erzeugen → filtern → bewerten → beste waehlen

  "Sortiere ein Array"  → 200 verschiedene Graphen
                         → 140 bestehen Validator
                         → 89 sind bewiesen korrekt
                         → Fitness-Bewertung
                         → Variante #67: neuartiger Hybrid aus
                           Radixsort + Insertionsort der fuer
                           diese Datenverteilung 3x schneller ist
                           als alles Bekannte

  Die KI ERFINDET. Korrektheit ist der Sicherheitsgurt, nicht das Lenkrad.
```

### Architektur der Explorativen Synthese

```
┌──────────────────────────────────────────────────────────────┐
│  PHASE 1: DIVERGENZ (viele Varianten erzeugen)               │
│                                                               │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐             │
│  │ LLM        │  │ Genetischer│  │ Constraint- │             │
│  │ Varianten  │  │ Algorithmus│  │ Relaxation  │             │
│  │            │  │ auf Graphen│  │             │             │
│  │ "Erzeuge   │  │            │  │ "Lockere    │             │
│  │  10 grund- │  │ Mutation:  │  │  Constraint │             │
│  │  verschiedene│ │ Nodes      │  │  X und suche│             │
│  │  Loesungen"│  │ ersetzen,  │  │  im groesse-│             │
│  │            │  │ Kanten     │  │  ren Raum"  │             │
│  │            │  │ umleiten,  │  │             │             │
│  │            │  │ Subgraphen │  │             │             │
│  │            │  │ kreuzen    │  │             │             │
│  └─────┬──────┘  └─────┬──────┘  └──────┬──────┘            │
│        │               │                │                    │
│        └───────────────┼────────────────┘                    │
│                        │                                     │
│                        ▼                                     │
│              Pool: N Kandidaten-Graphen                      │
│              (N = 50..10000, Zeit irrelevant)                │
└────────────────────────┬─────────────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────────────┐
│  PHASE 2: SELEKTION (harter Filter)                          │
│                                                               │
│  Validator:  Struktur + Typen + Effekte + Regionen           │
│              → Ungueltige Graphen verwerfen                  │
│                                                               │
│  Contract Prover:  Alle V-Nodes muessen bewiesen werden      │
│              → Unbeweisbare Graphen verwerfen                │
│                                                               │
│  Ergebnis: M korrekte Kandidaten (M <= N)                    │
└────────────────────────┬─────────────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────────────┐
│  PHASE 3: BEWERTUNG (Fitness jenseits Korrektheit)           │
│                                                               │
│  Jeder korrekte Kandidat wird bewertet nach:                 │
│                                                               │
│  ┌─────────────────────────────────────────────────────┐    │
│  │ Fitness-Dimensionen:                                 │    │
│  │                                                      │    │
│  │ Throughput       Instruktionen / Sekunde             │    │
│  │                  (gemessen via Sandbox-Execution)     │    │
│  │                                                      │    │
│  │ Speicher         Peak Memory / Gesamtallokation      │    │
│  │                  (statisch berechnet aus R-Nodes)     │    │
│  │                                                      │    │
│  │ Energie          Geschaetzte Instruktions-Energie     │    │
│  │                  (gewichtet nach Op-Typ und Target)   │    │
│  │                                                      │    │
│  │ Latenz           Worst-Case Ausfuehrungszeit          │    │
│  │                  (WCET-Analyse oder Messung)          │    │
│  │                                                      │    │
│  │ Graph-Groesse    Anzahl Nodes (weniger = besser)      │    │
│  │                                                      │    │
│  │ NEUARTIGKEIT     Distanz zu bekannten Loesungen      │    │
│  │                  (Structural Graph Distance)          │    │
│  └─────────────────────────────────────────────────────┘    │
│                                                               │
│  Bewertung ist MEHRDIMENSIONAL — kein einzelner Score.       │
│  Pareto-Front: Alle Kandidaten die in mindestens einer       │
│  Dimension ungeschlagen sind, ueberleben.                    │
└────────────────────────┬─────────────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────────────┐
│  PHASE 4: EVOLUTION (optional, bei "erfinde etwas Neues")    │
│                                                               │
│  Die besten Kandidaten aus Phase 3 werden:                   │
│                                                               │
│  1. MUTIERT:  Zufaellige Aenderungen an Nodes/Kanten         │
│               (Op ersetzen, Subgraph umstrukturieren,        │
│                Parallelismus einfuegen/entfernen)            │
│                                                               │
│  2. GEKREUZT: Subgraphen zwischen zwei korrekten             │
│               Kandidaten austauschen                         │
│                                                               │
│  3. REINJIZIERT: LLM analysiert die beste Variante           │
│               und erzeugt bewusst ABWEICHENDE Variationen    │
│               ("Was waere wenn der innere Loop anders         │
│                strukturiert waere?")                          │
│                                                               │
│  → Zurueck zu Phase 2 (Filter) → Phase 3 (Bewerten)         │
│  → Wiederholen bis Konvergenz oder Abbruchkriterium          │
│                                                               │
│  Generationen: 10..1000 (Zeit irrelevant)                    │
└────────────────────────┬─────────────────────────────────────┘
                         │
                         ▼
                  Bester Kandidat → Superoptimizer → Binary
```


### Die drei Synthese-Strategien

**Strategie 1: LLM-Divergenz (breit, schnell)**

```
Prompt an LLM:

  "Erzeuge 10 grundverschiedene FLUX-Graphen fuer:
   Sortiere ein Array von 64-bit Integers.

   Variante 1: Vergleichsbasiert
   Variante 2: Nicht-vergleichsbasiert
   Variante 3: Adaptiv (fast-sortierte Daten)
   Variante 4: Cache-optimiert (wenige Memory-Zugriffe)
   Variante 5: Minimale Branches
   Variante 6: Maximal parallel (SPAWN/JOIN)
   Variante 7: Minimaler Speicher (in-place)
   Variante 8: Hybride Strategie
   Variante 9: Unkonventionell — brich eine Regel
   Variante 10: Frei — ueberrasche"

Ergebnis: 10 strukturell verschiedene Graphen,
          die dasselbe Interface und dieselben Contracts erfuellen.
```

**Strategie 2: Genetische Graph-Evolution (tief, langsam)**

```
Population:
  100 korrekte Graphen (aus LLM-Divergenz oder Mutation)

Mutation-Operatoren auf Graph-Ebene:
  ┌────────────────────────────────────────────────────────┐
  │ NODE_REPLACE   Ersetze C-Node(ADD) durch C-Node(SUB)  │
  │                + Kompensation (V-Node muss halten)     │
  │                                                        │
  │ EDGE_REWIRE    Verbinde Input eines Nodes mit einem    │
  │                anderen Node gleichen Typs              │
  │                                                        │
  │ SUBGRAPH_SWAP  Tausche Subgraph A gegen Subgraph B    │
  │                aus einem anderen Kandidaten            │
  │                                                        │
  │ LOOP_TRANSFORM Aendere Loop-Strategie:                 │
  │                Iterativ ↔ Rekursiv ↔ Entrollt          │
  │                ↔ Vektorisiert ↔ Parallel               │
  │                                                        │
  │ BRANCH_ELIM    Ersetze BRANCH durch branchless         │
  │                Arithmetik (CMOV-Pattern)               │
  │                                                        │
  │ PAR_INSERT     Fuege Parallelismus ein wo              │
  │                keine Datenabhaengigkeit besteht        │
  │                                                        │
  │ REGION_MERGE   Vereinige zwei Regionen                 │
  │                (weniger alloc/free, groesserer Block)   │
  │                                                        │
  │ CONST_FOLD     Ersetze berechenbare Subgraphen         │
  │                durch ihre Ergebnisse                   │
  │                                                        │
  │ NOVEL_INSERT   Fuege einen zufaelligen, typkorrekten   │
  │                Subgraphen ein und pruefe ob             │
  │                die Contracts noch gelten               │
  └────────────────────────────────────────────────────────┘

Selection:  NSGA-II (Multi-Objective)
            Pareto-Front aus Throughput × Speicher × Neuartigkeit

Generationen: bis Konvergenz (typisch 50-500)
```

**Strategie 3: Constraint-Relaxation (gezielt, explorativ)**

```
Ausgangslage:
  "Verbessere die Bildkompression"

  Bestehender Graph: DCT-basiert (wie JPEG)
  Bestehende Contracts:
    V1: output.size < input.size * ratio
    V2: decompress(compress(input)) == input  (verlustfrei)

Schritt 1: Relaxation
  V2 aendern: decompress(compress(input)) ≈ input  (Distanz < epsilon)
  → Groesserer Loesungsraum (verlustbehaftet erlaubt)

Schritt 2: Exploration
  LLM generiert Varianten im erweiterten Raum:
  - Wavelet-basiert statt DCT
  - Fraktale Kompression
  - Neuronale Kompression (Lookup-Table aus Training)
  - Hybrid: grobe DCT + feine Wavelet-Korrektur
  - UNBEKANNT: etwas das kein Name hat

Schritt 3: Re-Verifikation
  Alle Varianten muessen die relaxierten Contracts erfuellen.
  Zusaetzlich: empirische Qualitaetsmessung (SSIM, PSNR)

Schritt 4: Pareto-Selektion
  Kompressionsrate × Qualitaet × Geschwindigkeit
  → Variante #4 (Hybrid) ist auf der Pareto-Front
  → Variante #5 (UNBEKANNT) hat besten Qualitaets/Groessen-Tradeoff
```


### Neuartigkeits-Metrik

Wie misst man ob eine Loesung "neu" ist?

```
Structural Graph Distance (SGD):

  Gegeben: Graph G_neu und eine Datenbank bekannter Graphen {G_1, ..., G_k}

  SGD(G_neu) = min_i( EditDistance(G_neu, G_i) )

  EditDistance zaehlt:
    - Nodes hinzugefuegt/entfernt
    - Ops geaendert (ADD→MUL = 1 Edit)
    - Kanten umgeleitet
    - Subgraphen ersetzt

  Hoher SGD = hohe Neuartigkeit = bevorzugt in Pareto-Selektion

  Die Datenbank waechst mit jeder Synthese:
  → Jedes akzeptierte Binary wird zum Referenzpunkt
  → System wird ueber Zeit kreativer, weil es sich von
     immer mehr bekannten Loesungen entfernen muss
```


### Konkretes Beispiel: "Verbessere sort()"

```
Anforderung:
  "Verbessere die Sortierung von 64-bit Integer-Arrays.
   Erfinde dafuer etwas Neues."

Contracts (unverhandelbar):
  V1: result ist aufsteigend sortiert
  V2: result ist Permutation von input
  V3: Terminierung fuer alle endlichen Inputs

Phase 1 — Divergenz:
  LLM erzeugt 50 Varianten + Genetischer Algo erzeugt 150 Mutationen
  = 200 Kandidaten

Phase 2 — Selektion:
  187 bestehen Validator
  134 sind bewiesen korrekt (V1, V2, V3 alle proven)
  66 verworfen (nicht beweisbar oder inkorrekt)

Phase 3 — Bewertung (Sandbox-Execution, 10M Elemente):

  Kandidat   Typ                    Throughput   Speicher  SGD
  ────────────────────────────────────────────────────────────
  #012       Mergesort (Standard)   480 MB/s     O(n)      0.0
  #023       Radixsort              890 MB/s     O(n)      3.2
  #041       Introsort              510 MB/s     O(log n)  2.1
  #067       ??? (KI-Erfindung)     1120 MB/s    O(√n)     8.7  ← NEU
  #089       Parallel Mergesort     1450 MB/s    O(n)      4.1
  #134       ??? (KI-Erfindung)     920 MB/s     O(1)      9.3  ← NEU

  Pareto-Front: {#067, #089, #134}

Phase 4 — Analyse von #067 (beste Neuartigkeit + Throughput):

  Was hat die KI erfunden?

  Graph-Struktur (automatisch rekonstruiert):
  1. Grobe Partitionierung durch Radix auf die oberen 16 Bits
     (256 Buckets, cache-line-aligned, branchless Verteilung)
  2. Pro Bucket: Insertionsort wenn n < 32, sonst Bitonic Sort
  3. Merge-Phase eliminiert: Buckets sind bereits geordnet
     durch die Radix-Partitionierung
  4. SIMD-Nutzung: 4-Element Sorting Networks via VMIN/VMAX
     innerhalb der Insertionsort-Phase

  Kein bekannter Algorithmus. Hybrid aus Radix + Bitonic + SIMD-Networks.
  Bewiesen korrekt. 2.3x schneller als std::sort fuer gleichverteilte Daten.

  Variante #067 → Superoptimizer → Binary
```


### Wachsende Wissensbasis

```
Jedes akzeptierte Binary erweitert die Wissensbasis:

┌──────────────────────────────────────────────────────────────┐
│  GRAPH REPOSITORY (Content-Addressiert)                      │
│                                                               │
│  Jeder Graph wird gespeichert mit:                           │
│  - Content-Hash (Identitaet)                                 │
│  - Bewiesene Contracts (was er garantiert)                   │
│  - Fitness-Profil (Throughput, Speicher, Latenz, ...)        │
│  - Neuartigkeits-Score zum Zeitpunkt der Erzeugung          │
│  - Abstammung (aus welcher Mutation/Kreuzung entstanden)     │
│                                                               │
│  Nutzen:                                                     │
│  - Subgraphen koennen in neuen Synthesen wiederverwendet     │
│    werden (bewiesene Korrektheit bleibt erhalten)            │
│  - Neuartigkeits-Metrik wird schaerfer (mehr Referenzpunkte) │
│  - Genetische Kreuzung nutzt bewaehrte Subgraphen            │
│  - KI lernt welche Strukturen in welchen Kontexten           │
│    ueberlegen sind                                           │
│                                                               │
│  Wachstum:                                                   │
│  Tag 1:     ~100 Referenz-Graphen (Standardalgorithmen)     │
│  Monat 1:   ~5.000 (erste kreative Varianten)               │
│  Jahr 1:    ~500.000 (eigenes Oekosystem)                   │
│  Langfristig: Millionen — eigene "Algorithmische DNA"        │
└──────────────────────────────────────────────────────────────┘
```


### Biologisches Modell: Zellulaere Mutation als Vorbild

Die explorative Synthese in FLUX folgt nicht dem klassischen genetischen
Algorithmus (Mutation → sofortige Bewertung → Selektion). Sie folgt dem
biologischen Modell der zellulaeren Mutation:

```
BIOLOGIE                              FLUX
────────────────────────────────────────────────────────────────────

DNA-Replikationsfehler                Zufaellige Graph-Mutation
(ein Basenpaar aendert sich)          (ein Node/eine Kante aendert sich)
    │                                     │
    ▼                                     ▼
Zelle funktioniert noch?              Contracts noch erfuellt?
    │                                     │
    ├── Nein → Apoptose (Zelltod)     ├── Nein → Graph verworfen
    │                                     │
    └── Ja → Zelle lebt weiter        └── Ja → Graph bleibt im Pool
         │                                  │
         │   Die Mutation ist NEUTRAL.      │   Die Mutation ist NEUTRAL.
         │   Weder besser noch              │   Weder schneller noch
         │   schlechter.                    │   langsamer.
         │   Aber sie EXISTIERT.            │   Aber sie EXISTIERT.
         │                                  │
         ▼                                  ▼
    Weitere Mutationen                 Weitere Mutationen
    akkumulieren auf                   akkumulieren auf
    der veraenderten Zelle             dem veraenderten Graph
         │                                  │
         ▼                                  ▼
    Wucherung entsteht                 Subgraph waechst
    (neues Gewebe,                     (neue Struktur,
     neue Struktur)                     neue Berechnungspfade)
         │                                  │
         │   Die Wucherung STOERT           │   Die Variante ist ANDERS.
         │   NICHT UNBEDINGT.               │   Nicht unbedingt besser.
         │   Sie existiert einfach.         │   Sie existiert einfach.
         │                                  │
         ▼                                  ▼
    Weitere Mutationen                 Weitere Mutationen
    auf der Wucherung                  auf der Variante
         │                                  │
         ▼                                  ▼
    ┌─────────────────┐                ┌─────────────────┐
    │ EMERGENZ:       │                │ EMERGENZ:       │
    │ Neue Eigenschaft│                │ Neuer Algorithmus│
    │ die vorher      │                │ der vorher       │
    │ nicht existierte│                │ nicht existierte │
    └────────┬────────┘                └────────┬────────┘
             │                                  │
             ▼                                  ▼
    Bewertung durch                    Bewertung durch
    den Organismus:                    Fitness-Funktion:
    ┌──────────────┐                   ┌──────────────┐
    │ POSITIV:     │                   │ POSITIV:     │
    │ Anpassung,   │                   │ Schneller,   │
    │ neues Organ, │                   │ kompakter,   │
    │ Resistenz    │                   │ neuartiger   │
    │ → BEHALTEN   │                   │ → BEHALTEN   │
    ├──────────────┤                   ├──────────────┤
    │ NEUTRAL:     │                   │ NEUTRAL:     │
    │ Kein Effekt, │                   │ Gleiche      │
    │ keine Kosten │                   │ Performance  │
    │ → TOLERIEREN │                   │ → TOLERIEREN │
    ├──────────────┤                   ├──────────────┤
    │ NEGATIV:     │                   │ NEGATIV:     │
    │ Krebs,       │                   │ Langsamer,   │
    │ Funktions-   │                   │ groesser,    │
    │ verlust      │                   │ Contract-    │
    │ → ELIMINIEREN│                   │ Verletzung   │
    └──────────────┘                   │ → ELIMINIEREN│
                                       └──────────────┘
```


**Der entscheidende Punkt: NEUTRALE MUTATIONEN UEBERLEBEN.**

```
Klassischer GA:
  Mutation → sofort bewerten → nur die Besten ueberleben
  → VERLIERT genetische Vielfalt
  → Konvergiert schnell auf lokales Optimum
  → Findet keine fundamental neuen Loesungen

Biologisches Modell (FLUX v3):
  Mutation → Contract-Check → wenn nicht schaedlich: BEHALTEN
  → Neutrale Varianten akkumulieren
  → Genetische Vielfalt bleibt erhalten
  → Irgendwann: Kombination neutraler Mutationen = qualitativer Sprung
  → Findet Loesungen die kein direkter Weg erreicht

Das ist Kimuras "Neutral Theory of Molecular Evolution" (1968):
Die meisten Mutationen sind neutral. Aber sie sind das ROHMATERIAL
aus dem spaeter Innovation entsteht.
```


**Das Immunsystem: V-Nodes als Diagnose, nicht als Todesurteil**

```
Biologie:
  Immunsystem erkennt entartete Zellen → Eliminierung
  ABER: Immunsystem ist NICHT perfekt
  → Krebs kann durchrutschen
  → Autoimmun kann gesunde Zellen zerstoeren
  → UND: Manchmal ist die "Entartung" nuetzlich
    (Sichelzellen → Malaria-Resistenz)

FLUX:
  V-Nodes erkennen fehlerhafte Graphen → INKUBATION (nicht Eliminierung)
  Der Fehler wird DIAGNOSTIZIERT, nicht BESTRAFT.

  Zwei Schutzschichten:
  1. POOL-EBENE: Fehlertolerant.
     Fehlerhafte Graphen werden isoliert weitergefuehrt.
     Gezielte Mutationen versuchen Heilung.
     Der Fehler kann der SCHLUESSEL zur Innovation sein.

  2. BINARY-EBENE: Unfehlbar.
     Nur bewiesene Graphen werden kompiliert.
     Kein fehlerhaftes Binary kann je entstehen.

  FLUX hat KEIN perfektes Immunsystem — es hat ein KLUGES:
  Es eliminiert nicht blind, es forscht.
  Es toetet nicht den Patienten, es heilt ihn.
  Und manchmal ist die Krankheit die Kur.
```


**Phasen der kumulativen Mutation in FLUX:**

```
┌──────────────────────────────────────────────────────────────┐
│  PHASE 0: GENESIS                                            │
│  LLM erzeugt Ausgangs-Population (50-100 Graphen)           │
│  Alle erfuellen Contracts. Manche sind besser, manche nicht. │
│  Die meisten sind Varianten bekannter Algorithmen.           │
└──────────────────────────┬───────────────────────────────────┘
                           │
┌──────────────────────────▼───────────────────────────────────┐
│  PHASE 1: NEUTRALE DRIFT (Generationen 1-50)                │
│                                                               │
│  Kleine Mutationen: ein Node ersetzen, eine Kante umleiten.  │
│  Die meisten aendern die Performance NICHT messbar.          │
│  Aber sie veraendern die STRUKTUR des Graphen.               │
│                                                               │
│  Beispiel:                                                   │
│    Graph #42 hat einen Subgraph A → B → C                    │
│    Mutation: B wird durch B' ersetzt (gleiche Funktion,      │
│    andere Implementierung)                                    │
│    Performance: identisch. Contract: erfuellt.               │
│    → BEHALTEN.                                               │
│                                                               │
│  Der Pool diversifiziert sich OHNE Selektionsdruck.          │
│  Vielfalt steigt. Noch keine Innovation sichtbar.            │
└──────────────────────────┬───────────────────────────────────┘
                           │
┌──────────────────────────▼───────────────────────────────────┐
│  PHASE 2: WUCHERUNG (Generationen 50-200)                    │
│                                                               │
│  Groessere Mutationen: Subgraphen wachsen, neue Pfade        │
│  entstehen, redundante Berechnungen werden eingefuegt.       │
│  Manche Mutationen VERLETZEN Contracts.                      │
│                                                               │
│  Die Graphen werden GROESSER, KOMPLEXER, und TEILWEISE       │
│  FEHLERHAFT. Das ist die "Wucherung" — neues Gewebe das      │
│  noch keine klare Funktion hat und manchmal stoert.          │
│                                                               │
│  Beispiel:                                                   │
│    Graph #42 (Generation 73) hat jetzt:                      │
│    - Einen zusaetzlichen Vorverarbeitungs-Subgraph           │
│    - Eine redundante Berechnung die nie genutzt wird         │
│    - Einen alternativen Pfad der bei bestimmten Inputs       │
│      aktiv wird                                              │
│    - Contract V3 ist VERLETZT (Sortierung instabil)          │
│      → Graph wandert in Inkubations-Zone                     │
│      → Wird weiter mutiert                                   │
│                                                               │
│  Das ist KEIN Fehler des Systems — das ist das MATERIAL      │
│  aus dem Innovation entsteht.                                │
│                                                               │
│  Die Wucherung DARF stoeren. Sie DARF fehlerhaft sein.       │
│  Sie wird nicht eliminiert. Sie wird weiterentwickelt.       │
└──────────────────────────┬───────────────────────────────────┘
                           │
┌──────────────────────────▼───────────────────────────────────┐
│  PHASE 3: EMERGENZ (Generationen 200+)                       │
│                                                               │
│  Die akkumulierten Mutationen INTERAGIEREN.                  │
│  Zwei neutrale Aenderungen kombiniert ergeben einen          │
│  qualitativen Sprung.                                        │
│                                                               │
│  Beispiel:                                                   │
│    Graph #42 (Generation 217):                               │
│    - War seit Generation 73 in der INKUBATIONS-ZONE          │
│      (Contract V3 verletzt: instabile Sortierung)            │
│    - Generation 73-216: 143 weitere Mutationen               │
│      manche verbesserten, manche verschlechterten            │
│      der Graph BLIEB fehlerhaft — wurde TOLERIERT            │
│    - Generation 217: Mutation M144 fuegt Tie-Breaking ein    │
│      (Originalindex als sekundaerer Sortierschluessel)       │
│      → Contract V3 ist WIEDER ERFUELLT (stabile Sortierung)  │
│      → HEILUNG! Graph steigt auf in Toleranz-Zone            │
│                                                               │
│  ABER: Der Graph ist nicht nur "repariert".                   │
│  Durch die 143 Mutationen WAEHREND der Inkubation hat er:    │
│    - Ein Cache-Zugriffsmuster das zufaellig entstand         │
│    - Eine Partitionierung die fuer SIMD optimiert ist        │
│    - Einen alternativen Pfad fuer fast-sortierte Daten       │
│                                                               │
│  Ergebnis: Ein NEUARTIGER ALGORITHMUS                        │
│    - Niemand hat ihn entworfen                               │
│    - Er entstand durch den UMWEG UEBER DEN FEHLER            │
│    - Er ist BEWIESEN korrekt (alle Contracts gelten)         │
│    - Er ist 5x SCHNELLER als der Ausgangsgraph               │
│    - Der "Fehler" war NOTWENDIG fuer die Innovation          │
│    - Ohne Inkubation waere er in Generation 73 eliminiert    │
│      worden und diese Loesung haette nie existiert           │
│                                                               │
│  Fitness-Bewertung: → Pareto-Front, ueberlegen              │
│  → Aufstieg in ELITE-ZONE                                   │
└──────────────────────────┬───────────────────────────────────┘
                           │
┌──────────────────────────▼───────────────────────────────────┐
│  PHASE 4: RADIATION (nach Emergenz)                          │
│                                                               │
│  Der emergente Graph wird zum neuen Ausgangspunkt.           │
│  Weitere Mutationen erzeugen VARIANTEN der Emergenz.         │
│  Spezialisierung auf verschiedene Kontexte:                  │
│                                                               │
│  Emergenz #42.217                                            │
│    ├── Variante A: optimiert fuer kleine Arrays (n < 100)    │
│    ├── Variante B: optimiert fuer fast-sortierte Daten       │
│    ├── Variante C: optimiert fuer gleichverteilte Daten      │
│    └── Variante D: optimiert fuer minimalen Speicher         │
│                                                               │
│  Wie in der Biologie: Nach einer erfolgreichen Mutation      │
│  folgt adaptive Radiation — Spezialisierung in Nischen.      │
└──────────────────────────────────────────────────────────────┘
```


**Implementierung: Population-Pool mit Toleranz-Zonen**

```
Pool-Struktur:

┌──────────────────────────────────────────────────────────────┐
│  ELITE-ZONE (Top 10%)                                        │
│  Beste Fitness. Werden NIE entfernt.                         │
│  Dienen als Eltern fuer Kreuzung.                            │
│                                                               │
│  TOLERANZ-ZONE (60%)                                         │
│  Neutrale Varianten. Weder beste noch schlechteste.          │
│  Werden BEHALTEN solange Contracts erfuellt.                 │
│  KEIN Selektionsdruck — pure Drift.                          │
│  Hier passiert die Akkumulation.                             │
│  Hier entsteht die Kreativitaet.                             │
│                                                               │
│  PRUEF-ZONE (30%)                                            │
│  Neue Mutationen und Kreuzungen.                             │
│  Muessen Validator + Prover bestehen.                        │
│  Bestanden → Toleranz-Zone.                                  │
│  Durchgefallen → verworfen.                                  │
│                                                               │
│  [Entfernt werden NUR:]                                      │
│  - Graphen die Contracts verletzen (Prover: DISPROVEN)       │
│  - Graphen die strukturell ungueltig sind (Validator: FAIL)  │
│  - Pool-Ueberlauf: AELTESTE aus Toleranz-Zone entfernen     │
│    (nicht schlechteste — Alter, nicht Fitness)               │
└──────────────────────────────────────────────────────────────┘

Pool-Parameter:
  POOL_SIZE:           1000-10000 Graphen
  ELITE_RATIO:         0.10
  TOLERANCE_RATIO:     0.40
  INCUBATION_RATIO:    0.30
  PROBE_RATIO:         0.20
  MUTATIONS_PER_GEN:   Pool * 0.3 (Toleranz: 30%, Inkubation: 60%)
  CROSSOVER_PER_GEN:   Pool * 0.1 (nur zwischen gesunden Graphen)
  MAX_INCUBATION:      500 Generationen (dann → Archiv)
  MAX_GENERATIONS:     unbegrenzt (bis Abbruchkriterium)
  CONVERGENCE_CHECK:   alle 50 Generationen

  Abbruchkriterien:
  - Fitness-Plateau (keine Verbesserung seit 100 Generationen)
  - Neuartigkeits-Plateau (SGD stagniert)
  - Heilungsrate in Inkubation sinkt auf 0
  - Externe Unterbrechung
  - NICHT: Zeitlimit (Zeit ist irrelevant)
```


**Vergleich: FLUX uebernimmt Biologie INKLUSIVE Fehlertoleranz**

```
                        Biologie              FLUX
────────────────────────────────────────────────────────────────
Mutationsrate           Festgelegt (~10⁻⁸    Steuerbar (0.01 bis 0.5
                        pro Basenpaar)        pro Node pro Generation)

Fehlertoleranz          Ja (Zellen mit        Ja (Graphen mit Contract-
                        DNA-Schaeden leben    Verletzung leben in
                        oft weiter)           Inkubations-Zone weiter)

Heilung durch           Ja (DNA-Reparatur,    Ja (Folge-Mutation kann
weitere Mutation        kompensatorische      Contract wiederherstellen)
                        Mutation)

Fehler als Vorteil      Ja (Sichelzellen →    Ja (Instabiler Sort →
                        Malaria-Resistenz)    5x schnellerer Sort)

Generationszeit         Minuten bis Jahre     Millisekunden bis Sekunden

Kreuzung                Nur innerhalb         Zwischen beliebigen
                        einer Spezies         kompatiblen Graphen

Gerichtete Mutation     Nicht moeglich        Moeglich (LLM + Gegenbeispiele
                        (Lamarck widerlegt)   aus Prover als Reparatur-Hints)

Rueckschritt            Moeglich (Verlust     Erlaubt in Inkubation,
                        von Anpassungen)      Elite bewahrt beste Varianten

Krebs (entarteter       Moeglich, toedlich    Unmoeglich auf Binary-Ebene
Output)                                       (nur bewiesene Graphen kompiliert)

Parallelitaet           Limitiert             Beliebig (1000 Mutationen
                                              parallel bewerten)
```

FLUX uebernimmt das biologische Modell VOLLSTAENDIG:
- Fehlertoleranz wie in der Natur (Inkubation statt Eliminierung)
- Kumulative Mutation wie in der Natur (neutrale Drift + Wucherung)
- Heilung durch den Fehler wie in der Natur (Sichelzell-Prinzip)
- ABER: Krebs auf Binary-Ebene unmoeglich (formaler Beweis als Gate)


### Anforderungstypen fuer Kreativitaet

```
Typ 1: UEBERSETZE (keine Kreativitaet noetig)
  "Sortiere ein Array mit Mergesort"
  → Genau ein bekannter Algorithmus → direkte Synthese

Typ 2: OPTIMIERE (gerichtete Kreativitaet)
  "Sortiere ein Array moeglichst schnell"
  → Viele bekannte + neue Varianten → Pareto-Selektion

Typ 3: ERFINDE (maximale Kreativitaet)
  "Verbessere die Sortierung. Erfinde etwas Neues."
  → Volle explorative Synthese mit Neuartigkeits-Bonus
  → Genetische Evolution ueber Generationen
  → Constraint-Relaxation erlaubt

Typ 4: ENTDECKE (offene Exploration)
  "Finde eine Berechnung die Eigenschaft X hat."
  → Nur Contracts definiert, kein Algorithmus vorgegeben
  → Reine Suche im Graphen-Raum
  → Kann fundamental neue Algorithmen hervorbringen
```


## 10. KI-Generierungspipeline v3 (erweitert)

```
Anforderung
    │
    ├── Typ 1/2: UEBERSETZE / OPTIMIERE
    │   → Direkte Synthese (wenige Varianten)
    │
    └── Typ 3/4: ERFINDE / ENTDECKE
        → Explorative Synthese (Sektion 9)
    │
    ▼
Kandidaten-Pool (1..10000 Graphen)
    │
    ▼
┌─────────────────────────────────────────────────┐
│ SELEKTION (keine Iterations-Begrenzung)          │
│                                                  │
│   Validator:  Struktur + Typen + Effekte         │
│      │                                           │
│      ├── FAIL → verwerfen ODER Feedback ans LLM  │
│      │          LLM erzeugt korrigierten Graph   │
│      │          → zurueck zum Validator          │
│      │                                           │
│      └── PASS ▼                                  │
│                                                  │
│   Contract Prover (KEIN Timeout):                │
│      │                                           │
│      ├── DISPROVEN → verwerfen ODER Feedback     │
│      │                                           │
│      ├── UNDECIDABLE → LLM erzeugt Lean-Beweis  │
│      │                                           │
│      └── ALL PROVEN ▼                            │
│                                                  │
│   Fitness-Bewertung (bei Typ 2/3/4):            │
│      Sandbox-Execution + statische Analyse       │
│      Pareto-Front: Throughput × Speicher ×       │
│                    Latenz × Neuartigkeit          │
│                                                  │
│   Optional: Zurueck zu Phase 1 (Evolution)       │
│      Mutation + Kreuzung → neue Generation       │
│                                                  │
└──────────────────┬──────────────────────────────┘
                   │ Bester Kandidat (oder Pareto-Set)
                   ▼
              Superoptimizer → MLIR → LLVM → Binary
```


## 11. Vergleich v2 → v3

```
Aspekt              v2                         v3
───────────────────────────────────────────────────────────────
Node-Typen          11                         7
Zwischenformat      JSON (menschenlesbar)      Binaer
Variablennamen      Ja                         Nein (Content-Hash)
Fehlerbehandlung    F-Node + Policy            Normale Graph-Pfade
SMT Timeout         5 Sekunden                 Kein Timeout
Unbewiesene Contr.  Runtime-Check (Branch)     Graph UNGUELTIG
Iterationen LLM     Max 3                      Unbegrenzt
Compile-Zeit        ~2 Sekunden                Minuten bis Stunden
Debug-Support       D-Node + Trace             Keiner
Optimierung         LLVM -O3                   Superoptimizer
Module              P-Node (Organisation)      Flacher Graph
Korrektheitsgarantie Teilweise                 Total
Runtime-Checks      0-N pro Binary             EXAKT 0
Kreativitaet        Keine (1:1 Uebersetzung)   Explorative Synthese
Varianten           1 Graph pro Anforderung    50-10000 Kandidaten
Wissensbasis        Keine                      Wachsendes Graph Repository
```


## 12. Konsequenzen

**Was dadurch moeglich wird:**

```
1. JEDES kompilierte Binary ist BEWIESENERMASSEN korrekt
   - Keine Bugs die durch Contracts abgedeckt sind
   - Null Runtime-Overhead fuer Korrektheit
   - Formal verifiziert, nicht nur getestet

2. Maschinencode ist OPTIMAL
   - Superoptimizer findet Sequenzen die kein Mensch findet
   - Besser als handgeschriebener Assembler
   - Pro Plattform individuell optimiert

3. KI kann NEUARTIGE ALGORITHMEN erfinden
   - Nicht limitiert auf bekannte Patterns
   - Explorative Synthese mit genetischer Evolution
   - Constraint-Relaxation oeffnet neue Loesungsraeume
   - Korrektheit ist garantiert — Kreativitaet ist frei

4. Wachsendes Oekosystem eigener Erfindungen
   - Graph Repository als "algorithmische DNA"
   - Jede Synthese erweitert die Wissensbasis
   - Neuartigkeits-Metrik verhindert Stagnation
   - System wird ueber Zeit kreativer

5. Extreme Kompaktheit
   - Kein Boilerplate, kein Framework, kein Overhead
   - Keine menschlichen Abstraktionen, nur Berechnung
```

**Was dadurch verloren geht:**

```
1. Kein Mensch kann den Output lesen oder debuggen
2. Kein Mensch kann den Output modifizieren
3. Compile-Zeiten von Minuten bis Stunden (oder Tagen bei ENTDECKE)
4. Abhaengig von SMT-Solver-Faehigkeiten
5. Erfundene Algorithmen sind nicht erklaerbar
   (sie funktionieren beweisbar, aber niemand versteht warum)
```


## 13. Minimal-Beispiel: Hello World (v3, Typ UEBERSETZE)

```
Kein JSON. Kein Name. Kein Kommentar. Nur Struktur:

T:a1 = { kind: array, elem: u8, len: 12 }
T:a2 = { kind: u64 }
T:a3 = { kind: unit }

R:b1 = { lifetime: static }

C:c1 = { op: CONST_BYTES, val: [72,101,108,108,111,32,87,111,114,108,100,10], out: T:a1, reg: R:b1 }
C:c2 = { op: CONST, val: 1, out: T:a2 }
C:c3 = { op: CONST, val: 12, out: T:a2 }
C:c4 = { op: CONST, val: 0, out: T:a2 }

E:d1 = { op: SYSCALL_WRITE, in: [C:c2, C:c1, C:c3], out: T:a2, eff: [IO] }
  edge success → E:d2
  edge failure → C:c5

C:c5 = { op: CONST, val: 1, out: T:a2 }
E:d3 = { op: SYSCALL_EXIT, in: [C:c5], out: T:a3, eff: [PROC] }

E:d2 = { op: SYSCALL_EXIT, in: [C:c4], out: T:a3, eff: [PROC] }

V:e1 = { target: E:d1, pre: "C:c2.val == 1" }
V:e2 = { target: E:d1, pre: "C:c3.val == len(C:c1.val)" }

K:f1 = { op: SEQ, seq: [E:d1, E:d2] }
  entry: K:f1

Content-Hash: blake3:9f1a...

Anmerkung: Die obige Darstellung ist ein TEXT-RENDERING
fuer dieses Dokument. Der tatsaechliche Graph ist binaer.
Im echten System existiert kein Text.
```
