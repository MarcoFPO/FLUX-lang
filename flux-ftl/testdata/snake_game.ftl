// Snake Game — Full FTL v3 graph
// Based on SIMULATION-snake-game.md
// ~100 Nodes: Terminal, Sound, Game Logic, Renderer, Cleanup

// ============================================================
// TYPES
// ============================================================

// Primitives
T:a10 = integer { bits: 32, signed: true }
T:a13 = integer { bits: 64, signed: false }
T:a14 = integer { bits: 16, signed: true }
T:a12 = boolean
T:a15 = unit
T:a16 = integer { bits: 8, signed: false }

// Struct: Position {x, y}
T:a1 = struct { fields: [x: T:a10, y: T:a10] }

// Array: snake body (max 800 segments)
T:a2 = array { element: T:a1, max_length: 800 }

// Variant: Direction
T:a11 = variant { cases: [UP: T:a15, DOWN: T:a15, LEFT: T:a15, RIGHT: T:a15] }

// Struct: GameState {snake, length, dir, food, score, alive}
T:a3 = struct { fields: [snake: T:a2, length: T:a10, dir: T:a11, food: T:a1, score: T:a10, alive: T:a12] }

// Framebuffer (16384 bytes, corrected from simulation iteration 3)
T:a4 = array { element: T:a16, max_length: 16384 }

// PCM sound buffer (2048 samples, S16_LE)
T:a5 = array { element: T:a14, max_length: 2048 }

// ANSI escape sequences
T:a6 = array { element: T:a16, max_length: 32 }

// Float for sin approximation
T:a17 = integer { bits: 32, signed: true }

// ============================================================
// REGIONS
// ============================================================

R:b1 = region { lifetime: static }
R:b2 = region { lifetime: scoped, parent: R:b1 }
R:b3 = region { lifetime: scoped, parent: R:b2 }
R:b4 = region { lifetime: scoped, parent: R:b1 }

// ============================================================
// CONSTANTS
// ============================================================

// File descriptors and syscall params
C:c_stdin = const { value: 0, type: T:a10 }
C:c_stdout = const { value: 1, type: T:a10 }
C:c_zero = const { value: 0, type: T:a10 }
C:c_one = const { value: 1, type: T:a10 }
C:c_neg1 = const { value: -1, type: T:a10 }
C:c_exit0 = const { value: 0, type: T:a10 }
C:c_exit1 = const { value: 1, type: T:a10 }
C:c_true = const { value: 1, type: T:a12 }
C:c_false = const { value: 0, type: T:a12 }

// Grid dimensions
C:c_width = const { value: 40, type: T:a10 }
C:c_height = const { value: 20, type: T:a10 }
C:c_max_snake = const { value: 800, type: T:a10 }

// Initial position
C:c_init_x = const { value: 20, type: T:a10 }
C:c_init_y = const { value: 10, type: T:a10 }
C:c_init_pos = const { value: 0, type: T:a1 }

// Food initial position
C:c_food_x = const { value: 7, type: T:a10 }
C:c_food_y = const { value: 14, type: T:a10 }

// Sound parameters
C:c_freq_eat = const { value: 880, type: T:a10 }
C:c_freq_die = const { value: 220, type: T:a10 }
C:c_sample_rate = const { value: 44100, type: T:a10 }
C:c_samples_eat = const { value: 276, type: T:a10 }
C:c_samples_die = const { value: 2048, type: T:a10 }

// Score increment
C:c_score_inc = const { value: 10, type: T:a10 }

// Typed zero constants for specific element types
C:c_zero_u8 = const { value: 0, type: T:a16 }
C:c_zero_i16 = const { value: 0, type: T:a14 }

// ioctl constants for terminal raw mode
C:c_tcgets = const { value: 21505, type: T:a13 }
C:c_tcsets = const { value: 21506, type: T:a13 }

// ALSA device path: /dev/snd/pcmC0D0p
C:c_alsa_path = const_bytes { value: [47,100,101,118,47,115,110,100,47,112,99,109,67,48,68,48,112,0], type: T:a6, region: R:b1 }

// ANSI: hide cursor, clear screen
C:c_ansi_hide = const_bytes { value: [27,91,63,50,53,108], type: T:a6, region: R:b1 }
C:c_ansi_show = const_bytes { value: [27,91,63,50,53,104], type: T:a6, region: R:b1 }
C:c_ansi_clear = const_bytes { value: [27,91,50,74,27,91,72], type: T:a6, region: R:b1 }

// Sleep duration (150ms in nanoseconds)
C:c_sleep_ns = const { value: 150000000, type: T:a13 }

// Framebuffer size
C:c_fb_size = const { value: 16384, type: T:a13 }

// ============================================================
// MEMORY NODES — Framebuffer and PCM buffer
// ============================================================

M:g1 = alloc { type: T:a4, region: R:b3 }
M:g2 = alloc { type: T:a5, region: R:b4 }
M:g3 = alloc { type: T:a3, region: R:b2 }

