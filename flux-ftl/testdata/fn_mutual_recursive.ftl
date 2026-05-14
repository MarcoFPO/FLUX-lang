// Mutual recursion test: is_even / is_odd
// is_even(4) = true, is_odd(4) = false
T:i32 = integer { bits: 32, signed: true }
T:bool = boolean

// is_even(n) = if n == 0 then true else is_odd(n - 1)
F:is_even = fn {
    params: [n: T:i32],
    result: T:bool,
    body: [
        C:_zero = const { value: 0, type: T:i32 },
        C:_one = const { value: 1, type: T:i32 },
        C:_true = const { value: 1, type: T:bool },
        C:_is_zero = eq { inputs: [P:n, C:_zero], type: T:bool },
        C:_n_minus_1 = sub { inputs: [P:n, C:_one], type: T:i32 },
        C:_odd_result = call_pure { target: "is_odd", inputs: [C:_n_minus_1], type: T:bool },
        C:_final = select { inputs: [C:_is_zero, C:_true, C:_odd_result], type: T:bool }
    ],
    returns: C:_final
}

// is_odd(n) = if n == 0 then false else is_even(n - 1)
F:is_odd = fn {
    params: [n: T:i32],
    result: T:bool,
    body: [
        C:_zero = const { value: 0, type: T:i32 },
        C:_one = const { value: 1, type: T:i32 },
        C:_false = const { value: 0, type: T:bool },
        C:_is_zero = eq { inputs: [P:n, C:_zero], type: T:bool },
        C:_n_minus_1 = sub { inputs: [P:n, C:_one], type: T:i32 },
        C:_even_result = call_pure { target: "is_even", inputs: [C:_n_minus_1], type: T:bool },
        C:_final = select { inputs: [C:_is_zero, C:_false, C:_even_result], type: T:bool }
    ],
    returns: C:_final
}

C:c1 = const { value: 4, type: T:i32 }
C:c2 = call_pure { target: "is_even", inputs: [C:c1], type: T:bool }
C:c3 = call_pure { target: "is_odd", inputs: [C:c1], type: T:bool }
K:main = seq { steps: [C:c2, C:c3] }
entry: K:main
