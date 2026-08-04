// generators.rs
//enum dispatch, not dyn Trait

use crate::rng::Rng;

pub struct Outcome {
    pub net: f64,
}

pub enum Generator {
    Iid(IidTable),
    Shoe(Shoe),
    Correlated(Copula),
}

impl Generator {
    #[inline]
    pub fn resolve(&mut self, rng: &mut Rng) -> Outcome {
        match self {
            Generator::Iid(t) => t.draw(rng),
            Generator::Shoe(s) => s.play_hand(rng),
            Generator::Correlated(c) => c.draw_bundle(rng),
        }
    }
}

/// IID: Discrete probability net-payout table.
/// Arbitrary (not closed form) so weighted virtual reels and 0-inflated
/// near misses just fall out for free.
pub struct IidTable {
    pub cum: Vec<f64>,
    pub net: Vec<f64>,
}

impl IidTable {
    pub fn new(entries: &[(f64, f64)]) -> Self {
        let mut cum = Vec::with_capacity(entries.len());
        let mut acc = 0.0;
        for (p, _) in entries {
            acc += *p;
            cum.push(acc);
        }
        debug_assert!((acc - 1.0).abs() < 1e-9, "probabilities must sum to 1");
        IidTable {
            cum,
            net: entries.iter().map(|e| e.1).collect(),
        }
    }
    #[inline]
    pub fn draw(&self, rng: &mut Rng) -> Outcome {
        let u = rng.uniform();
        let i = self
            .cum
            .iter()
            .position(|&c| u < c)
            .unwrap_or(self.net.len() - 1);
        Outcome { net: self.net[i] }
    }
}

/// Basic-strategy lookup (hard/soft/pair decisions). Not built yet.
pub struct StrategyTable;

/// House rules that shift EV: resplits, 3:2 vs 6:5, dealer hits soft 17, etc.
pub struct BjRules;

/// Shoe: Stateful, sampling *without* replacement, reshuffles.
/// It should make within-shoe runs.
pub struct Shoe {
    decks: u32,
    cards: Vec<u8>,
    pos: usize,
    penetration: f64,
    pub running_count: i32, // Hi-Lo: +1 for 2..6; -1 for 10..A; 0 otherwise.
    strategy: &'static StrategyTable,
    rules: BjRules, // resplits, 3:2 vs 6:5, etc...
}

impl Shoe {
    #[inline]
    pub fn true_count(&self) -> f64 {
        let dealt = self.pos as f64;
        let decks_left = (self.decks as f64) - dealt / 52.0;
        if decks_left > 0.25 {
            self.running_count as f64 / decks_left
        } else {
            0.0
        }
    }
    fn maybe_rehuffle(&mut self, rng: &mut Rng) {
        if self.pos as f64 / self.cards.len() as f64 >= self.penetration {
            self.shuffle(rng);
            self.pos = 0;
            self.running_count = 0;
        }
    }
    /// Fisher–Yates over `self.cards`. Not built yet.
    fn shuffle(&mut self, _rng: &mut Rng) {}
    pub fn play_hand(&mut self, rng: &mut Rng) -> Outcome {
        Outcome { net: 0.0 }
    }
}

/// Copula: It draws a joint vector of leg outcomes for parlays. Uses Gaussian Copula.a
/// So you draw the correlated normals, push each through the legs marginal via the inverse CDF
/// and threshold to say win or loss. rho is a slider that that turns the proprietary correlations
/// into an uncertainty users can control.
pub struct Copula {
    pub rho: f64,
    pub legs: usize,
    pub p_win: f64,
    pub payout: f64,
}

impl Copula {
    /// equi-correlated one-factor model: z_i = sqrt(rho)*f + sqrt(1-rho)*e_i
    /// its easier this way
    pub fn draw_bundle(&mut self, rng: &mut Rng) -> Outcome {
        let f = rng.normal();
        let (a, b) = (self.rho.max(0.0).sqrt(), (1.0 - self.rho.max(0.0)).sqrt());
        let thresh = inv_norm_cdf(self.p_win);
        let all_win = (0..self.legs).all(|_| {
            let z = a * f + b * rng.normal();
            z < thresh
        });
        Outcome {
            net: if all_win { self.payout } else { -1.0 },
        }
    }
}

// Rational approx to inverse normal CDF. Not built.
pub fn inv_norm_cdf(p: f64) -> f64 {
    0.0
}
