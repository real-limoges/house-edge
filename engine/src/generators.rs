// generators.rs
//enum dispatch, not dyn Trait

use crate::rng::Rng;

pub struct Outcome {
    pub net: f64,
    pub wagered: f64,
}

// pub enum Generator {
//     Iid(IidTable),
//     Shoe(Shoe),
//     Correlated(Copula),
// }

// impl Generator {
//     #[inline]
//     pub fn resolve(&mut self, rng: &mut Rng) -> Outcome {
//         match self {
//             Generator::Iid(t) => t.draw(rng),
//             Generator::Shoe(s) => s.play_hand(rng),
//             Generator::Correlated(c) => c.draw_bundle(rng),
//         }
//     }
// }

/// IID: Discrete probability net-payout table.
/// Arbitrary (not closed form) so weighted virtual reels and 0-inflated
/// near misses just fall out for free.
pub struct IidTable {
    pub cum: Vec<f64>,
    pub net: Vec<f64>,
}

impl IidTable {
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
}

// Shoe: Stateful, sampling *without* replacement, reshuffles.
// It should make within-shoe runs.
// pub struct Shoe {
//     decks: u32,
//     cards: Vec<u8>,
//     pos: usize,
//     penetration: f64,
//     pub running_count: i32, // Hi-Lo: +1 for 2..6; -1 for 10..A; 0 otherwise.
//     strategy: &'static StrategyTable,
//     rules: BjRules, // resplits, 3:2 vs 6:5, etc...
// }

// impl Shoe {
//     #[inline]
//     pub fn true_count(&self) -> f64 {
//         let dealt = self.pos as f64;
//         let decks_left = (self.decks as f64) - dealt / 52.0;
//         if decks_left > 0.25 {
//             self.running_count as f64 / decks_left
//         } else {
//             0.0
//         }
//     }
//     fn maybe_rehuffle(&mut self, rng: &mut Rng) {
//         if self.pos as f64 / self.cards.len() as f64 >= self.penetration {
//             self.shuffle(rng);
//             self.pos = 0;
//             self.running_count = 0;
//         }
//     }
//     pub fn play_hand(&mut self, rng: &mut Rng) -> Outcome {
//         Outcome { net: 0.0 }
//     }
// }
