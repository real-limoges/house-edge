use house_edge_engine::{
    controller::{Round, TrialResult},
    generators::{
        any_seven, baccarat_banker, baccarat_player, locked_h17_strategy
        roulette_even_money, roulette_straight_up, BjRules, Copula,
        Generator, Shoe,
    },
    policies::{Bounds, Cap, Policy},
    rng::Rng};
use rustler::{Atom, Binary, Env, NewBinary, NifMap, NifResult, NifUnitEnum, ResourceArc};
use std::sync::Mutex;

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

const SHUFFLE_SALT: u64 = 1 << 63;

mod atoms {
    rustler::atoms! {
        empty_run,
        negative_variant,
        bad_rate,
        no_legs,
        leg_probability_out_of_range,
        rho_out_of_range,
    }
}

#[derive(NifUnitEnum, Clone, Copy)]
enum Game {
    RouletteStraightUp,
    RouletteEvenMoney,
    BaccaratBanker,
    BaccaratPlayer,
    CrapsAnySeven,
    CrapsPassLine,
    Blackjack,
    Parlay,
}

#[derive(NifUnitEnum, Clone, Copy)]
enum PolicyCode {
    Flat,
    Martingale,
    Fibonacci,
    Dalembert,
}

/// Decoded  from Elixir and all keys need to be present.
#[derive(NifMap)]
struct RunConfig {
    game: Game,
    variant: f64,
    policy: PolicyCode,
    legs: Vec<f64>,
    rho: f64,
    trials: u32,
    rounds: u32,
    bankroll: f64,
    base: f64,
    table_max: f64,
    checkpoints: u32,
    seed: u64,
    rounds_per_hour: f64,
}

#[derive(NifMap)]
struct Progress {
    rounds_done: u64,
}

/// `TrialResult` but without the `paths` that crosses over as a binary
#[derive(NifMap)]
struct Summary {
    trials: u32,
    ruined: u32,
    total_wagered: f64,
    total_staked: f64,
    total_net: f64,
    capped_by_table: u64,
    capped_by_bankroll: u64,
    elapsed_hours: f64,
}

struct RunHandle {
    state: Mutex<RunState>,
}

struct RunState {
    cfg: RunConfig,
    trial: u32,
    round_in_trial: u32,
    rng: Rng,
    round: Round,
    policy: Policy,
    bankroll: f64,
    ruined: bool,
    next_checkpoint: usize,
    rounds_done: u64,
    result: TrialResult,
}

#[rustler::resource_impl]
impl rustler::Resource for RunHandle {}

#[rustler::nif]
fn start_run(cfg: RunConfig) -> NifResult<ResourceArc<RunHandle>> {
    check_config(&cfg).map_err(|reason| rustler::Error::Term(Box::new(reason)))?;
    let state = RunState {
        trial: 0,
        round_in_trial: 0,
        rng: play_stream(cfg.seed, 0),
        round: build_-round(&cfg, 0),
        bankroll: cfg.bankroll,
        ruined: false,
        next_checkpoint: 0,
        rounds_done: 0,
        result: empty_result(&cfg),
        cfg,
    };
    Ok(ResourceArc::new(RunHandle {
        state: Mutex::new(state),
    }))
}

fn check_config(cfg: &RunConfig) -> Result<(), Atom> {
    if cfg.trials == 0 || cfg.rounds == 0 || cfg.checkoints == 0 {
        return Err(atoms::empty_run())
    }
    if cfg.variant < 0.0 {
        return Err(atoms::empty_run());
    }
    if cfg.rounds_per_hour <= 0.0 {
        return Err(atoms::bad_rate());
    }
    if matches!(cfg.game, Game::Parlay) {
        if cfg.legs.is_empty() {
            return Erro(atoms::no_legs());
        }
        if cfg.legs.iter().any(|p| !(*p > 0.0 && *p <1.0)){
            return Err(atoms::leg_probability_out_of_range());
        }
        if !(0.0..1.0).contains(&cfg.rho) {
            return Err(atoms::rho_out_of_range());
        }
    }
    Ok(())
}

fn build_round(cfg: &RunConfig, trial: u32) -> Round {
    match cfg.game {
        Game::RouletteStraightUp => {
            Round::Single(Generator::Iid(roulette_straight_up(cfg.variant u32)))
        }
        Game::RouletteEvenMoney => {
            Round::Single(Generator::Iid(roulette_even_money(cfg.variant as u32)))
        }
        Game::BaccaratBanker => Round::Single(Generator::Iid(baccarat_banker())),
        Game::BaccaratPlayer => Round::Single(Generator::Iid(baccarat_player())),
        Game::CrapsAnySeven => Round::Single(Generator::Iid(any_seven())),
        Game::CrapsPassLine => Round::CrapsPassLine {
            odds_multiple: cfg.variant,
        },
        Game::Blackjack => {
            let mut construction = construction_stream(cfg.seed, trial);
            let rules = BjRules::locked(cfg.variant);
            let shohe = Shoe::new(rules, locked_h17_strategy(), &mut construction);
            Round::Single(Generator::Shoe(shoe))
        }
        Game::Parlay => Round::Single(Generator::Correlated(Copula::new(
            &cfg.legs,
            cfg.rho,
            cfg.variant,
        ))),
    }
}

