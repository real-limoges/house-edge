//! The hand-rolled wasm boundary. No bindgen, so JS sees one `WebAssembly.Memory`
//! and a few functions taking integers and floats: an allocator pair and one
//! entry point that takes a flat `f64` config and writes a flat `f64` result.
//!
//! The config layout is the `CFG_*` slot constants below; the result is a fixed
//! header of run totals followed by the sampled bankroll paths. JS reads both
//! positionally, so slots only ever append.

use std::alloc::{alloc as rust_alloc, dealloc as rust_dealloc, Layout};

use house_edge_engine::controller::{run_trials, Round, TrialConfig};
use house_edge_engine::generators::{
    any_seven, baccarat_banker, baccarat_player, locked_h17_strategy, roulette_even_money,
    roulette_straight_up, BjRules, Generator, Shoe,
};
use house_edge_engine::policies::{Bounds, Policy};
use house_edge_engine::rng::Rng;

/// Reserve `len` bytes of linear memory, 8-aligned, and return the offset.
/// Returns null for `len == 0` or on failure; offset 0 is never a live allocation.
#[no_mangle]
pub extern "C" fn engine_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::null_mut();
    }

    // 8 for f64: every buffer crossing this boundary is an array of f64.
    match Layout::from_size_align(len, 8) {
        Ok(layout) => unsafe { rust_alloc(layout) },
        Err(_) => std::ptr::null_mut(),
    }
}

/// Release a reservation; `len` must be the length `engine_alloc` was given.
/// Null or zero `len` is a no-op. Nothing tracks reservations, so a missed free leaks.
// C-ABI export called from JS: cannot be `unsafe fn`; the raw pointer is the boundary.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn engine_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    unsafe { rust_dealloc(ptr, Layout::from_size_align_unchecked(len, 8)) }
}

/// Config slots with a fixed length.
/// Adding a field appends, never inserts, because JS uses these positionally.
const CFG_GAME: usize = 0;
const CFG_VARIANT: usize = 1; // zeros on the wheel, odds multiple on the pass line, payout on the natural
const CFG_POLICY: usize = 2;
const CFG_TRIALS: usize = 3;
const CFG_ROUNDS: usize = 4;
const CFG_BANKROLL: usize = 5;
const CFG_BASE: usize = 6;
const CFG_TABLE_MAX: usize = 7;
const CFG_CHECKPOINTS: usize = 8;
const CFG_SEED_HI: usize = 9; // upper 32 bits
const CFG_SEED_LO: usize = 10; // lower 32 bits
const CFG_ROUNDS_PER_HOUR: usize = 11;
pub const CFG_LEN: usize = 12;

/// Result header, then `trials * checkpoints` path samples.
pub const OUT_HEADER: usize = 6;

// Status codes. 0 is success.
pub const OK: i32 = 0;
pub const ERR_BAD_LEN: i32 = -1;
pub const ERR_BAD_GAME: i32 = -2;
pub const ERR_BAD_TABLE: i32 = -3;
pub const ERR_OUT_TOO_SMALL: i32 = -4;

/// Game selectors, in `CFG_GAME`; each reads at most one scalar from `CFG_VARIANT`.
/// Append-only, like the config slots.
const GAME_ROULETTE_STRAIGHT_UP: u32 = 0; // variant: green zeros on the wheel
const GAME_ROULETTE_EVEN_MONEY: u32 = 1; // variant: green zeros on the wheel
const GAME_BACCARAT_BANKER: u32 = 2; // variant: unused
const GAME_BACCARAT_PLAYER: u32 = 3; // variant: unused
const GAME_CRAPS_ANY_SEVEN: u32 = 4; // variant: unused
const GAME_CRAPS_PASS_LINE: u32 = 5; // variant: free-odds multiple behind the point
const GAME_BLACKJACK: u32 = 6; // variant: net paid on a natural (1.5 at 3:2, 1.2 at 6:5)

/// Staking policies, in `CFG_POLICY`. Every policy starts from base with zeroed
/// state; the trial loop advances it. Same append-only rule as the game codes.
const POLICY_FLAT: u32 = 0;
const POLICY_MARTINGALE: u32 = 1;
const POLICY_FIBONACCI: u32 = 2;
const POLICY_DALEMBERT: u32 = 3;

/// The odd mixing constant `run_trials` uses for per-trial seeds; must match it.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// The top bit, XORed into every shuffle seed so none equals a play seed.
const SHUFFLE_SALT: u64 = 1 << 63;

/// The seed for one trial's blackjack shoe shuffle.
/// Never equal to any trial's play seed, `seed ^ trial * GOLDEN`.
fn shuffle_seed(seed: u64, trial: u64) -> u64 {
    seed ^ (trial.wrapping_mul(GOLDEN) ^ SHUFFLE_SALT)
}

/// Fresh policy for one trial. `code` must already be validated.
fn make_policy(code: u32) -> Policy {
    match code {
        POLICY_MARTINGALE => Policy::Martingale { streak: 0 },
        POLICY_FIBONACCI => Policy::Fibonacci { idx: 0 },
        POLICY_DALEMBERT => Policy::DAlembert { offset: 0 },
        _ => Policy::Flat, // POLICY_FLAT, and the validated default
    }
}

