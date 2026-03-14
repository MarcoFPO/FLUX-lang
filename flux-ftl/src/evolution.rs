// ---------------------------------------------------------------------------
// Phase 12: Pool/Evolution — Genetic algorithm for graph variant evolution
// ---------------------------------------------------------------------------
//
// Implements a genetic algorithm that evolves FTL graph populations:
//   - GraphPool manages a population of compiled graphs
//   - Fitness evaluation combines correctness, size, depth, and cost
//   - Mutation operators: SwapComputeOp, ModifyConstant, InsertNode,
//     RemoveNode, SwapEdges, ModifyContract
//   - Tournament and Pareto selection for multi-objective optimization
//   - Incubation zone for failed graphs that may evolve to correctness
// ---------------------------------------------------------------------------

use std::collections::HashSet;

use rand::prelude::*;
use rand::rngs::StdRng;

use crate::ast::*;
use crate::prover::{ProofResult, ProofStatus};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the evolutionary algorithm.
#[derive(Debug, Clone)]
pub struct EvolutionConfig {
    /// Number of individuals in the main population.
    pub population_size: usize,
    /// Number of elite individuals preserved unchanged per generation.
    pub elite_count: usize,
    /// Probability that a gene is mutated (0.0 - 1.0).
    pub mutation_rate: f64,
    /// Probability that crossover is applied (0.0 - 1.0).
    pub crossover_rate: f64,
    /// Number of individuals competing in tournament selection.
    pub tournament_size: usize,
    /// Maximum number of generations to evolve.
    pub max_generations: u32,
    /// Maximum number of individuals in the incubation zone.
    pub incubation_limit: usize,
    /// Optional seed for the random number generator (determinism).
    pub seed: Option<u64>,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            population_size: 50,
            elite_count: 5,
            mutation_rate: 0.3,
            crossover_rate: 0.5,
            tournament_size: 3,
            max_generations: 100,
            incubation_limit: 20,
            seed: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Fitness
// ---------------------------------------------------------------------------

/// Multi-objective fitness score for an individual.
#[derive(Debug, Clone)]
pub struct FitnessScore {
    /// Fraction of contracts proven correct (0.0 - 1.0).
    pub correctness: f64,
    /// Normalized node count metric (smaller graph = higher value).
    pub node_count: f64,
    /// Normalized depth metric (shallower graph = higher value).
    pub depth: f64,
    /// Normalized estimated execution cost (lower cost = higher value).
    pub estimated_cost: f64,
}

impl FitnessScore {
    /// Weighted aggregate fitness. Correctness dominates at 50%.
    pub fn total(&self) -> f64 {
        self.correctness * 0.5
            + self.node_count * 0.2
            + self.depth * 0.1
            + self.estimated_cost * 0.2
    }
}

// ---------------------------------------------------------------------------
// Individual
// ---------------------------------------------------------------------------

/// A single individual in the evolutionary population.
#[derive(Debug, Clone)]
pub struct Individual {
    /// The FTL program (graph) this individual represents.
    pub program: Program,
    /// Computed fitness score (None if not yet evaluated).
    pub fitness: Option<FitnessScore>,
    /// Proof results from the contract prover (None if not yet evaluated).
    pub proof_status: Option<Vec<ProofResult>>,
    /// Generation in which this individual was created.
    pub generation: u32,
    /// History of mutations applied to produce this individual.
    pub lineage: Vec<String>,
}

impl Individual {
    /// Create a new individual from a program.
    pub fn new(program: Program, generation: u32) -> Self {
        Self {
            program,
            fitness: None,
            proof_status: None,
            generation,
            lineage: Vec::new(),
        }
    }

    /// Whether this individual has been proven fully correct.
    pub fn is_proven(&self) -> bool {
        match &self.proof_status {
            Some(results) if !results.is_empty() => {
                results.iter().all(|r| r.status == ProofStatus::Proven)
            }
            _ => false,
        }
    }

