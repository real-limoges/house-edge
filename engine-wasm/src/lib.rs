//! Hand-rolled wasm boundary (no bindgen): an allocator pair and `engine_run`,
//! which reads a flat `f64` config and writes a flat `f64` result.

use std::alloc::{alloc as rust_alloc, dealloc as rust_dealloc, Layout};

use house_edge_engine::controller::{run_trials, Round, TrialConfig};
use house_edge_engine::generators::{
    any_seven, baccarat_banker, baccarat_player, locked_h17_strategy, roulette_even_money,
    roulette_straight_up, BjRules, Generator, Shoe,
};
use house_edge_engine::policies::{Bounds, Policy};
use house_edge_engine::rng::Rng;

/// 8-aligned, since every buffer crossing the boundary is `f64`. Null for `len == 0`.
#[no_mangle]
pub extern "C" fn engine_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::null_mut();
    }
    match Layout::from_size_align(len, 8) {
        Ok(layout) => unsafe { rust_alloc(layout) },
        Err(_) => std::ptr::null_mut(),
    }
}

/// `len` must match the `engine_alloc` call.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn engine_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    unsafe { rust_dealloc(ptr, Layout::from_size_align_unchecked(len, 8)) }
}

// JS reads every code and slot below positionally: append, never insert.

const CFG_GAME: usize = 0;
const CFG_VARIANT: usize = 1;
const CFG_POLICY: usize = 2;
const CFG_TRIALS: usize = 3;
const CFG_ROUNDS: usize = 4;
const CFG_BANKROLL: usize = 5;
const CFG_BASE: usize = 6;
const CFG_TABLE_MAX: usize = 7;
const CFG_CHECKPOINTS: usize = 8;
const CFG_SEED_HI: usize = 9;
const CFG_SEED_LO: usize = 10;
const CFG_ROUNDS_PER_HOUR: usize = 11;
pub const CFG_LEN: usize = 12;

/// Header length; `trials * checkpoints` path samples follow it.
pub const OUT_HEADER: usize = 6;

pub const OK: i32 = 0;
pub const ERR_BAD_LEN: i32 = -1;
pub const ERR_BAD_GAME: i32 = -2;
pub const ERR_BAD_TABLE: i32 = -3;
pub const ERR_OUT_TOO_SMALL: i32 = -4;

// What `CFG_VARIANT` means for each game, where it matters.
const GAME_ROULETTE_STRAIGHT_UP: u32 = 0; // green zeros
const GAME_ROULETTE_EVEN_MONEY: u32 = 1; // green zeros
const GAME_BACCARAT_BANKER: u32 = 2;
const GAME_BACCARAT_PLAYER: u32 = 3;
const GAME_CRAPS_ANY_SEVEN: u32 = 4;
const GAME_CRAPS_PASS_LINE: u32 = 5; // odds multiple
const GAME_BLACKJACK: u32 = 6; // natural payout (1.5 at 3:2)

const POLICY_FLAT: u32 = 0;
const POLICY_MARTINGALE: u32 = 1;
const POLICY_FIBONACCI: u32 = 2;
const POLICY_DALEMBERT: u32 = 3;

/// Must match the per-trial seed mixing in `run_trials`.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// Keeps every shuffle seed distinct from every play seed.
const SHUFFLE_SALT: u64 = 1 << 63;

fn shuffle_seed(seed: u64, trial: u64) -> u64 {
    seed ^ (trial.wrapping_mul(GOLDEN) ^ SHUFFLE_SALT)
}

fn make_policy(code: u32) -> Policy {
    match code {
        POLICY_MARTINGALE => Policy::Martingale { streak: 0 },
        POLICY_FIBONACCI => Policy::Fibonacci { idx: 0 },
        POLICY_DALEMBERT => Policy::DAlembert { offset: 0 },
        _ => Policy::Flat,
    }
}

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

    // `run_trials` asserts on these, and under panic = "abort" that is a silent trap.
    if trials == 0 || rounds == 0 || checkpoints == 0 {
        return ERR_BAD_LEN;
    }
    if out_len < OUT_HEADER + trials * checkpoints {
        return ERR_OUT_TOO_SMALL;
    }

    let game = cfg[CFG_GAME] as u32;
    let policy = cfg[CFG_POLICY] as u32;
    let variant = cfg[CFG_VARIANT];

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
    // No separate policy error code.
    if !matches!(
        policy,
        POLICY_FLAT | POLICY_MARTINGALE | POLICY_FIBONACCI | POLICY_DALEMBERT
    ) {
        return ERR_BAD_GAME;
    }

    if (game == GAME_CRAPS_PASS_LINE || game == GAME_BLACKJACK)
        && (variant.is_nan() || variant < 0.0)
    {
        return ERR_BAD_TABLE;
    }

    // Split in halves because a u64 above 2^53 does not survive f64.
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

    // `run_trials` calls this once per trial, so `next_trial` tracks its index.
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
            _ => unreachable!("game code validated above"),
        };
        (round, make_policy(policy))
    });

    let out = unsafe { std::slice::from_raw_parts_mut(out, out_len) };
    out[0] = f64::from(result.ruined);
    out[1] = result.total_wagered;
    out[2] = result.total_net;
    out[3] = result.capped_by_table as f64;
    out[4] = result.capped_by_bankroll as f64;
    out[5] = result.elapsed_hours;
    out[OUT_HEADER..OUT_HEADER + result.paths.len()].copy_from_slice(&result.paths);

    OK
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play_seed(seed: u64, trial: u64) -> u64 {
        seed ^ trial.wrapping_mul(GOLDEN)
    }

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
