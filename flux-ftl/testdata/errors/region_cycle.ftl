// Error test: circular region hierarchy (6002)
// R:b2 and R:b3 form a cycle: b2 -> b3 -> b2

T:a1 = integer { bits: 32, signed: true }

R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b3 }
R:b3 = region { lifetime: scoped, parent: R:b2 }

C:c1 = const { value: 0, type: T:a1 }
E:d1 = syscall_exit { inputs: [C:c1], type: T:a1, effects: [PROC] }
K:f1 = seq { steps: [E:d1] }
entry: K:f1
