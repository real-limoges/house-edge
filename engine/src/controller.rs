// controller.rs

use crate::generators::{Generator, Outcome};
use crate::rng::Rng;

pub enum Round {
    Single,  // passthrough
    CrapsPassLine,  // geometric stopping
    Parlay,  // fixed correlated bundle
}

impl Round {
    pub fn resolve(&self, gen: &mut Generator, rng: &mut Rng) -> Outcome {
        match self {
            Round::Single | Round::Parlay => gen.resolve(rng),
            Round::CrapsPassLine => craps_pass_line(rng),
        }
    }
}

/// Pass line: come-out roll wins on 7/11, loses on 2/3/12; otherwise the total
/// becomes the point and we roll until it repeats (win) or a 7 lands (loss).
/// Net is +1 (win) or -1 (loss) per resolved round — the ~1.41% edge falls out.
fn craps_pass_line(rng: &mut Rng) -> Outcome {
    fn die(rng: &mut Rng) -> u32 {
        1 + (rng.uniform() * 6.0) as u32
    }
    fn roll(rng: &mut Rng) -> u32 {
        die(rng) + die(rng)
    }
    let net = match roll(rng) {
        7 | 11 => 1.0,
        2 | 3 | 12 => -1.0,
        point => loop {
            match roll(rng) {
                r if r == point => break 1.0,
                7 => break -1.0,
                _ => {}
            }
        },
    };
    Outcome { net }
}