// C-ABI export called from JS: cannot be `unsafe fn`. The two raw pointers are
// the boundary; both are null-checked and length-checked before any deref.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn engine_run(
    cfg: *const f64,
    cfg_len: usize,
    out: *mut f64,
    out_len: usize,
) -> i32 {
    if cfg.is_null() || out.is_null() || cfg_len != CFG_LEN {
        return ERR_BAD_LEN;
    }
    let cfg = unsafe { std::slice::from_raw_parts(cfg, cfg_len) };

    let trials = cfg[CFG_TRIALS] as usize;
    let rounds = cfg[CFG_ROUNDS] as usize;
    let checkpoints = cfg[CFG_CHECKPOINTS] as usize;

    // `run_trials` asserts on zero rounds or checkpoints, and under
    // panic = "abort" an assert is a silent trap, so these get a status code.
    if trials == 0 || rounds == 0 || checkpoints == 0 {
        return ERR_BAD_LEN;
    }
    if out_len < OUT_HEADER + trials * checkpoints {
        return ERR_OUT_TOO_SMALL;
    }

    let game = cfg[CFG_GAME] as u32;
    let policy = cfg[CFG_POLICY] as u32;
    let variant = cfg[CFG_VARIANT];

    // Validate both codes before spending a run.
    // An unknown policy reuses ERR_BAD_GAME; there is no policy-error code.
    if !matches!(
        game,
        GAME_ROULETTE_STRAIGHT_UP
            | GAME_ROULETTE_EVEN_MONEY
            | GAME_BACCARAT_BANKER
            | GAME_BACCARAT_PLAYER
            | GAME_CRAPS_ANY_SEVEN
            | GAME_CRAPS_PASS_LINE
            | GAME_BLACKJACK
    ) {
        return ERR_BAD_GAME;
    }
    if !matches!(
        policy,
        POLICY_FLAT | POLICY_MARTINGALE | POLICY_FIBONACCI | POLICY_DALEMBERT
    ) {
        return ERR_BAD_GAME;
    }

    // A NaN or negative odds multiple or payout is a nonsense bet; reject it
    // before a table constructor can assert on it.
    if (game == GAME_CRAPS_PASS_LINE || game == GAME_BLACKJACK)
        && (variant.is_nan() || variant < 0.0)
    {
        return ERR_BAD_TABLE;
    }

    // Two 32-bit halves, because a u64 above 2^53 does not survive f64.
    let seed = ((cfg[CFG_SEED_HI] as u64) << 32) | (cfg[CFG_SEED_LO] as u64);

    let trial_cfg = TrialConfig {
        trials: trials as u32,
        rounds: rounds as u32,
        bankroll: cfg[CFG_BANKROLL],
        bounds: Bounds {
            base: cfg[CFG_BASE],
            table_max: cfg[CFG_TABLE_MAX],
        },
        checkpoints: checkpoints as u32,
        seed,
        rounds_per_hour: cfg[CFG_ROUNDS_PER_HOUR],
    };

    // A fresh (Round, Policy) per trial, since a reused Shoe would correlate trials.
    // `next_trial` tracks `run_trials`' trial index, which calls this once per trial.
    let mut next_trial = 0u64;
    let result = run_trials(&trial_cfg, || {
        let trial = next_trial;
        next_trial += 1;
        let round = match game {
            GAME_ROULETTE_STRAIGHT_UP => {
                Round::Single(Generator::Iid(roulette_straight_up(variant as u32)))
            }
            GAME_ROULETTE_EVEN_MONEY => {
                Round::Single(Generator::Iid(roulette_even_money(variant as u32)))
            }
            GAME_BACCARAT_BANKER => Round::Single(Generator::Iid(baccarat_banker())),
            GAME_BACCARAT_PLAYER => Round::Single(Generator::Iid(baccarat_player())),
            GAME_CRAPS_ANY_SEVEN => Round::Single(Generator::Iid(any_seven())),
            GAME_CRAPS_PASS_LINE => Round::CrapsPassLine {
                odds_multiple: variant,
            },
            GAME_BLACKJACK => {
                let mut construction = Rng::seed(shuffle_seed(seed, trial));
                let rules = BjRules::locked(variant);
                let shoe = Shoe::new(rules, locked_h17_strategy(), &mut construction);
                Round::Single(Generator::Shoe(shoe))
            }
            // Unreachable: the game code was validated above.
            _ => Round::Single(Generator::Iid(any_seven())),
        };
        (round, make_policy(policy))
    });

    // Header, in the order JS reads it. `total_staked` is deliberately absent.
    let out = unsafe { std::slice::from_raw_parts_mut(out, out_len) };
    out[0] = f64::from(result.ruined);
    out[1] = result.total_wagered;
    out[2] = result.total_net;
    out[3] = result.capped_by_table as f64;
    out[4] = result.capped_by_bankroll as f64;
    out[5] = result.elapsed_hours;

    // Then the fan-chart paths, trial-major and checkpoint-minor, exactly as
    // `run_trials` laid them out. The out_len check above guarantees the room.
    out[OUT_HEADER..OUT_HEADER + result.paths.len()].copy_from_slice(&result.paths);

    OK
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The play seed `run_trials` derives for trial `t`.
    fn play_seed(seed: u64, trial: u64) -> u64 {
        seed ^ trial.wrapping_mul(GOLDEN)
    }

    /// No trial's shuffle seed equals any trial's play seed, its own and its neighbors' included.
    #[test]
    fn shuffle_seeds_never_meet_play_seeds() {
        let seed = 0x1234_5678_9ABC_DEF0;
        let play: std::collections::HashSet<u64> =
            (0..10_000).map(|t| play_seed(seed, t)).collect();
        for t in 0..10_000 {
            assert!(
                !play.contains(&shuffle_seed(seed, t)),
                "trial {t}'s shuffle seed is a play seed"
            );
        }
    }
}
