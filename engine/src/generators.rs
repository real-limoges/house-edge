// generators.rs
//enum dispatch, not dyn Trait

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

impl IidTable {}

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
    pub fn play_hand(&mut self, rng: &mut Rng) -> Outcome {
        Outcome { net: 0.0 }
    }
}
