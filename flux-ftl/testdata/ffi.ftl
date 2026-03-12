// FFI — External C library calls (malloc/free/memcpy)
// Demonstrates X-Node declarations, call_extern E-Nodes,
// opaque types, and trust: EXTERN contracts

// === Typen ===
T:a1 = integer { bits: 64, signed: false }
T:a2 = integer { bits: 32, signed: true }
T:a3 = unit
T:a4 = boolean
T:ptr = integer { bits: 64, signed: false }
T:size_t = integer { bits: 64, signed: false }

// Opaque type for external struct (e.g. FILE)
T:ext_file = opaque { size: 216, align: 8 }

// Buffer type
T:buf = array { element: u8, max_length: 4096 }

// === Regionen ===
R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b1 }

// === Extern declarations (X-Nodes) ===
X:ext1 = extern { name: "malloc", abi: C, params: [T:size_t], result: T:ptr, effects: [MEM] }
X:ext2 = extern { name: "free", abi: C, params: [T:ptr], result: T:a3, effects: [MEM] }
X:ext3 = extern { name: "memcpy", abi: C, params: [T:ptr, T:ptr, T:size_t], result: T:ptr, effects: [MEM] }
X:ext4 = extern { name: "fopen", abi: C, params: [T:ptr, T:ptr], result: T:ptr, effects: [IO] }
X:ext5 = extern { name: "fwrite", abi: C, params: [T:ptr, T:size_t, T:size_t, T:ptr], result: T:size_t, effects: [IO] }
X:ext6 = extern { name: "fclose", abi: C, params: [T:ptr], result: T:a2, effects: [IO] }

// === Constants ===
C:c_alloc_size = const { value: 4096, type: T:size_t }
C:c_zero = const { value: 0, type: T:a1 }
C:c_one = const { value: 1, type: T:size_t }
C:c_null = const { value: 0, type: T:ptr }
C:c_exit0 = const { value: 0, type: T:a2 }
C:c_exit1 = const { value: 1, type: T:a2 }

// File path and mode as byte arrays
T:a_path = array { element: u8, max_length: 16 }
T:a_mode = array { element: u8, max_length: 4 }
C:c_path = const_bytes { value: [47,116,109,112,47,111,117,116,46,98,105,110,0], type: T:a_path, region: R:b1 }
C:c_mode = const_bytes { value: [119,98,0], type: T:a_mode, region: R:b1 }

// Data to write
C:c_data = const_bytes { value: [72,101,108,108,111,32,70,70,73,10], type: T:buf, region: R:b1 }
C:c_data_len = const { value: 10, type: T:size_t }

// === Step 1: malloc — allocate buffer ===
E:d1 = call_extern { target: X:ext1, inputs: [C:c_alloc_size], type: T:ptr, effects: [MEM], success: K:f_alloc_ok, failure: K:f_cleanup }

// === Step 2: memcpy — copy data into allocated buffer ===
K:f_alloc_ok = seq { steps: [E:d2] }
E:d2 = call_extern { target: X:ext3, inputs: [E:d1, C:c_data, C:c_data_len], type: T:ptr, effects: [MEM], success: K:f_copy_ok, failure: K:f_free_and_exit }

// === Step 3: fopen — open output file ===
K:f_copy_ok = seq { steps: [E:d3] }
E:d3 = call_extern { target: X:ext4, inputs: [C:c_path, C:c_mode], type: T:ptr, effects: [IO], success: K:f_file_ok, failure: K:f_free_and_exit }

// === Step 4: fwrite — write buffer to file ===
K:f_file_ok = seq { steps: [E:d4] }
E:d4 = call_extern { target: X:ext5, inputs: [E:d1, C:c_one, C:c_data_len, E:d3], type: T:size_t, effects: [IO], success: K:f_write_ok, failure: K:f_close_free_exit }

// === Step 5: fclose — close file ===
K:f_write_ok = seq { steps: [E:d5] }
E:d5 = call_extern { target: X:ext6, inputs: [E:d3], type: T:a2, effects: [IO], success: K:f_close_ok, failure: K:f_free_and_exit }

// === Step 6: free — release buffer ===
K:f_close_ok = seq { steps: [E:d6] }
E:d6 = call_extern { target: X:ext2, inputs: [E:d1], type: T:a3, effects: [MEM], success: K:f_success, failure: K:f_exit_fail }

// === Success exit ===
K:f_success = seq { steps: [E:d_exit0] }
E:d_exit0 = syscall_exit { inputs: [C:c_exit0], type: T:a3, effects: [PROC] }

// === Failure / Cleanup paths ===

// Cleanup: close file, free buffer, exit(1)
K:f_close_free_exit = seq { steps: [E:d_close_cleanup, E:d_free_cleanup, E:d_exit1] }
E:d_close_cleanup = call_extern { target: X:ext6, inputs: [E:d3], type: T:a2, effects: [IO], success: K:f_free_and_exit, failure: K:f_free_and_exit }

// Cleanup: free buffer, exit(1)
K:f_free_and_exit = seq { steps: [E:d_free_cleanup, E:d_exit1] }
E:d_free_cleanup = call_extern { target: X:ext2, inputs: [E:d1], type: T:a3, effects: [MEM], success: K:f_exit_fail, failure: K:f_exit_fail }

// Cleanup: just exit(1)
K:f_exit_fail = seq { steps: [E:d_exit1] }
K:f_cleanup = seq { steps: [E:d_exit1] }
E:d_exit1 = syscall_exit { inputs: [C:c_exit1], type: T:a3, effects: [PROC] }

// === Contracts (trust: EXTERN for FFI assumptions) ===

// malloc returns non-null for valid size
V:e1 = contract { target: E:d1, trust: EXTERN, assume: result != 0, post: region_valid(R:b2) }

// memcpy returns dest pointer
V:e2 = contract { target: E:d2, trust: EXTERN, assume: result == E:d1, post: result != 0 }

// fopen returns non-null on success
V:e3 = contract { target: E:d3, trust: EXTERN, assume: result != 0, post: result != 0 }

// fwrite writes exactly the requested bytes
V:e4 = contract { target: E:d4, trust: EXTERN, assume: result == C:c_data_len.val, post: result > 0 }

// free does not fail (void return)
V:e5 = contract { target: E:d6, trust: EXTERN, assume: true, post: true }

// Verified contracts: parameter types match declarations
V:e6 = contract { target: E:d1, pre: C:c_alloc_size.val > 0 }
V:e7 = contract { target: E:d4, pre: C:c_data_len.val <= 4096 }

// === Main sequence ===
K:f_main = seq { steps: [E:d1] }
entry: K:f_main
