// Phase 28: F-Node basic test — user-defined pure function
T:i32 = integer { bits: 32, signed: true }

F:double = fn {
    params: [x: T:i32],
    result: T:i32,
    body: [
        C:_two = const { value: 2, type: T:i32 },
        C:_r = mul { inputs: [P:x, C:_two], type: T:i32 }
    ],
    returns: C:_r
}

C:c1 = const { value: 5, type: T:i32 }
C:c2 = call_pure { target: "double", inputs: [C:c1], type: T:i32 }
K:main = seq { steps: [C:c2] }
entry: K:main
