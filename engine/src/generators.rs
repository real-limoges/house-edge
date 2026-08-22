//! generators.rs
//!
//! Sets up all the games. I use Enum dispatch rather than
//! dyn Traits. I guess it's just a preference + the shape
//! of things.

use crate::rng::{probit, Rng};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outcome {
    pub net: f64,
    pub wagered: f64,
}

pub enum Generator {
    Iid(IidTable),      // no state
    Shoe(Shoe),         // no replacement, reshuffled occasionally
    Correlated(Copula), // bundle of binary legs decided at once
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

/// IidTable: Discrete probability net-payout table.
/// This powers certain games.

/// Arbitrary (not closed form) so weighted virtual reels and 0-inflated
/// near misses just fall out for free.
pub struct IidTable {
    pub cum: Vec<f64>,
    pub net: Vec<f64>,
}

impl IidTable {
    /// I sort the outcomes by desc prob, which makes the scan in
    /// `draw` kick out early. Big for slots.
    pub fn new(outcomes: &[(f64, f64)]) -> Self {
        assert!(!outcomes.is_empty(), "empty outcome table");
        let mut v = outcomes.to_vec();
        v.sort_by(|a, b| b.0.partial_cmp(&a.0).expect("NaN probability"));

        let total: f64 = v.iter().map(|(p, _)| p).sum();
        assert!(
            (total - 1.0).abs() < 1e-9,
            "probabilities sum to {total}, not 1.0"
        );

        let mut cum = Vec::with_capacity(v.len());
        let mut acc = 0.0;
        for (p, _) in &v {
            assert!(*p >= 0.0, "negative probability");
            acc += p;
            cum.push(acc);
        }
        // Guard floating point shortfall
        *cum.last_mut().unwrap() = 1.0;

        IidTable {
            cum,
            net: v.into_iter().map(|(_, n)| n).collect(),
        }
    }

    #[inline]
    pub fn draw(&self, rng: &mut Rng) -> Outcome {
        // linear, not binary. sorted heaviest first
        // the branch predictor should beat a binary search
        let u = rng.uniform();
        let mut i = 0;
        while i + 1 < self.cum.len() && u >= self.cum[i] {
            i += 1;
        }
        Outcome {
            net: self.net[i],
            wagered: 1.0,
        }
    }

    /// Exact expect net (no sampling, no tolerance)
    /// These are deterministic so the table tests assert against this
    pub fn expected_net(&self) -> f64 {
        let mut prev = 0.0;
        let mut ev = 0.0;
        for (c, n) in self.cum.iter().zip(&self.net) {
            ev += (c - prev) * n;
            prev = *c;
        }
        ev
    }

    /// Exact per-round standard deviation of net. Tests and minimum-N floors
    /// read the spread from the table instead of hardcoding it.
    pub fn sd(&self) -> f64 {
        let mean = self.expected_net();
        let mut prev = 0.0;
        let mut var = 0.0;
        for (c, n) in self.cum.iter().zip(&self.net) {
            var += (c - prev) * (n - mean).powi(2);
            prev = *c;
        }
        var.sqrt()
    }
}

// Shoe: Stateful, sampling *without* replacement, reshuffles.
pub struct Shoe {
    cards: Vec<u8>,
    pos: usize,
    rules: BJRules, // resplits, 3:2 vs 6:5, etc...
    strategy: &'static StrategyTable,
    pub running_count: i32,
}

impl Shoe {
    #[inline]
    fn hi_lo(rank: u8) -> i32 {
        match rank {
            2..=6 => 1,
            1 | 10 => -1,
            _ => 0,
        }
    }

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
    pub fn play_hand(&mut self, rng: &mut Rng) -> Outcome {
        Outcome { net: 0.0 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BJRules {
    pub decks: u32,
    pub penetration: f64,  // fraction of shoe before the cut card
    pub hit_soft_17: bool, // what most people have done, costs players 0.22%
    pub double_after_split: bool,
    pub max_hands: u32,            // All the hands allowed at once + splits
    pub split_aces_one_card: bool, // split aces get one card each and then closed.
    /// If this is turned on, then we need a new chart, not a different code path
    pub surrender: bool,
    pub blackjack_pays: f64, // Net paid per unit on a natural. (1.5 at 3:2, 1.2 at 6:5)
}

impl BJRules {
    pub fn locked(blackjack_pays: f64) -> Self {
        BJRules {
            decks: 6,
            penetration: 0.75,
            hit_soft_17: true,
            double_after_split: true,
            max_hands: 4,
            split_aces_one_card: true,
            surrender: false,
            blackjack_pays,
        }
    }
}

/// Single number. 36 pockets + # of zeros.
/// The edge falls out as `zeros / (36 + zeros)`
pub fn roulette_straight_up(zeros: u32) -> IidTable {
    let pockets = f64::from(36 + zeros);
    IidTable::new(&[(1.0 / pockets, 35.0), (1.0 - 1.0 / pockets, -1.0)])
}

pub fn roulette_even_money(zeros: u32) -> IidTable {
    let pockets = f64::from(36 + zeros);
    let win = 18.0 / pockets;
    IidTable::new(&[(win, 1.0), (1.0 - win, -1.0)])
}

/// Craps any-seven prop. Pays 4:1 on a 1 in 6 event.
pub fn any_seven() -> IidTable {
    IidTable::new(&[(1.0 / 6.0, 4.0), (5.0 / 6.0, -1.0)])
}

/// Parlay Stuff (Copula of a bunch of normals)

pub struct Copula {
    thresholds: Vec<f64>,
    rho: f64,
    sqrt_rho: f64,
    sqrt_one_minus_rho: f64,
    payout: f64,
}

impl Copula {
    /// `legs` are per-leg win probs.
    pub fn new(legs: &[f64], rho: f64, payout: f64) -> Self {
        assert!(!legs.is_empty(), "parlay with no legs");
        assert!((0.0..1.0).contains(&rho), "rho {rho} is outside [0, 1)");
        Copula {
            thresholds: legs.iter().map(|p| probit(*p)).collect(),
            rho,
            sqrt_rho: rho.sqrt(),
            sqrt_one_minus_rho: (1.0 - rho).sqrt(),
            payout,
        }
    }

    /// All legs or nothing. The parlay is a single decision staking 1 unit,
    /// but it has many legs, which is why the bundle collapses to a single
    /// `Outcome`.
    #[inline]
    pub fn draw_bundle(&self, rng: &mut Rng) -> Outcome {
        let f = rng.normal();
        let common = self.sqrt_rho * f;
        for t in &self.thresholds {
            let z = common + self.sqrt_one_minus_rho * rng.normal();
            if z >= *t {
                return Outcome {
                    net: -1.0,
                    wagered: 1.0,
                };
            }
        }
        Outcome {
            net: self.payout,
            wagered: 1.0,
        }
    }

    /// Controls how correlated the legs are.
    pub fn rho(&self) -> f64 {
        self.rho
    }
}

/// Baccarat Stuff

/// Banker and player with 8 decks.
/// Ties push so `net` is 0.0 but `wagered` is 1.0
/// Banker wins pay 0.95 after 5% commission. This is why the banker's edge
/// is lower than its win rate.
///
/// This shows that arbitrarily discrete design choices works.
pub fn baccarat_banker() -> IidTable {
    IidTable::new(&[(0.458597, 0.95), (0.446247, -1.0), (0.095156, 0.0)])
}

pub fn baccarat_player() -> IidTable {
    IidTable::new(&[(0.446247, 1.0), (0.458597, -1.0), (0.095156, 0.0)])
}
