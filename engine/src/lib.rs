// lib.rs

mod controller;
mod generators;
mod policies;
mod rng;
mod stats;

pub use controller::Round;
pub use generators::{Copula, Generator, IidTable, Outcome, Shoe};
pub use policies::Policy;

use crate::rng::Rng;

pub struct SimParams {
    pub trials: u32,
    pub rounds: u32,
    pub start_bankroll: f64,
    pub base_bet: f64,
    pub seed: u64,
}

pub struct SimResult {
    pub rounds: u32,
    pub p05: Vec<f64>,
    pub p50: Vec<f64>,
    pub p95: Vec<f64>,
    pub ruin_prob: f64, // fraction that hit 0
}

pub fn run_trials(
    params: &SimParams,
    make_gen: impl Fn() -> Generator,
    make_policy: impl Fn() -> Policy,
    round: Round,
) -> SimResult {
    let r = params.rounds as usize;
    // column-major: bankroll[round]
    let mut cols: Vec<Vec<f64>> = vec![Vec::with_capacity(params.trials as usize); r + 1];
    let mut ruined = 0u32;

    for t in 0..params.trials {
        let mut rng = Rng::seed(params.seed ^ (t as u64).wrapping_mul(0x9E3779B1));
        let mut gen = make_gen();
        let mut pol = make_policy();
        let mut bank = params.start_bankroll;
        cols[0].push(bank);
        let mut alive = true;
        for i in 0..r {
            if alive {
                let bet = pol.next_bet() * params.base_bet;
                if bet > bank { alive = false; } // cap binds
                else {
                    let o = round.resolve(&mut gen, &mut rng);
                    bank += o.net * bet;
                    pol.update(o.net);
                    if bank <= 0.0 { alive = false };
                }
            }
            if !alive { bank = 0.0 }
            cols[i + 1].push(bank);
        }
        if !alive { ruined += 1; }
    }

    let (mut p05, mut p50, mut p95) = (vec![], vec![], vec![]);
    for col in &mut cols {
        col.sort_by(|a, b| a.partial_cmp(b).unwrap());
        p05.push(percentile(col, 0.05));
        p50.push(percentile(col, 0.50));
        p95.push(percentile(col, 0.95));
    }
    SimResult { rounds: params.rounds, p05, p50, p95, ruin_prob: ruined as f64 / params.trials as f64 }
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[idx]
}

use core::slice;

// reserve region in linear memory; JS owns the returned pointer until free_f64
#[no_mangle]
pub extern "C" fn alloc_f64(len: usize) -> *mut f64 {
    let mut v = Vec::<f64>::with_capacity(len);
    let ptr = v.as_mut_ptr();
    core::mem::forget(v); // hand ownership to JS; freed via free_f64
    ptr
}

#[no_mangle]
pub extern "C" fn free_f64(ptr: *mut f64, len: usize) {
    unsafe { drop(Vec::from_raw_parts(ptr, 0, len)); }
}

// layout (f64): [game, variant, policy, trials, rounds, bankroll,
//                base_bet, rho, legs, table_max, seed_lo, seed_hi]
// out layout (f64): [ruin_prob, p05[0..=rounds], p50[..], p95[..]]
// returns number sittign in `out`.
#[no_mangle]
pub extern "C" fn simulate(params: *const f64, plen: usize, out: *mut f64, olen: usize) -> usize {
    let p = unsafe { slice::from_raw_parts(params, plen) };
    let cfg = decode_params(p);  // SimParams + gen/pol selectors
    let res = run_trials(&cfg.params, cfg.make_gen(), cfg.make_policy(), cfg.round);
    let o = unsafe { slice::from_raw_parts_mut(out, olen) };
    encode_result(&res, o)
}

/// Decoded simulation configuration: owned params plus generator/policy
/// selectors. `make_gen`/`make_policy` return owned closures (capturing copies
/// of the primitives) so `simulate` can still move `round` out of the config.
struct SimConfig {
    params: SimParams,
    round: Round,
    game: i32,
    policy: i32,
    rho: f64,
    legs: usize,
    table_max: f64,
    bankroll: f64,
}

impl SimConfig {
    fn make_gen(&self) -> impl Fn() -> Generator {
        let (game, rho, legs) = (self.game, self.rho, self.legs);
        move || match game {
            // Parlay: equi-correlated legs, fair even-money payout as a placeholder.
            2 => Generator::Correlated(Copula {
                rho,
                legs,
                p_win: 0.5,
                payout: ((1u64 << legs.min(52)) as f64) - 1.0,
            }),
            // TODO: game 1 (blackjack) once Shoe construction/strategy tables exist.
            // Placeholder single-zero roulette straight-up so the path runs end to end.
            _ => Generator::Iid(IidTable::new(&[(1.0 / 37.0, 35.0), (36.0 / 37.0, -1.0)])),
        }
    }

    fn make_policy(&self) -> impl Fn() -> Policy {
        let (policy, table_max, bankroll) = (self.policy, self.table_max, self.bankroll);
        // Bases are in units; run_trials scales by params.base_bet.
        move || match policy {
            1 => Policy::Martingale { streak: 0, base: 1.0, mult: 2.0, table_max, bankroll_cap: bankroll },
            2 => Policy::Fibonacci { a: 0.0, b: 1.0, base: 1.0 },
            3 => Policy::DAlembert { offset: 0, base: 1.0 },
            _ => Policy::Flat,
        }
    }
}

/// params layout (f64): [game, variant, policy, trials, rounds, bankroll,
///                       base_bet, rho, legs, table_max, seed_lo, seed_hi]
fn decode_params(p: &[f64]) -> SimConfig {
    let f = |i: usize| p.get(i).copied().unwrap_or(0.0);
    let game = f(0) as i32;
    let round = match game {
        1 => Round::CrapsPassLine,
        2 => Round::Parlay,
        _ => Round::Single,
    };
    SimConfig {
        params: SimParams {
            trials: f(3) as u32,
            rounds: f(4) as u32,
            start_bankroll: f(5),
            base_bet: f(6),
            seed: ((f(11) as u64) << 32) ^ (f(10) as u64),
        },
        round,
        game,
        policy: f(2) as i32,
        rho: f(7),
        legs: f(8) as usize,
        table_max: f(9),
        bankroll: f(5),
    }
}

/// out layout (f64): [ruin_prob, p05[0..=rounds], p50[..], p95[..]].
/// Returns the count of f64 written (0 if `out` is too small).
fn encode_result(res: &SimResult, out: &mut [f64]) -> usize {
    let n = res.p50.len();
    let total = 1 + 3 * n;
    if out.len() < total {
        return 0;
    }
    out[0] = res.ruin_prob;
    out[1..1 + n].copy_from_slice(&res.p05);
    out[1 + n..1 + 2 * n].copy_from_slice(&res.p50);
    out[1 + 2 * n..total].copy_from_slice(&res.p95);
    total
}