    /// Whether this individual has any disproven contracts.
    pub fn has_failures(&self) -> bool {
        match &self.proof_status {
            Some(results) => results
                .iter()
                .any(|r| r.status == ProofStatus::Disproven || r.status == ProofStatus::Unknown),
            None => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Mutation operators
// ---------------------------------------------------------------------------

/// The available mutation operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationOp {
    /// Swap an arithmetic opcode (e.g. add -> sub).
    SwapComputeOp,
    /// Modify an integer constant by a small delta.
    ModifyConstant,
    /// Insert a new identity C-Node (e.g. add(x, 0)).
    InsertNode,
    /// Remove a C-Node that has exactly one consumer.
    RemoveNode,
    /// Swap two input edges of an Arith C-Node.
    SwapEdges,
    /// Modify a comparison operator or constant in a V-Node formula.
    ModifyContract,
}

impl MutationOp {
    /// All available mutation operators.
    pub const ALL: [MutationOp; 6] = [
        MutationOp::SwapComputeOp,
        MutationOp::ModifyConstant,
        MutationOp::InsertNode,
        MutationOp::RemoveNode,
        MutationOp::SwapEdges,
        MutationOp::ModifyContract,
    ];
}

// ---------------------------------------------------------------------------
// Evolution result types
// ---------------------------------------------------------------------------

/// Result of an evolutionary run.
#[derive(Debug, Clone)]
pub struct EvolutionResult {
    /// The best individual found.
    pub best: Individual,
    /// How many generations were actually run.
    pub generations_run: u32,
    /// Final population statistics.
    pub population_stats: PopulationStats,
}

/// Aggregate statistics for a population.
#[derive(Debug, Clone)]
pub struct PopulationStats {
    /// Average total fitness across the population.
    pub avg_fitness: f64,
    /// Best total fitness in the population.
    pub best_fitness: f64,
    /// Number of fully proven individuals.
    pub proven_count: usize,
    /// Number of individuals in the incubation zone.
    pub incubated_count: usize,
}

// ---------------------------------------------------------------------------
// GraphPool — the evolutionary engine
// ---------------------------------------------------------------------------

/// The main evolutionary pool that manages a population of FTL programs.
pub struct GraphPool {
    /// The active population of individuals.
    pub population: Vec<Individual>,
    /// Incubation zone for failed individuals that may recover.
    pub incubation: Vec<Individual>,
    /// Configuration parameters.
    config: EvolutionConfig,
    /// Current generation counter.
    generation: u32,
    /// The random number generator (seeded for reproducibility).
    rng: StdRng,
}

impl GraphPool {
    /// Create a new empty pool with the given configuration.
    pub fn new(config: EvolutionConfig) -> Self {
        let rng = match config.seed {
            Some(seed) => StdRng::seed_from_u64(seed),
            None => StdRng::from_entropy(),
        };
        Self {
            population: Vec::new(),
            incubation: Vec::new(),
            config,
            generation: 0,
            rng,
        }
    }

    /// Seed the population by cloning and mutating a base program.
    ///
    /// The first individual is always the unmodified base program.
    /// Subsequent individuals are created by applying random mutations.
    pub fn seed_population(&mut self, base: &Program, count: usize) {
        let actual_count = count.min(self.config.population_size);
        self.population.clear();

        // First individual is the original (unmutated)
        self.population
            .push(Individual::new(base.clone(), self.generation));

        // Fill remaining slots with mutated variants
        for _ in 1..actual_count {
            let mutated = self.mutate(base);
            let mut ind = Individual::new(mutated, self.generation);
            ind.lineage.push("seed-mutation".to_string());
            self.population.push(ind);
        }
    }

    /// Evaluate fitness for all individuals that lack a fitness score.
    pub fn evaluate_fitness(&mut self) {
        for individual in &mut self.population {
            if individual.fitness.is_none() {
                individual.fitness = Some(calculate_fitness(individual));
            }
        }
        for individual in &mut self.incubation {
            if individual.fitness.is_none() {
                individual.fitness = Some(calculate_fitness(individual));
            }
        }
    }

    /// Select parent pairs using tournament selection.
    pub fn select_parents(&self) -> Vec<(usize, usize)> {
        let count = self
            .config
            .population_size
            .saturating_sub(self.config.elite_count);
        let pair_count = count.div_ceil(2);
        let mut pairs = Vec::with_capacity(pair_count);

        if self.population.is_empty() {
            return pairs;
        }

        // Rank-based selection for the immutable API
        let mut indices: Vec<usize> = (0..self.population.len()).collect();
        indices.sort_by(|&a, &b| {
            let fa = self.population[a]
                .fitness
                .as_ref()
                .map_or(0.0, |f| f.total());
            let fb = self.population[b]
                .fitness
                .as_ref()
                .map_or(0.0, |f| f.total());
            fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
        });

        for i in 0..pair_count {
            let p1 = indices[i % indices.len()];
            let p2 = indices[(i + 1) % indices.len()];
            pairs.push((p1, p2));
        }

        pairs
    }

    /// Perform tournament selection using the mutable RNG.
    fn tournament_select(&mut self) -> usize {
        if self.population.is_empty() {
            return 0;
        }

        let mut best_idx = self.rng.gen_range(0..self.population.len());
        let mut best_fitness = self.population[best_idx]
            .fitness
            .as_ref()
            .map_or(0.0, |f| f.total());

        for _ in 1..self.config.tournament_size {
            let idx = self.rng.gen_range(0..self.population.len());
            let fitness = self.population[idx]
                .fitness
                .as_ref()
                .map_or(0.0, |f| f.total());
            if fitness > best_fitness {
                best_idx = idx;
                best_fitness = fitness;
            }
        }

        best_idx
    }

    /// Crossover: combine compute nodes from two parent programs.
    ///
    /// Takes compute nodes from both parents, preferring the first parent
    /// for shared IDs and adding unique nodes from both.
    pub fn crossover(&self, parent1: &Program, parent2: &Program) -> Program {
        let mut child = parent1.clone();

        if parent2.computes.is_empty() {
            return child;
        }

        // Collect IDs from parent1
        let p1_ids: HashSet<String> = parent1.computes.iter().map(|c| c.id.0.clone()).collect();

        // For each compute in parent2 that is NOT in parent1, add it
        for compute in &parent2.computes {
            if !p1_ids.contains(&compute.id.0) {
                child.computes.push(compute.clone());
            }
        }

        // For compute nodes present in both, randomly pick from parent2
        // with 50% probability (using a simple hash-based determinism).
        let mut new_computes = Vec::with_capacity(child.computes.len());
        let p2_map: std::collections::HashMap<String, &ComputeDef> = parent2
            .computes
            .iter()
            .map(|c| (c.id.0.clone(), c))
            .collect();

        for c in &child.computes {
            if let Some(p2_compute) = p2_map.get(&c.id.0) {
                // Use a simple hash to decide deterministically
                let hash = c.id.0.len() % 2;
                if hash == 0 {
                    new_computes.push((*p2_compute).clone());
                } else {
                    new_computes.push(c.clone());
                }
            } else {
                new_computes.push(c.clone());
            }
        }
        child.computes = new_computes;

        // Also crossover contracts: take union
        let c1_ids: HashSet<String> = parent1.contracts.iter().map(|c| c.id.0.clone()).collect();
        for contract in &parent2.contracts {
            if !c1_ids.contains(&contract.id.0) {
                child.contracts.push(contract.clone());
            }
        }

        child
    }

    /// Apply a random mutation to a program, returning the mutated copy.
    pub fn mutate(&mut self, program: &Program) -> Program {
        let op_idx = self.rng.gen_range(0..MutationOp::ALL.len());
        let op = MutationOp::ALL[op_idx];
        self.apply_mutation(program, op)
    }

    /// Apply a specific mutation operator to a program.
    pub fn apply_mutation(&mut self, program: &Program, op: MutationOp) -> Program {
        match op {
            MutationOp::SwapComputeOp => self.mutate_swap_compute_op(program),
            MutationOp::ModifyConstant => self.mutate_modify_constant(program),
            MutationOp::InsertNode => self.mutate_insert_node(program),
            MutationOp::RemoveNode => self.mutate_remove_node(program),
            MutationOp::SwapEdges => self.mutate_swap_edges(program),
            MutationOp::ModifyContract => self.mutate_modify_contract(program),
        }
    }

    /// SwapComputeOp: find Arith nodes and swap the opcode.
    fn mutate_swap_compute_op(&mut self, program: &Program) -> Program {
        let mut result = program.clone();

        let arith_indices: Vec<usize> = result
            .computes
            .iter()
            .enumerate()
            .filter(|(_, c)| matches!(&c.op, ComputeOp::Arith { .. }))
            .map(|(i, _)| i)
            .collect();

        if arith_indices.is_empty() {
            return result;
        }

        let target_idx = arith_indices[self.rng.gen_range(0..arith_indices.len())];
        let opcodes = ["add", "sub", "mul", "div", "mod"];

        if let ComputeOp::Arith {
            ref opcode,
            ref inputs,
            ref type_ref,
        } = result.computes[target_idx].op
        {
            let other_ops: Vec<&&str> = opcodes.iter().filter(|o| **o != opcode.as_str()).collect();
            if !other_ops.is_empty() {
                let new_op = other_ops[self.rng.gen_range(0..other_ops.len())];
                result.computes[target_idx].op = ComputeOp::Arith {
                    opcode: (*new_op).to_string(),
                    inputs: inputs.clone(),
                    type_ref: type_ref.clone(),
                };
            }
        }

        result
    }

    /// ModifyConstant: find Const(Integer) nodes and adjust the value.
    fn mutate_modify_constant(&mut self, program: &Program) -> Program {
        let mut result = program.clone();

        let const_indices: Vec<usize> = result
            .computes
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                matches!(
                    &c.op,
                    ComputeOp::Const {
                        value: Literal::Integer { .. },
                        ..
                    }
                )
            })
            .map(|(i, _)| i)
            .collect();

        if const_indices.is_empty() {
            return result;
        }

        let target_idx = const_indices[self.rng.gen_range(0..const_indices.len())];

        if let ComputeOp::Const {
            value: Literal::Integer { value },
            ref type_ref,
            ref region,
        } = result.computes[target_idx].op
        {
            let delta: i64 = self.rng.gen_range(-10..=10);
            let new_value = value.wrapping_add(delta);
            result.computes[target_idx].op = ComputeOp::Const {
                value: Literal::Integer { value: new_value },
                type_ref: type_ref.clone(),
                region: region.clone(),
            };
        }

        result
    }

