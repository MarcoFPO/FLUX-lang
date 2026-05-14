// Recursive F-Node test: factorial
// factorial(5) = 120
T:i32 = integer { bits: 32, signed: true }
T:bool = boolean

F:factorial = fn {
    params: [n: T:i32],
    result: T:i32,
    body: [
        C:_zero = const { value: 0, type: T:i32 },
        C:_one = const { value: 1, type: T:i32 },
        C:_is_zero = eq { inputs: [P:n, C:_zero], type: T:bool },
        C:_n_minus_1 = sub { inputs: [P:n, C:_one], type: T:i32 },
        C:_rec = call_pure { target: "factorial", inputs: [C:_n_minus_1], type: T:i32 },
        C:_result = mul { inputs: [P:n, C:_rec], type: T:i32 },
        C:_final = select { inputs: [C:_is_zero, C:_one, C:_result], type: T:i32 }
    ],
    returns: C:_final
}

C:c1 = const { value: 5, type: T:i32 }
C:c2 = call_pure { target: "factorial", inputs: [C:c1], type: T:i32 }
K:main = seq { steps: [C:c2] }
entry: K:main
