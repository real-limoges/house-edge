use crate::generators::{Generator, Outcome};
use crate::policies::{Bounds, Cap, Policy};
use crate::rng::Rng;

#[inline]
fn roll(rng: &mut Rng) -> u32 {
    let d1 = (rng.next_u64() % 6) as u32 + 1;
    let d2 = (rng.next_u64() % 6) as u32 + 1;
    d1 + d2
}

/// Free odds go up only once a point is set.
pub fn pass_line(rng: &mut Rng, odds_multiple: f64) -> Outcome {
    match roll(rng) {
        7 | 11 => Outcome {
            net: 1.0,
            wagered: 1.0,
        },
        2 | 3 | 12 => Outcome {
            net: -1.0,
            wagered: 1.0,
        },
        point => {
            let true_odds = match point {
                4 | 10 => 2.0,
                5 | 9 => 1.5,
                6 | 8 => 6.0 / 5.0,
                other => unreachable!("impossible point {other}"),
            };
            let wagered = 1.0 + odds_multiple;
            loop {
                match roll(rng) {
                    r if r == point => {
                        return Outcome {
                            net: 1.0 + odds_multiple * true_odds,
                            wagered,
                        }
                    }
                    7 => {
                        return Outcome {
                            net: -wagered,
                            wagered,
                        }
                    }
                    _ => continue,
                }
            }
        }
    }
}

pub struct TrialConfig {
    pub trials: u32,
    pub rounds: u32,
    pub bankroll: f64,
    pub bounds: Bounds,
    pub checkpoints: u32, // fan-chart x-positions
    pub seed: u64,
    pub rounds_per_hour: f64, // only sets `elapsed_hours`
}

pub struct TrialResult {
    pub paths: Vec<f64>, // trial-major, checkpoint-minor
    pub trials: u32,
    pub ruined: u32,
    pub total_wagered: f64, // every bet, doubles/splits/odds included
    pub total_staked: f64,  // initial bets only
    pub total_net: f64,
    pub capped_by_table: u64,
    pub capped_by_bankroll: u64,
    pub elapsed_hours: f64, // per trial
}

pub enum Round {
    Single(Generator),
    CrapsPassLine { odds_multiple: f64 },
}

impl Round {
    #[inline]
    pub fn resolve(&mut self, rng: &mut Rng) -> Outcome {
        match self {
            Round::Single(g) => g.resolve(rng),
            Round::CrapsPassLine { odds_multiple } => pass_line(rng, *odds_multiple),
        }
    }
}

/// Calls `make_trial` once per trial. Panics on zero `rounds` or `checkpoints`.
pub fn run_trials<F>(cfg: &TrialConfig, mut make_trial: F) -> TrialResult
where
    F: FnMut() -> (Round, Policy),
{
    assert!(cfg.rounds > 0, "trial has no rounds");
    assert!(cfg.checkpoints > 0, "trajectory has no checkpoints");

    let checkpoints = cfg.checkpoints as usize;
    let mut out = TrialResult {
        paths: vec![0.0; cfg.trials as usize * checkpoints],
        trials: cfg.trials,
        ruined: 0,
        total_wagered: 0.0,
        total_staked: 0.0,
        total_net: 0.0,
        capped_by_table: 0,
        capped_by_bankroll: 0,
        elapsed_hours: f64::from(cfg.rounds) / cfg.rounds_per_hour,
    };

    for trial in 0..cfg.trials {
        let mut rng = Rng::seed(cfg.seed ^ u64::from(trial).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let (mut round, mut policy) = make_trial();

        let mut bankroll = cfg.bankroll;
        let mut ruined = false;
        let mut next_c = 0usize;

        for r in 0..cfg.rounds {
            // Ruin: the bankroll can't cover the base bet.
            if !ruined && bankroll < cfg.bounds.base {
                ruined = true;
            }

            if !ruined {
                let bet = policy.next_bet(bankroll, &cfg.bounds);
                match bet.cap {
                    Cap::Table => out.capped_by_table += 1,
                    Cap::Bankroll => out.capped_by_bankroll += 1,
                    Cap::None => {}
                }

                let o = round.resolve(&mut rng);
                let net = o.net * bet.amount;
                bankroll += net;
                out.total_net += net;
                out.total_wagered += o.wagered * bet.amount;
                out.total_staked += bet.amount;

                if o.net > 0.0 {
                    policy.update(true);
                } else if o.net < 0.0 {
                    policy.update(false);
                }
            }

            // A ruined path holds flat, so the bands carry no survivorship bias.
            while next_c < checkpoints
                && (u64::from(r) + 1) * checkpoints as u64
                    >= (next_c as u64 + 1) * u64::from(cfg.rounds)
            {
                out.paths[trial as usize * checkpoints + next_c] = bankroll;
                next_c += 1;
            }
        }

        // Catches a bust on the final round, which the loop check never sees.
        if ruined || bankroll < cfg.bounds.base {
            out.ruined += 1;
        }
    }

    out
}