    /// InsertNode: insert an identity node (add(x, 0)) after a random node.
    fn mutate_insert_node(&mut self, program: &Program) -> Program {
        let mut result = program.clone();

        if result.computes.is_empty() {
            return result;
        }

        // Pick a random existing compute node to wire through
        let source_idx = self.rng.gen_range(0..result.computes.len());
        let source_id = result.computes[source_idx].id.clone();

        // Determine the type_ref of the source node
        let type_ref = get_compute_type_ref(&result.computes[source_idx]);

        // Generate unique IDs for the new nodes
        let zero_id = NodeRef::new(format!("C:evo_zero_{}", self.generation));
        let new_id = NodeRef::new(format!("C:evo_ins_{}", self.generation));

        // Add a zero constant
        result.computes.push(ComputeDef {
            id: zero_id.clone(),
            op: ComputeOp::Const {
                value: Literal::Integer { value: 0 },
                type_ref: type_ref.clone(),
                region: None,
            },
        });

        // Add an add(source, 0) node
        result.computes.push(ComputeDef {
            id: new_id,
            op: ComputeOp::Arith {
                opcode: "add".to_string(),
                inputs: vec![source_id, zero_id],
                type_ref,
            },
        });

        result
    }

    /// RemoveNode: remove a compute node that is referenced by at most
    /// one other node, rewiring the consumer to use the removed node's input.
    fn mutate_remove_node(&mut self, program: &Program) -> Program {
        let mut result = program.clone();

        // Find Arith nodes with at least one input that are not the entry
        let removable: Vec<usize> = result
            .computes
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                matches!(&c.op, ComputeOp::Arith { inputs, .. } if !inputs.is_empty())
                    && c.id != result.entry
            })
            .map(|(i, _)| i)
            .collect();