// Game state store/load
M:g4 = store { target: M:g3, index: C:c_zero, value: C:c_init_pos }
M:g5 = load { source: M:g3, index: C:c_zero, type: T:a3 }

// Framebuffer write (render output)
M:g6 = store { target: M:g1, index: C:c_zero, value: C:c_zero_u8 }

// Framebuffer read (for syscall_write)
M:g7 = load { source: M:g1, index: C:c_zero, type: T:a16 }

// PCM buffer write (sound samples)
M:g8 = store { target: M:g2, index: C:c_zero, value: C:c_zero_i16 }

// PCM buffer read (for pcm_write)
M:g9 = load { source: M:g2, index: C:c_zero, type: T:a14 }

// ============================================================
// TERMINAL INIT (E-Nodes with success/failure)
// ============================================================

// Save terminal state and set raw mode
E:d_term_save = syscall_ioctl { inputs: [C:c_stdin, C:c_tcgets, M:g3], type: T:a10, effects: [IO], success: K:f_term_raw, failure: K:f_cleanup }
K:f_term_raw = seq { steps: [E:d_term_set] }
E:d_term_set = syscall_ioctl { inputs: [C:c_stdin, C:c_tcsets, M:g3], type: T:a10, effects: [IO], success: K:f_term_ok, failure: K:f_cleanup }

// Write hide-cursor ANSI sequence
K:f_term_ok = seq { steps: [E:d_hide_cursor] }
E:d_hide_cursor = syscall_write { inputs: [C:c_stdout, C:c_ansi_hide, C:c_one], type: T:a10, effects: [IO], success: K:f_term_done, failure: K:f_cleanup }
K:f_term_done = seq { steps: [E:d_snd_init] }

// ============================================================
// SOUND INIT
// ============================================================

E:d_snd_init = syscall_open { inputs: [C:c_alsa_path], type: T:a10, effects: [IO], success: K:f_snd_ok, failure: K:f_cleanup }
K:f_snd_ok = seq { steps: [K:f_game_init] }

// ============================================================
// GAME INIT — Pure computation
// ============================================================

// Initialize game state
C:c_game_init = const { value: 0, type: T:a3 }
K:f_game_init = seq { steps: [M:g4, K:f_game_loop] }

// ============================================================
// GAME LOOP — K:loop with game state
// ============================================================

// Alive check (loaded from game state)
C:c_alive = load { source: M:g3, index: C:c_zero, type: T:a12 }

K:f_game_loop = loop { condition: C:c_alive, body: K:f_tick, state: M:g5, state_type: T:a3 }

// ============================================================
// TICK — One game loop iteration
// ============================================================

K:f_tick = seq { steps: [E:d_read_key, C:c_update, K:f_render_phase, K:f_food_check, E:d_sleep] }

// --- Input: non-blocking read ---
E:d_read_key = syscall_read { inputs: [C:c_stdin, M:g3, C:c_one], type: T:a10, effects: [IO], success: K:f_key_ok, failure: K:f_key_ok }
K:f_key_ok = seq { steps: [C:c_update] }

// --- Update: pure game state computation ---
C:c_update = call_pure { target: "update_game", inputs: [M:g5, E:d_read_key], type: T:a3 }

// --- Collision check ---
C:c_self_collide = call_pure { target: "check_self_collision", inputs: [C:c_update], type: T:a12 }
C:c_wall_collide = call_pure { target: "check_wall_collision", inputs: [C:c_update, C:c_width, C:c_height], type: T:a12 }
C:c_any_collide = or { inputs: [C:c_self_collide, C:c_wall_collide], type: T:a12 }

// --- Alive check: branch on collision ---
K:f_alive_check = branch { condition: C:c_any_collide, true: K:f_die, false: K:f_continue }

// --- Food check ---
C:c_head_eq_food = call_pure { target: "head_equals_food", inputs: [C:c_update], type: T:a12 }
K:f_food_check = branch { condition: C:c_head_eq_food, true: K:f_eat, false: K:f_no_eat }

// ============================================================
// RENDER PHASE
// ============================================================

K:f_render_phase = seq { steps: [C:c_render, M:g6, E:d_write_frame] }

// Render game state to framebuffer (pure)
C:c_render = call_pure { target: "render_game", inputs: [C:c_update, C:c_width, C:c_height], type: T:a4 }

// Write framebuffer to stdout
E:d_write_frame = syscall_write { inputs: [C:c_stdout, M:g1, C:c_fb_size], type: T:a10, effects: [IO], success: K:f_frame_done, failure: K:f_cleanup }
K:f_frame_done = seq { steps: [K:f_food_check] }

// ============================================================
// EAT — Food consumed: score++, grow snake, play sound
// ============================================================

K:f_eat = seq { steps: [C:c_new_score, C:c_grow, K:f_eat_sound] }

C:c_new_score = add { inputs: [M:g5, C:c_score_inc], type: T:a10 }
C:c_grow = add { inputs: [M:g5, C:c_one], type: T:a10 }

