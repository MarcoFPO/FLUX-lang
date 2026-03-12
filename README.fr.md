<p align="center">
  <img src="assets/logo.gif" alt="FLUX Validator Logo" width="400">
</p>

<p align="center">
  <a href="README.md">DE</a> |
  <a href="README.en.md">EN</a> |
  <strong>FR</strong> |
  <a href="README.es.md">ES</a> |
  <a href="README.ja.md">JA</a> |
  <a href="README.zh.md">ZH</a>
</p>

# FLUX — Substrat de Calcul Natif pour l'IA

**FLUX** est une architecture d'exécution dans laquelle les systèmes d'IA (LLMs) génèrent des graphes de calcul en FTL (FLUX Text Language), qui sont formellement vérifiés et compilés en code machine optimal.

**Le LLM génère du texte FTL. Le système compile en binaire. Formellement vérifié. Optimal.**

## Axiomes de Conception

```
1. Le temps de compilation est sans importance → Vérification exhaustive, superoptimisation
2. La lisibilité humaine est sans importance   → Le LLM travaille avec FTL (texte structuré),
                                                  le système compile en binaire
3. Les compensations humaines                  → Pas de debug, pas de gestion d'exceptions,
   ne sont pas nécessaires                       pas de programmation défensive
4. La performance de génération de code        → Itérations LLM illimitées,
   est secondaire                                profondeur d'analyse illimitée
5. La créativité est souhaitée                 → L'IA doit INVENTER des solutions nouvelles,
                                                  pas seulement reproduire des schémas connus
6. Pragmatisme dans la vérification            → Stratégie de prouveurs échelonnée avec timeouts,
                                                  indécidable → escalade, pas de boucle infinie
```

## Architecture

```
Exigence (langage naturel, hors périmètre)
    │
LLM (le programmeur — remplace l'humain)
    │  FTL (FLUX Text Language) — texte structuré
    ▼
Système FLUX
    ├─ Compilateur FTL (Texte → Binaire + hachages BLAKE3)
    ├─ Validateur (Structure + Types + Effets + Régions)
    │    ÉCHEC → Retour JSON au LLM (avec suggestions)
    ├─ Prouveur de Contrats (échelonné : Z3 60s → BMC 5m → Lean)
    │    RÉFUTÉ → Contre-exemple au LLM
    │    INDÉCIDABLE → Indice au LLM ou incubation
    ├─ Pool / Évolution (pour INVENTER/DÉCOUVRIR)
    │    Retour de fitness au LLM (métriques relatives)
    ├─ Superoptimiseur (3 niveaux : LLVM + MLIR + STOKE)
    │    Chemins chauds optimaux, reste qualité LLVM -O3
    └─ MLIR → LLVM → code machine natif
    │
┌───┴────┬──────────┬──────────┐
ARM64   x86-64    RISC-V     WASM
```

## Types de Nœuds

| Nœud | Fonction |
|------|----------|
| **C-Node** | Calcul pur (ADD, MUL, CONST, ...) |
| **E-Node** | Effet de bord avec exactement 2 sorties (succès + échec) |
| **K-Node** | Flux de contrôle : Seq, Par, Branch, Loop |
| **V-Node** | Contrat (SMT-LIB2) — DOIT être prouvé pour la compilation |
| **T-Node** | Type : Integer, Float, Struct, Array, Variant, Fn, Opaque |
| **M-Node** | Opération mémoire (liée à une région) |
| **R-Node** | Durée de vie mémoire (arène) |


## Principes Fondamentaux

**Le LLM comme Programmeur :** Le LLM remplace le programmeur humain. Il fournit du texte FTL (pas de binaire, pas de hachages). Le système compile le FTL en graphes binaires, calcule les hachages BLAKE3 et renvoie un retour JSON.

**Correction Totale :** Chaque binaire compilé est formellement vérifié. Zéro vérification à l'exécution. Les contrats sont prouvés par une stratégie de prouveurs échelonnée (Z3 → BMC → Lean).

**Synthèse Exploratoire :** L'IA ne génère pas un graphe, mais des centaines. La correction est le filtre, la créativité est le générateur. L'algorithme génétique (AG) est le moteur principal d'innovation ; le LLM fournit l'initialisation et les réparations ciblées.

**Superoptimisation :** 3 niveaux (LLVM -O3 → niveau MLIR → STOKE). Les chemins chauds sont meilleurs que l'assembleur écrit à la main. Réaliste : 5-20% d'amélioration globale par rapport au pur LLVM -O3.

**Adressage par Contenu :** Pas de noms de variables. Identité = hachage BLAKE3 du contenu (calculé par le système). Même calcul = même hachage = déduplication automatique.

**Modèle de Mutation Biologique :** Les graphes défectueux sont isolés dans une zone d'incubation pour un développement ultérieur. Une mutation sur une mutation peut transformer quelque chose de « mauvais » en quelque chose de « spécial ». Seul le binaire final doit être prouvé correct — le chemin peut passer par des erreurs.

## Documentation

- **[Spécification FLUX v3](docs/FLUX-v3-SPEC.md)** — Spécification actuelle (18 sections)
- **[Spécification FLUX v2](docs/FLUX-v2-SPEC.md)** — Version précédente (avec concessions humaines)
- **[Analyse d'Experts](docs/ANALYSIS.md)** — Évaluation par 3 agents spécialisés (Round 2)
- **[Simulation Hello World](docs/SIMULATION-hello-world.md)** — Pipeline de l'exigence au code machine
- **[Simulation Snake Game](docs/SIMULATION-snake-game.md)** — Exemple complexe avec son

## Exemples

- [`examples/hello-world.flux.json`](examples/hello-world.flux.json) — Hello World (format JSON v2)
- [`examples/snake-game.flux.json`](examples/snake-game.flux.json) — Snake Game (format JSON v2)

*Note : v3 utilise FTL (FLUX Text Language) au lieu de JSON. Les exemples montrent le format v2.*

## Types d'Exigences

```
TRADUIRE     "Trier avec mergesort"                → Synthèse directe (1 graphe)
OPTIMISER    "Trier le plus vite possible"          → Sélection Pareto (nombreuses variantes)
INVENTER     "Améliorer sort(), inventer du nouveau"→ Synthèse exploratoire + évolution
DÉCOUVRIR    "Trouver un calcul avec propriété X"   → Recherche ouverte dans l'espace des graphes
```


## Licence

MIT

## Remerciements
- Bea pour le logo
- Gerd pour l'inspiration
- Michi pour les commentaires
