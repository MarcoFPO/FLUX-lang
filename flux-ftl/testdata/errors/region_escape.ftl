// Error test: region escape (6006)
// M:g2 allocates in R:b2 (scoped, depth=1), M:g1 allocates in R:b1 (static, depth=0).
// M:g3 stores a value from M:g2 into M:g1 — data escapes the shorter-lived region.

T:a1 = integer { bits: 32, signed: true }

R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b1 }

// Allocate in static region (long-lived)
M:g1 = alloc { type: T:a1, region: R:b1 }

// Allocate in scoped region (short-lived)
M:g2 = alloc { type: T:a1, region: R:b2 }

C:c_zero = const { value: 0, type: T:a1 }

// Load from scoped allocation
M:g_load = load { source: M:g2, index: C:c_zero, type: T:a1 }

// ERROR: store value from R:b2 (depth=1) into target in R:b1 (depth=0)
// This is a region escape — when R:b2 ends, the data in M:g1 would dangle.
M:g3 = store { target: M:g1, index: C:c_zero, value: M:g2 }

E:d1 = syscall_exit { inputs: [C:c_zero], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
