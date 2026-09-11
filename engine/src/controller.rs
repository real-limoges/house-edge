use crate::generators::{Generator, Outcome};
use crate::policies::{Bounds, Cap, Policy};
use crate::rng::Rng;

/// Rolls two d6 dice
#[inline]
fn roll(rng: &mut Rng) -> u32 {
    let d1 = (rng.next_u64() % 6) as u32 + 1;
    let d2 = (rng.next_u64() % 6) as u32 + 1;
    d1 + d2
}

/// Pass line with `odds_multiple` in free odds taken behind the point.
///
/// Odds bet only pays true odds - no edge of its own.
/// It can only be placed once a point is established, so it dilutes the
/// blended edge (and doesn't change the pass-line math.)
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
    pub checkpoints: u32, // number of x-positions for the fan chart
    pub seed: u64,
    pub rounds_per_hour: f64, // independent of stuff, important for bacarat
}

pub struct TrialResult {
    /// `trials * checkpoints`, row-major: trial-major, checkpoint-minor
    pub paths: Vec<f64>,
    pub trials: u32,
    pub ruined: u32,
    /// Sum of every bet placed, double and split money included. The denominator
    /// for the element-of-risk edge (craps free odds live here).
    pub total_wagered: f64,
    /// Sum of the initial bet on each decision, before any double or split adds
    /// to it. The denominator for the per-initial-bet house edge, which is how
    /// blackjack and the other table games are published.
    pub total_staked: f64,
    pub total_net: f64,
    pub capped_by_table: u64,
    pub capped_by_bankroll: u64,
    /// `rounds / rounds_per_hour` so the chart axis and ledger
    /// cannot disagree.
    pub elapsed_hours: f64,
}

pub enum Round {
    /// This is a single decision (any thing you can imagine besides craps)
    Single(Generator),
    /// Geometric stopping time. Dice rolled directly.
    /// I want to see the individual die rolls, even though it
    /// is computed at once.
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

/// Builds a fresh round + policy per trial.
/// I made this a closure rather than two values because `Shoe` carries state.
/// Reusing trials would corelate them through the show.
/// The trials need to be independent for the p-bands to mean anything.

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
        // Every trial gets its own stream so they don't mix
        let mut rng = Rng::seed(cfg.seed ^ u64::from(trial).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let (mut round, mut policy) = make_trial();

        let mut bankroll = cfg.bankroll;
        let mut ruined = false;
        let mut next_c = 0usize;

        for r in 0..cfg.rounds {
            // Ruin is bankroll you can't cover the base bet.
            // So holding $5 at a $10 means you're ruined.
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

            // Absorbing ruin. Once `bankroll` stops moving everything is flat.
            // This just makes sure that there isn't a survivorship bias
            while next_c < checkpoints
                && (u64::from(r) + 1) * checkpoints as u64
                    >= (next_c as u64 + 1) * u64::from(cfg.rounds)
            {
                out.paths[trial as usize * checkpoints + next_c] = bankroll;
                next_c += 1;
            }
        }

        if ruined {
            out.ruined += 1;
        }
    }

    out
}