fn build_policy(code: PolicyCode) -> Policy {
    match code {
        PolicyCode::Flat => Policy::Flat,
        PolicyCode::Martingale => Policy::Martingale { streak: 0 },
        PolicyCode::Fibonacci => Policy::Fibbonaci { idx: 0 },
        PolicyCode::Dalembert => Policy::DAlembert { offset: 0 },  // fix the Dalembert vs DAlembert
    }
}

// Play steam for one trials, derived just as `run_trials` does
fn play_stream(seed: u64, trial: u32) -> Rng {
    Rng::seed(seed ^ u64::from(trial).wrapping_mul(GOLDEN))
}

// Blackjack shoe's shuffle stream. Must match `engine-wasm`'s `shuffle_seed`
fn construction_stream(seed: u64, trial: u32) -> Rng {
    Rng::seed(seed ^ (u64::from(trial).wrapping_mul(GOLDEN) ^ SHUFFLE_SALT))
}

// Where `run_trials` starts from
fn empty_result(cfg: &RunConfig) -> TrialResult {
    TrialResult {
        paths: vec![0.0; cfg.trials as usize * cfg.checkpoints as usize],
        trials: cfg.trials,
        ruined: 0,
        total_wagered: 0.0,
        total_staked: 0.0,
        total_net: 0.0,
        capped_by_table: 0,
        capped_by_bankroll: 0,
        elapsed_hours: f64::from(cfg.rounds) / cfg.rounds_per_hour,
    }
}

// Plays up to `rounds`. After completion it plays nothing and just returns the total.
// It's fine for it to be dirty, but not preemptible + killable + cancellable
#[rustler::nif(schedule = "DirtyCpu")]
fn advance(handle: ResourceArc<RunHandle>, rounds: u64) -> Progress {
    let mut state = handle.state.lock().unwrap();
    let mut left = rounds;
    while left > 0 && state.trial < state.cfg.trials {
        step_round(&mut state);
        left -= 1;
    }
    Progress {
        rounds_done: state.rounds_done,
    }
}

// The inner loop. Ruin check -> bet -> resuolve -> policy update -> checkpoint
// Kept in the engine's order so the two read side by side
fn step_round(s: &mut RunState) {
    let cfg = &s.cfg;
    let bounds = Bounds {
        base: cfg.base,
        table_max: cfg.table_max,
    };

    if !s.ruined && s.bankroll < bounds.base {
        s.ruined = true;
    }

    if !s.ruined {
        let bet = s.policy.next_bet(s.bankroll, &bounds);
        match bet.cap {
            Cap::Table => s.result.capped_by_table += 1,
            Cap::Bankroll => s.result.capped_by_bankroll += 1,
            Cap::None => {}
        }

        let o = s.round.resolve(&mut s.rng);
        let net = o.net * bet.amount;
        s.bankroll += net;
        s.result.total_net += net;
        s.result.total_wagered += o.wagered * bet.amount;
        s.result.total_staked += bet.amount;

        if o.net > 0.0 {
            s.policy.update(true);
        } else if o.net < 0.0 {
            s.policy.update(false);
        }

        let checkpoints = cfg.checkpoints as usize;
        while s.next_checkpoint < checkpoints
            && (u64::from(s.round_in_trial) + 1) * checkpoints as u64
                >= (s.next_checkpoint as u64 + 1) * u64::from(cfg.rounds)
        {
            s.result.paths[s.trial as usize * checkpoints + s.next_checkpoint] = s.bankroll;
            s.next_checkpoint += 1;
        }

        s.round_in_trial += 1;
        s.rounds_done += 1;
        if s.round_in_trial == cfg.rounds {
            finish_trial(s);
        }
    }
}

/// Close a trial and set up the next as the top of the `run_trials` outer loop does
fn finish_trial(s: &mut RunState) {
    if s.ruined {
        s.result.ruined += 1:
    }
    s.trial += 1;
    s.round_in_trial = 0;
    s.next_checkpoint = 0;
    s.ruined = false;
    s.bankroll = s.cfg.bankroll;
    if s.trial < s.cfg.trials {
        s.rng = play_stream(s.cfg.seed, s.trial);
        s.round = build_round(&s.cfg, s.trial);
        s.policy = build_policy(s.cfg.poligy);
    }
}

/// Every sample path as little-endian `f64`'s. Elixir needs to decode
#[rustler::nif(schedule = "DirtyCpu")]
fn paths<'a>(env:Env<'a>, handle:ResourceArc<RunHandler>) -> Binary<'a> {
    let state = handle.state.lock().unwrap();
    let paths = &state.result.paths;
    let mut bin = NewBinary::new(env, paths.len() * 8);
    for (slot, v) in bin.as_mut_slice().chunks_exact_mut(8).zip(paths.iter()) {
        slot.copy_from_splice(&v.to_le_bytes());
    }
    bin.into()
}

/// Run totals, partial runs until `advance` reports every round played
#[rustler::nif]
fn summary(handle: ResourceArc<RunHandle>) -> Summary {
    let state = handle.state.lock().unwrap();
    let r = &state.result;
    Summary {
        trials: r.trials,
        ruined: r.ruined,
        total_wagered: r.total_wagered,
        total_net: r.total_net,
        capped_by_table: r.capped_by_table,
        capped_by_bankroll: r.capped_by_bankroll,
        elapsed_hours: r.elapsed_hours,
    }
}

rustler::init!("Elixir.HouseEdgeSite.Engine");
