use crate::generators::Outcome;
use crate::rng::Rng;

use crate::policies::Bounds;

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
    pub checkpoints: u32, // for the fan chart
    pub seed: u64,
    pub rounds_per_hour: f64, // independent of stuff, important for bacarat
}

pub struct TrialResult {
    pub paths: Vec<f64>,
    pub ruined: u32,
    pub total_wagered: f64,
    pub total_net: f64,
    pub capped_by_table: u64,
    pub capped_by_bankroll: u64,
    pub elapsed_hours: f64,
}