// Generate eat sound via Bhaskara sine approximation
C:c_angle_eat = mul { inputs: [C:c_freq_eat, C:c_samples_eat], type: T:a10 }
C:c_sin_eat = bhaskara_approx { inputs: [C:c_angle_eat], type: T:a14 }

// Fill PCM buffer with sine samples
M:g8_eat = store { target: M:g2, index: C:c_zero, value: C:c_sin_eat }

// Write PCM to ALSA device
K:f_eat_sound = seq { steps: [M:g8_eat, E:d_pcm_eat] }
E:d_pcm_eat = syscall_write { inputs: [E:d_snd_init, M:g2, C:c_samples_eat], type: T:a10, effects: [IO], success: K:f_no_eat, failure: K:f_no_eat }

// No eat: continue
K:f_no_eat = seq { steps: [E:d_sleep] }

// ============================================================
// DIE — Game over: play death sound, set alive=false
// ============================================================

K:f_die = seq { steps: [C:c_set_dead, K:f_die_sound] }

C:c_set_dead = const { value: 0, type: T:a12 }

// Generate death sound
C:c_angle_die = mul { inputs: [C:c_freq_die, C:c_samples_die], type: T:a10 }
C:c_sin_die = bhaskara_approx { inputs: [C:c_angle_die], type: T:a14 }
M:g8_die = store { target: M:g2, index: C:c_zero, value: C:c_sin_die }

K:f_die_sound = seq { steps: [M:g8_die, E:d_pcm_die] }
E:d_pcm_die = syscall_write { inputs: [E:d_snd_init, M:g2, C:c_samples_die], type: T:a10, effects: [IO], success: K:f_after_die, failure: K:f_after_die }
K:f_after_die = seq { steps: [K:f_cleanup] }

// ============================================================
// CONTINUE — Normal tick end
// ============================================================

K:f_continue = seq { steps: [K:f_render_phase] }

// Sleep (nanosleep syscall)
E:d_sleep = syscall_nanosleep { inputs: [C:c_sleep_ns], type: T:a10, effects: [IO], success: K:f_tick_end, failure: K:f_tick_end }
K:f_tick_end = seq { steps: [K:f_alive_check] }

// ============================================================
// CLEANUP — Converging failure/exit path
// All failure paths lead here: term_restore + sound_close + exit
// ============================================================

K:f_cleanup = seq { steps: [E:d_snd_close, E:d_restore, E:d_show_cursor, E:d_exit_clean] }

// Close ALSA device
E:d_snd_close = syscall_close { inputs: [E:d_snd_init], type: T:a10, effects: [IO], success: K:f_restore, failure: K:f_restore }
K:f_restore = seq { steps: [E:d_restore] }

// Restore terminal settings
E:d_restore = syscall_ioctl { inputs: [C:c_stdin, C:c_tcsets, M:g3], type: T:a10, effects: [IO], success: K:f_show, failure: K:f_show }
K:f_show = seq { steps: [E:d_show_cursor] }

// Show cursor again
E:d_show_cursor = syscall_write { inputs: [C:c_stdout, C:c_ansi_show, C:c_one], type: T:a10, effects: [IO], success: K:f_exit_seq, failure: K:f_exit_seq }
K:f_exit_seq = seq { steps: [E:d_exit_clean] }

// Exit
E:d_exit_clean = syscall_exit { inputs: [C:c_exit0], type: T:a15, effects: [PROC] }
E:d_exit_fail = syscall_exit { inputs: [C:c_exit1], type: T:a15, effects: [PROC] }

// ============================================================
// CONTRACTS
// ============================================================

// Terminal fd is stdin
V:e1 = contract { target: E:d_term_save, pre: C:c_stdin.val == 0 }

// Snake bounds: all positions within grid
V:e2 = contract { target: K:f_game_loop, invariant: forall i in 0..state.length: state.snake[i].x >= 0 }
V:e3 = contract { target: K:f_game_loop, invariant: forall i in 0..state.length: state.snake[i].x < 40 }
V:e4 = contract { target: K:f_game_loop, invariant: forall i in 0..state.length: state.snake[i].y >= 0 }
V:e5 = contract { target: K:f_game_loop, invariant: forall i in 0..state.length: state.snake[i].y < 20 }

// Framebuffer size: render output fits buffer
V:e6 = contract { target: C:c_render, post: result.size <= 16384 }

// Bhaskara sine approximation range
V:e7 = contract { target: C:c_sin_eat, post: result >= -1 AND result <= 1 }
V:e8 = contract { target: C:c_sin_die, post: result >= -1 AND result <= 1 }

// Snake length invariant
V:e9 = contract { target: K:f_game_loop, invariant: state.length <= 800 }

// ALSA path is non-null
V:e10 = contract { target: E:d_snd_init, pre: C:c_alsa_path != null }

// ============================================================
// MAIN — Entry point
// ============================================================

K:f_main = seq { steps: [E:d_term_save] }
entry: K:f_main
