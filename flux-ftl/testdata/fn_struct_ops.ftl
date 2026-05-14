// Phase 28: F-Node with struct_get, struct_set, variant_create/is/get
T:i32 = integer { bits: 32, signed: true }
T:bool = boolean

T:pos = struct { fields: [x: T:i32, y: T:i32] }

// F-Node: get x coordinate from a pos
F:get_x = fn {
    params: [p: T:pos],
    result: T:i32,
    body: [
        C:_x = struct_get { input: P:p, field: "x", type: T:i32 }
    ],
    returns: C:_x
}

// F-Node: set x coordinate in a pos
F:set_x = fn {
    params: [p: T:pos, new_x: T:i32],
    result: T:pos,
    body: [
        C:_new = struct_set { input: P:p, field: "x", value: P:new_x, type: T:pos }
    ],
    returns: C:_new
}

// F-Node: add two i32 values
F:add_i32 = fn {
    params: [a: T:i32, b: T:i32],
    result: T:i32,
    body: [
        C:_r = add { inputs: [P:a, P:b], type: T:i32 }
    ],
    returns: C:_r
}

// Main program: create a pos, get x, add 10, set x
C:c_x = const { value: 3, type: T:i32 }
C:c_y = const { value: 7, type: T:i32 }

K:main = seq { steps: [C:c_x, C:c_y] }
entry: K:main
