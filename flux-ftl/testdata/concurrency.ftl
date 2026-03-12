// Concurrency — K:Par with atomic operations and BARRIER sync
// Two parallel branches accessing shared state via atomic ops

// === Typen ===
T:a1 = integer { bits: 64, signed: false }
T:a2 = integer { bits: 64, signed: true }
T:a3 = integer { bits: 32, signed: true }
T:a4 = unit
T:a5 = boolean
T:a6 = array { element: T:a2, max_length: 1024 }

// === Regionen ===
R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b1 }

// === Shared Memory ===
M:g1 = alloc { type: T:a1, region: R:b2 }
M:g2 = alloc { type: T:a6, region: R:b2 }

// === Constants ===
C:c0 = const { value: 0, type: T:a1 }
C:c1 = const { value: 1, type: T:a1 }
C:c2 = const { value: 2, type: T:a1 }
C:c10 = const { value: 10, type: T:a1 }
C:c42 = const { value: 42, type: T:a2 }
C:c99 = const { value: 99, type: T:a2 }
C:c_true = const { value: 1, type: T:a5 }
C:c_exit0 = const { value: 0, type: T:a3 }
C:c_exit1 = const { value: 1, type: T:a3 }

// === Initial store: shared counter = 0 ===
C:s_init = atomic_store { target: M:g1, value: C:c0, order: SEQ_CST }

// === Branch 1: Producer — increment counter, write to array ===

// Load current counter
C:s1_load = atomic_load { source: M:g1, order: ACQUIRE, type: T:a1 }

// Compute new value
C:c_inc1 = add { inputs: [C:s1_load, C:c1], type: T:a1 }

// Store incremented counter
C:s1_store = atomic_store { target: M:g1, value: C:c_inc1, order: RELEASE }

// Write data to array at index 0
M:g3 = store { target: M:g2, index: C:c0, value: C:c42 }

// Producer sequence
K:f_prod = seq { steps: [C:s1_store, M:g3] }

// === Branch 2: Consumer — spin until counter > 0, then read ===

// Load counter atomically
C:s2_load = atomic_load { source: M:g1, order: ACQUIRE, type: T:a1 }

// Check if counter > 0
C:c_ready = gt { inputs: [C:s2_load, C:c0], type: T:a5 }

// Read from array at index 0
M:g4 = load { source: M:g2, index: C:c0, type: T:a2 }

// CAS: try to set counter from current to 0 (consume token)
C:s2_cas = atomic_cas { target: M:g1, expected: C:s2_load, desired: C:c0, order: SEQ_CST, success: K:f_cas_ok, failure: K:f_cas_retry }

// CAS success: read the data
K:f_cas_ok = seq { steps: [M:g4] }

// CAS failure: retry the load
K:f_cas_retry = seq { steps: [C:s2_load] }

// Consumer: branch on readiness, then CAS
K:f_cons_body = branch { condition: C:c_ready, true: K:f_cas_attempt, false: K:f_cons_spin }
K:f_cas_attempt = seq { steps: [C:s2_cas] }
K:f_cons_spin = seq { steps: [C:s2_load] }

// Consumer loop: spin until ready
K:f_cons = loop { condition: C:c_ready, body: K:f_cons_body, state: C:s2_load, state_type: T:a1 }

// === Parallel execution with BARRIER sync ===
K:f_par = par { branches: [K:f_prod, K:f_cons], sync: BARRIER, memory_order: ACQUIRE_RELEASE }

// === Contracts ===

// Data-race freedom: all shared accesses are atomic
V:e1 = contract { target: K:f_par, invariant: forall (b1, b2) in branches: shared_mnodes(b1, b2) == {} OR all_accesses_atomic(b1, b2) }

// Counter never exceeds 10
V:e2 = contract { target: K:f_par, invariant: C:s1_load.val <= 10 }

// CAS expected value is consistent with loaded value
V:e3 = contract { target: C:s2_cas, pre: C:s2_load.val >= 0 }

// === Cleanup and exit ===
E:d_exit0 = syscall_exit { inputs: [C:c_exit0], type: T:a4, effects: [PROC] }
E:d_exit1 = syscall_exit { inputs: [C:c_exit1], type: T:a4, effects: [PROC] }

// === Main sequence ===
K:f_main = seq { steps: [C:s_init, K:f_par, E:d_exit0] }
entry: K:f_main