        if removable.is_empty() {
            return result;
        }

        let target_idx = removable[self.rng.gen_range(0..removable.len())];
        let target_id = result.computes[target_idx].id.0.clone();

        // Get the first input of the removed node (to rewire consumers)
        let replacement_id =
            if let ComputeOp::Arith { ref inputs, .. } = result.computes[target_idx].op {
                inputs[0].0.clone()
            } else {
                return result;
            };

        // Count references to target_id from other compute nodes
        let ref_count = result
            .computes
            .iter()
            .filter(|c| c.id.0 != target_id)
            .filter(|c| compute_references_id(c, &target_id))
            .count();

        // Only remove if referenced by at most one other node
        if ref_count > 1 {
            return result;
        }

        // Rewire: replace all references to target_id with replacement_id
        for c in &mut result.computes {
            replace_node_ref_in_compute(c, &target_id, &replacement_id);
        }

        // Also rewire control flow references
        for k in &mut result.controls {
            replace_node_ref_in_control(k, &target_id, &replacement_id);
        }

        // Update entry if needed
        if result.entry.0 == target_id {
            result.entry = NodeRef::new(replacement_id);
        }

        // Remove the node
        result.computes.retain(|c| c.id.0 != target_id);

        result
    }

    /// SwapEdges: swap two inputs of an Arith node.
    fn mutate_swap_edges(&mut self, program: &Program) -> Program {
        let mut result = program.clone();

        let multi_input_indices: Vec<usize> = result
            .computes
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                matches!(&c.op, ComputeOp::Arith { inputs, .. } if inputs.len() >= 2)
            })
            .map(|(i, _)| i)
            .collect();

        if multi_input_indices.is_empty() {
            return result;
        }

        let target_idx = multi_input_indices[self.rng.gen_range(0..multi_input_indices.len())];

        if let ComputeOp::Arith {
            ref opcode,
            ref inputs,
            ref type_ref,
        } = result.computes[target_idx].op
        {
            let mut new_inputs = inputs.clone();
            let i = self.rng.gen_range(0..new_inputs.len());
            let j = self.rng.gen_range(0..new_inputs.len());
            new_inputs.swap(i, j);

            result.computes[target_idx].op = ComputeOp::Arith {
                opcode: opcode.clone(),
                inputs: new_inputs,
                type_ref: type_ref.clone(),
            };
        }

        result
    }

    /// ModifyContract: change a comparison operator or constant in a contract.
    fn mutate_modify_contract(&mut self, program: &Program) -> Program {
        let mut result = program.clone();

        if result.contracts.is_empty() {
            return result;
        }

        let target_idx = self.rng.gen_range(0..result.contracts.len());

        if result.contracts[target_idx].clauses.is_empty() {
            return result;
        }

        let clause_idx = self
            .rng
            .gen_range(0..result.contracts[target_idx].clauses.len());

        let new_clause = {
            let clause = &result.contracts[target_idx].clauses[clause_idx];
            let formula = match clause {
                ContractClause::Pre { formula } => formula,
                ContractClause::Post { formula } => formula,
                ContractClause::Invariant { formula } => formula,
                ContractClause::Assume { formula } => formula,
            };

            let mutated_formula = self.mutate_formula(formula);

            match clause {
                ContractClause::Pre { .. } => ContractClause::Pre {
                    formula: mutated_formula,
                },
                ContractClause::Post { .. } => ContractClause::Post {
                    formula: mutated_formula,
                },
                ContractClause::Invariant { .. } => ContractClause::Invariant {
                    formula: mutated_formula,
                },
                ContractClause::Assume { .. } => ContractClause::Assume {
                    formula: mutated_formula,
                },
            }
        };

        result.contracts[target_idx].clauses[clause_idx] = new_clause;
        result
    }

    /// Mutate a formula by changing comparison operators or integer literals.
    fn mutate_formula(&mut self, formula: &Formula) -> Formula {
        match formula {
            Formula::Comparison { left, op: _, right } => {
                let all_ops = [
                    CmpOp::Eq,
                    CmpOp::Neq,
                    CmpOp::Lt,
                    CmpOp::Lte,
                    CmpOp::Gt,
                    CmpOp::Gte,
                ];
                let new_op = all_ops[self.rng.gen_range(0..all_ops.len())].clone();
                let new_right = self.mutate_expr(right);

                Formula::Comparison {
                    left: left.clone(),
                    op: new_op,
                    right: new_right,
                }
            }
            Formula::And { left, right } => {
                if self.rng.gen_bool(0.5) {
                    Formula::And {
                        left: Box::new(self.mutate_formula(left)),
                        right: right.clone(),
                    }
                } else {
                    Formula::And {
                        left: left.clone(),
                        right: Box::new(self.mutate_formula(right)),
                    }
                }
            }
            Formula::Or { left, right } => {
                if self.rng.gen_bool(0.5) {
                    Formula::Or {
                        left: Box::new(self.mutate_formula(left)),
                        right: right.clone(),
                    }
                } else {
                    Formula::Or {
                        left: left.clone(),
                        right: Box::new(self.mutate_formula(right)),
                    }
                }
            }
            Formula::Not { inner } => Formula::Not {
                inner: Box::new(self.mutate_formula(inner)),
            },
            other => other.clone(),
        }
    }

    /// Mutate an expression by adjusting integer literals.
    fn mutate_expr(&mut self, expr: &Expr) -> Expr {
        match expr {
            Expr::IntLit { value } => {
                let delta: i64 = self.rng.gen_range(-5..=5);
                Expr::IntLit {
                    value: value.wrapping_add(delta),
                }
            }
            other => other.clone(),
        }
    }

    /// Run one full generation of evolution.
    pub fn evolve_generation(&mut self) {
        self.evaluate_fitness();

        if self.population.is_empty() {
            return;
        }

        // Sort by fitness (descending)
        self.population.sort_by(|a, b| {
            let fa = a.fitness.as_ref().map_or(0.0, |f| f.total());
            let fb = b.fitness.as_ref().map_or(0.0, |f| f.total());
            fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Preserve elites
        let elite_count = self.config.elite_count.min(self.population.len());
        let mut next_gen: Vec<Individual> = self.population[..elite_count].to_vec();

        // Move failed individuals to incubation
        let (viable, failed): (Vec<_>, Vec<_>) = self
            .population
            .iter()
            .skip(elite_count)
            .cloned()
            .partition(|ind| !ind.has_failures());

        for failed_ind in failed {
            if self.incubation.len() < self.config.incubation_limit {
                self.incubation.push(failed_ind);
            }
        }

        // Temporarily put viable back (for tournament selection)
        self.population = next_gen.iter().chain(viable.iter()).cloned().collect();

        // Fill remaining slots
        let target_size = self.config.population_size;
        while next_gen.len() < target_size && !self.population.is_empty() {
            let p1_idx = self.tournament_select();
            let p2_idx = self.tournament_select();

            let p1 = self.population[p1_idx].program.clone();
            let p2 = self.population[p2_idx].program.clone();

            let mut child_program = if self.rng.gen_bool(self.config.crossover_rate) {
                self.crossover(&p1, &p2)
            } else {
                p1
            };

            if self.rng.gen_bool(self.config.mutation_rate) {
                child_program = self.mutate(&child_program);
            }

            let mut child = Individual::new(child_program, self.generation + 1);
            child.lineage.push(format!("gen-{}", self.generation));
            next_gen.push(child);
        }

        self.generation += 1;
        self.population = next_gen;
    }

    /// Run the evolutionary algorithm for the given number of generations.
    pub fn run(&mut self, generations: u32) -> EvolutionResult {
        let actual_gens = generations.min(self.config.max_generations);

        for _ in 0..actual_gens {
            self.evolve_generation();
            self.promote_from_incubation();
        }

        self.evaluate_fitness();

        let best = self.best().cloned().unwrap_or_else(|| {
            Individual::new(
                Program {
                    types: vec![],
                    regions: vec![],
                    computes: vec![],
                    effects: vec![],
                    controls: vec![],
                    contracts: vec![],
                    memories: vec![],
                    externs: vec![],
                    entry: NodeRef::new("K:entry"),
                },
                0,
            )
        });

        EvolutionResult {
            best,
            generations_run: actual_gens,
            population_stats: self.compute_stats(),
        }
    }

    /// Return the best individual in the population (by total fitness).
    pub fn best(&self) -> Option<&Individual> {
        self.population.iter().max_by(|a, b| {
            let fa = a.fitness.as_ref().map_or(0.0, |f| f.total());
            let fb = b.fitness.as_ref().map_or(0.0, |f| f.total());
            fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// Try to promote individuals from incubation back to the population.
    ///
    /// Incubated individuals are re-evaluated; those with improved fitness
    /// are moved back to the main population. Remaining individuals are
    /// mutated further to give them another chance.
    pub fn promote_from_incubation(&mut self) {
        if self.incubation.is_empty() {
            return;
        }

        // Re-evaluate incubation fitness
        for ind in &mut self.incubation {
            ind.fitness = Some(calculate_fitness(ind));
        }

        // Mutate incubated individuals
        let mut mutated_incubation = Vec::new();
        for ind in &self.incubation {
            let mutated_program = self.mutate_without_rng(&ind.program, ind.generation);
            let mut new_ind = Individual::new(mutated_program, self.generation);
            new_ind.lineage = ind.lineage.clone();
            new_ind.lineage.push("incubation-mutation".to_string());
            new_ind.fitness = Some(calculate_fitness(&new_ind));
            mutated_incubation.push(new_ind);
        }

        // Promote those with good correctness (> 0.5)
        let mut remaining = Vec::new();
        for ind in mutated_incubation {
            let correctness = ind.fitness.as_ref().map_or(0.0, |f| f.correctness);
            if correctness > 0.5 && self.population.len() < self.config.population_size {
                self.population.push(ind);
            } else {
                remaining.push(ind);
            }
        }

        // Trim incubation to limit
        remaining.truncate(self.config.incubation_limit);
        self.incubation = remaining;
    }

    /// Mutate without needing &mut self (used during incubation promotion).
    fn mutate_without_rng(&self, program: &Program, generation: u32) -> Program {
        let mut result = program.clone();

        // Simple constant modification based on generation number
        let const_indices: Vec<usize> = result
            .computes
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                matches!(
                    &c.op,
                    ComputeOp::Const {
                        value: Literal::Integer { .. },
                        ..
                    }
                )
            })
            .map(|(i, _)| i)
            .collect();

        if !const_indices.is_empty() {
            let target_idx = const_indices[generation as usize % const_indices.len()];
            if let ComputeOp::Const {
                value: Literal::Integer { value },
                ref type_ref,
                ref region,
            } = result.computes[target_idx].op
            {
                let delta = ((generation as i64 % 7) - 3).clamp(-5, 5);
                result.computes[target_idx].op = ComputeOp::Const {
                    value: Literal::Integer {
                        value: value.wrapping_add(delta),
                    },
                    type_ref: type_ref.clone(),
                    region: region.clone(),
                };
            }
        }

        result
    }

    /// Compute aggregate population statistics.
    fn compute_stats(&self) -> PopulationStats {
        let fitnesses: Vec<f64> = self
            .population
            .iter()
            .filter_map(|ind| ind.fitness.as_ref().map(|f| f.total()))
            .collect();

        let avg_fitness = if fitnesses.is_empty() {
            0.0
        } else {
            fitnesses.iter().sum::<f64>() / fitnesses.len() as f64
        };

        let best_fitness = fitnesses.iter().cloned().fold(0.0_f64, f64::max);

        let proven_count = self
            .population
            .iter()
            .filter(|ind| ind.is_proven())
            .count();

        PopulationStats {
            avg_fitness,
            best_fitness,
            proven_count,
            incubated_count: self.incubation.len(),
        }
    }

    /// Access the current population.
    pub fn population(&self) -> &[Individual] {
        &self.population
    }

    /// Access the current population mutably.
    pub fn population_mut(&mut self) -> &mut Vec<Individual> {
        &mut self.population
    }

    /// Access the incubation zone.
    pub fn incubation(&self) -> &[Individual] {
        &self.incubation
    }

    /// Access the incubation zone mutably.
    pub fn incubation_mut(&mut self) -> &mut Vec<Individual> {
        &mut self.incubation
    }

    /// Get the current generation number.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Get the configuration.
    pub fn config(&self) -> &EvolutionConfig {
        &self.config
    }
}

// ---------------------------------------------------------------------------
// Fitness calculation
// ---------------------------------------------------------------------------

/// Calculate the fitness score for an individual.
pub fn calculate_fitness(individual: &Individual) -> FitnessScore {
    let correctness = match &individual.proof_status {
        Some(results) if !results.is_empty() => {
            let proven = results
                .iter()
                .filter(|r| r.status == ProofStatus::Proven)
                .count();
            let total = results.len();
            proven as f64 / total as f64
        }
        _ => 0.0,
    };

    let node_count = count_nodes(&individual.program);
    let depth = calculate_depth(&individual.program);
    let cost = estimate_cost(&individual.program);

    FitnessScore {
        correctness,
        node_count: 1.0 / (1.0 + node_count as f64),
        depth: 1.0 / (1.0 + depth as f64),
        estimated_cost: 1.0 / (1.0 + cost),
    }
}

/// Count the total number of nodes in a program.
pub fn count_nodes(program: &Program) -> usize {
    program.types.len()
        + program.regions.len()
        + program.computes.len()
        + program.effects.len()
        + program.controls.len()
        + program.contracts.len()
        + program.memories.len()
        + program.externs.len()
}

/// Calculate the maximum depth of the compute graph.
///
/// Depth is measured as the longest chain of compute node dependencies.
pub fn calculate_depth(program: &Program) -> usize {
    if program.computes.is_empty() {
        return 0;
    }

    let mut depths: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for c in &program.computes {
        let input_ids = match &c.op {
            ComputeOp::Const { .. } | ComputeOp::ConstBytes { .. } => vec![],
            ComputeOp::Arith { inputs, .. }
            | ComputeOp::CallPure { inputs, .. }
            | ComputeOp::Generic { inputs, .. } => inputs.iter().map(|i| i.0.clone()).collect(),
            ComputeOp::AtomicLoad { source, .. } => vec![source.0.clone()],
            ComputeOp::AtomicStore { target, value, .. } => {
                vec![target.0.clone(), value.0.clone()]
            }
            ComputeOp::AtomicCas {
                target,
                expected,
                desired,
                ..
            } => vec![
                target.0.clone(),
                expected.0.clone(),
                desired.0.clone(),
            ],
        };

        let max_input_depth = input_ids
            .iter()
            .filter_map(|id| depths.get(id))
            .max()
            .copied()
            .unwrap_or(0);

        depths.insert(c.id.0.clone(), max_input_depth + 1);
    }

    depths.values().max().copied().unwrap_or(0)
}

/// Estimate the execution cost of a program.
///
/// Simple heuristic: Arith ops cost 1, effects cost 10, memory ops cost 5.
pub fn estimate_cost(program: &Program) -> f64 {
    let compute_cost: f64 = program
        .computes
        .iter()
        .map(|c| match &c.op {
            ComputeOp::Const { .. } | ComputeOp::ConstBytes { .. } => 0.5,
            ComputeOp::Arith { .. } => 1.0,
            ComputeOp::CallPure { .. } => 3.0,
            ComputeOp::Generic { .. } => 2.0,
            ComputeOp::AtomicLoad { .. } => 5.0,
            ComputeOp::AtomicStore { .. } => 5.0,
            ComputeOp::AtomicCas { .. } => 10.0,
        })
        .sum();

    let effect_cost: f64 = program.effects.len() as f64 * 10.0;
    let memory_cost: f64 = program.memories.len() as f64 * 5.0;

    compute_cost + effect_cost + memory_cost
}

// ---------------------------------------------------------------------------
// Pareto dominance
// ---------------------------------------------------------------------------

/// Check if fitness score `a` Pareto-dominates fitness score `b`.
///
/// `a` dominates `b` if `a` is at least as good in all objectives
/// and strictly better in at least one.
pub fn pareto_dominates(a: &FitnessScore, b: &FitnessScore) -> bool {
    let dims_a = [a.correctness, a.node_count, a.depth, a.estimated_cost];
    let dims_b = [b.correctness, b.node_count, b.depth, b.estimated_cost];

    let all_geq = dims_a.iter().zip(&dims_b).all(|(x, y)| x >= y);
    let any_gt = dims_a.iter().zip(&dims_b).any(|(x, y)| x > y);

    all_geq && any_gt
}

/// Compute the Pareto front: indices of non-dominated individuals.
pub fn pareto_front(population: &[Individual]) -> Vec<usize> {
    let mut front = Vec::new();

    for (i, ind_i) in population.iter().enumerate() {
        let fitness_i = match &ind_i.fitness {
            Some(f) => f,
            None => continue,
        };

        let is_dominated = population.iter().enumerate().any(|(j, ind_j)| {
            if i == j {
                return false;
            }
            match &ind_j.fitness {
                Some(fitness_j) => pareto_dominates(fitness_j, fitness_i),
                None => false,
            }
        });

        if !is_dominated {
            front.push(i);
        }
    }

    front
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the type_ref from a compute operation.
fn get_compute_type_ref(compute: &ComputeDef) -> TypeRef {
    match &compute.op {
        ComputeOp::Const { type_ref, .. }
        | ComputeOp::ConstBytes { type_ref, .. }
        | ComputeOp::Arith { type_ref, .. }
        | ComputeOp::CallPure { type_ref, .. }
        | ComputeOp::Generic { type_ref, .. }
        | ComputeOp::AtomicLoad { type_ref, .. } => type_ref.clone(),
        ComputeOp::AtomicStore { .. } | ComputeOp::AtomicCas { .. } => TypeRef::Builtin {
            name: "unit".to_string(),
        },
    }
}

/// Check if a compute node references a given ID in its inputs.
fn compute_references_id(compute: &ComputeDef, id: &str) -> bool {
    match &compute.op {
        ComputeOp::Const { .. } | ComputeOp::ConstBytes { .. } => false,
        ComputeOp::Arith { inputs, .. }
        | ComputeOp::CallPure { inputs, .. }
        | ComputeOp::Generic { inputs, .. } => inputs.iter().any(|i| i.0 == id),
        ComputeOp::AtomicLoad { source, .. } => source.0 == id,
        ComputeOp::AtomicStore { target, value, .. } => target.0 == id || value.0 == id,
        ComputeOp::AtomicCas {
            target,
            expected,
            desired,
            ..
        } => target.0 == id || expected.0 == id || desired.0 == id,
    }
}

/// Replace references to `old_id` with `new_id` inside a compute node.
fn replace_node_ref_in_compute(compute: &mut ComputeDef, old_id: &str, new_id: &str) {
    match &mut compute.op {
        ComputeOp::Arith { inputs, .. }
        | ComputeOp::CallPure { inputs, .. }
        | ComputeOp::Generic { inputs, .. } => {
            for input in inputs.iter_mut() {
                if input.0 == old_id {
                    *input = NodeRef::new(new_id);
                }
            }
        }
        ComputeOp::AtomicLoad { source, .. } => {
            if source.0 == old_id {
                *source = NodeRef::new(new_id);
            }
        }
        ComputeOp::AtomicStore { target, value, .. } => {
            if target.0 == old_id {
                *target = NodeRef::new(new_id);
            }
            if value.0 == old_id {
                *value = NodeRef::new(new_id);
            }
        }
        ComputeOp::AtomicCas {
            target,
            expected,
            desired,
            ..
        } => {
            if target.0 == old_id {
                *target = NodeRef::new(new_id);
            }
            if expected.0 == old_id {
                *expected = NodeRef::new(new_id);
            }
            if desired.0 == old_id {
                *desired = NodeRef::new(new_id);
            }
        }
        ComputeOp::Const { .. } | ComputeOp::ConstBytes { .. } => {}
    }
}

/// Replace references to `old_id` with `new_id` inside a control node.
fn replace_node_ref_in_control(control: &mut ControlDef, old_id: &str, new_id: &str) {
    match &mut control.op {
        ControlOp::Seq { steps } => {
            for step in steps.iter_mut() {
                if step.0 == old_id {
                    *step = NodeRef::new(new_id);
                }
            }
        }
        ControlOp::Branch {
            condition,
            true_branch,
            false_branch,
        } => {
            if condition.0 == old_id {
                *condition = NodeRef::new(new_id);
            }
            if true_branch.0 == old_id {
                *true_branch = NodeRef::new(new_id);
            }
            if false_branch.0 == old_id {
                *false_branch = NodeRef::new(new_id);
            }
        }
        ControlOp::Loop {
            condition,
            body,
            state,
            ..
        } => {
            if condition.0 == old_id {
                *condition = NodeRef::new(new_id);
            }
            if body.0 == old_id {
                *body = NodeRef::new(new_id);
            }
            if state.0 == old_id {
                *state = NodeRef::new(new_id);
            }
        }
        ControlOp::Par { branches, .. } => {
            for branch in branches.iter_mut() {
                if branch.0 == old_id {
                    *branch = NodeRef::new(new_id);
                }
            }
        }
    }
}
