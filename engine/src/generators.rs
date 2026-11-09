//! Sets up all the games. I use Enum dispatch rather than
//! dyn Traits. I guess it's just a preference + the shape
//! of things.

use crate::rng::{probit, Rng};
use std::sync::OnceLock;

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
    /// Resolves one decision into a per-unit `Outcome`.
    #[inline]
    pub fn resolve(&mut self, rng: &mut Rng) -> Outcome {
        match self {
            Generator::Iid(t) => t.draw(rng),
            Generator::Shoe(s) => s.play_hand(rng),
            Generator::Correlated(c) => c.draw_bundle(rng),
        }
    }
}

/// Discrete table of net payouts per unit staked: cumulative probabilities in
/// `cum` and their payouts in `net`, heaviest outcome first.
pub struct IidTable {
    pub cum: Vec<f64>,
    pub net: Vec<f64>,
}

impl IidTable {
    /// From `(probability, net)` pairs. Panics on an empty table, a NaN or
    /// negative probability, or a sum off 1.0.
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
        // Linear scan: sorted heaviest first, so it beats a binary search.
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

    /// Exact expected net per unit staked.
    pub fn expected_net(&self) -> f64 {
        let mut prev = 0.0;
        let mut ev = 0.0;
        for (c, n) in self.cum.iter().zip(&self.net) {
            ev += (c - prev) * n;
            prev = *c;
        }
        ev
    }

    /// Exact per-round standard deviation of net.
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

/// Single number. Edge is `zeros / (36 + zeros)`.
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

#[derive(Clone, Copy, Debug)]
pub struct BjRules {
    pub decks: u32,
    pub penetration: f64, // fraction dealt before the cut card
    pub hit_soft_17: bool,
    pub double_after_split: bool,
    pub max_hands: u32, // including splits
    pub split_aces_one_card: bool,
    pub surrender: bool,     // unread: the chart assumes no surrender
    pub blackjack_pays: f64, // 1.5 at 3:2, 1.2 at 6:5
}

impl BjRules {
    pub fn locked(blackjack_pays: f64) -> Self {
        BjRules {
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

pub struct Shoe {
    cards: Vec<u8>,
    pos: usize,
    rules: BjRules,
    strategy: &'static StrategyTable,
    pub running_count: i32,
}

/// No card list, so the split tree is `Copy`. `total` counts aces as 1.
#[derive(Clone, Copy)]
struct Hand {
    total: u8,
    aces: u8,
    n: u8,
    r0: u8,
    r1: u8,
    bet: f64,
    resolved: bool,
    busted: bool,
    from_split: bool,
    split_ace: bool,
}

impl Hand {
    fn two(a: u8, b: u8, bet: f64) -> Self {
        let mut h = Hand {
            total: 0,
            aces: 0,
            n: 0,
            r0: 0,
            r1: 0,
            bet,
            resolved: false,
            busted: false,
            from_split: false,
            split_ace: false,
        };
        h.add(a);
        h.add(b);
        h
    }

    fn add(&mut self, r: u8) {
        match self.n {
            0 => self.r0 = r,
            1 => self.r1 = r,
            _ => {}
        }
        self.total += r;
        if r == 1 {
            self.aces += 1;
        }
        self.n += 1;
    }

    #[inline]
    fn best(&self) -> u8 {
        if self.aces > 0 && self.total + 10 <= 21 {
            self.total + 10
        } else {
            self.total
        }
    }

    #[inline]
    fn soft(&self) -> bool {
        self.aces > 0 && self.total + 10 <= 21
    }

    #[inline]
    fn is_pair(&self) -> bool {
        self.n == 2 && self.r0 == self.r1
    }
}

impl Shoe {
    /// Shuffled once. Per deck: four each of ranks 1..=9, sixteen tens.
    pub fn new(rules: BjRules, strategy: &'static StrategyTable, rng: &mut Rng) -> Self {
        let mut cards = Vec::with_capacity(52 * rules.decks as usize);
        for _ in 0..rules.decks {
            for rank in 1..=9u8 {
                for _ in 0..4 {
                    cards.push(rank);
                }
            }
            cards.extend(std::iter::repeat_n(10u8, 16));
        }
        let mut shoe = Shoe {
            cards,
            pos: 0,
            rules,
            strategy,
            running_count: 0,
        };
        shoe.shuffle(rng);
        shoe
    }

    /// Uncounted: the caller counts it once it is visible.
    #[inline]
    fn draw_card(&mut self) -> u8 {
        debug_assert!(
            self.pos < self.cards.len(),
            "dealt past the end of the shoe"
        );
        let c = self.cards[self.pos];
        self.pos += 1;
        c
    }

    #[inline]
    fn count(&mut self, rank: u8) {
        self.running_count += Self::hi_lo(rank);
    }

    #[inline]
    fn hi_lo(rank: u8) -> i32 {
        match rank {
            2..=6 => 1,
            1 | 10 => -1,
            _ => 0,
        }
    }

    #[inline]
    pub fn decks_remaining(&self) -> f64 {
        (self.cards.len() - self.pos) as f64 / 52.0
    }

    #[inline]
    pub fn true_count(&self) -> f64 {
        let d = self.decks_remaining();
        if d > 0.25 {
            f64::from(self.running_count) / d
        } else {
            0.0
        }
    }

    pub fn shuffle(&mut self, rng: &mut Rng) {
        for i in (1..self.cards.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            self.cards.swap(i, j);
        }
        self.pos = 0;
        self.running_count = 0;
    }

    fn maybe_reshuffle(&mut self, rng: &mut Rng) {
        if self.pos as f64 >= self.cards.len() as f64 * self.rules.penetration {
            self.shuffle(rng);
        }
    }

    /// One decision, splits included. `net` is in base-bet units; `wagered`
    /// sums every bet placed.
    pub fn play_hand(&mut self, rng: &mut Rng) -> Outcome {
        self.maybe_reshuffle(rng);
        debug_assert!(
            self.rules.max_hands <= 4,
            "split array assumes max_hands <= 4"
        );

        let p1 = self.draw_card();
        let up = self.draw_card();
        let p2 = self.draw_card();
        let hole = self.draw_card();
        // The hole card is counted when revealed.
        self.count(p1);
        self.count(up);
        self.count(p2);

        let dealer_bj = (up == 1 && hole == 10) || (up == 10 && hole == 1);
        let player_bj = (p1 == 1 && p2 == 10) || (p1 == 10 && p2 == 1);

        // Dealer peeks on a ten or ace; a natural ends it with one unit at risk.
        if (up == 1 || up == 10) && dealer_bj {
            self.count(hole);
            return if player_bj {
                Outcome {
                    net: 0.0,
                    wagered: 1.0,
                }
            } else {
                Outcome {
                    net: -1.0,
                    wagered: 1.0,
                }
            };
        }
        if player_bj {
            self.count(hole);
            return Outcome {
                net: self.rules.blackjack_pays,
                wagered: 1.0,
            };
        }

        // A rescan, not a cursor: a split writes its second hand into a free slot.
        let mut hands: [Option<Hand>; 4] = [None, None, None, None];
        hands[0] = Some(Hand::two(p1, p2, 1.0));
        let up_col = up_index(up);

        while let Some(i) = (0..4).find(|&i| matches!(hands[i], Some(h) if !h.resolved)) {
            let h = hands[i].unwrap();

            let free = (0..4).find(|&j| hands[j].is_none());
            let occupied = hands.iter().filter(|x| x.is_some()).count() as u32;
            if h.is_pair() && occupied < self.rules.max_hands {
                if let Some(j) = free {
                    let rank = h.r0;
                    let split_idx = if rank == 1 { 0 } else { (rank - 1) as usize };
                    if self.strategy.split[split_idx][up_col] {
                        let c_i = self.draw_card();
                        self.count(c_i);
                        let c_j = self.draw_card();
                        self.count(c_j);
                        let mut hi = Hand::two(rank, c_i, 1.0);
                        let mut hj = Hand::two(rank, c_j, 1.0);
                        hi.from_split = true;
                        hj.from_split = true;
                        // A 21 on split aces is not a natural: it pays even money.
                        if rank == 1 && self.rules.split_aces_one_card {
                            hi.split_ace = true;
                            hi.resolved = true;
                            hj.split_ace = true;
                            hj.resolved = true;
                        }
                        hands[i] = Some(hi);
                        hands[j] = Some(hj);
                        continue;
                    }
                }
            }

            let action = self.decide(&h, up_col);
            let can_double =
                h.n == 2 && !h.split_ace && (!h.from_split || self.rules.double_after_split);
            let mut hh = h;
            match action {
                Action::Stand => hh.resolved = true,
                Action::Hit => {
                    let c = self.draw_card();
                    self.count(c);
                    hh.add(c);
                    if hh.best() > 21 {
                        hh.busted = true;
                        hh.resolved = true;
                    }
                }
                Action::DoubleElseHit | Action::DoubleElseStand => {
                    if can_double {
                        let c = self.draw_card();
                        self.count(c);
                        hh.add(c);
                        hh.bet *= 2.0;
                        hh.busted = hh.best() > 21;
                        hh.resolved = true;
                    } else if matches!(action, Action::DoubleElseHit) {
                        let c = self.draw_card();
                        self.count(c);
                        hh.add(c);
                        if hh.best() > 21 {
                            hh.busted = true;
                            hh.resolved = true;
                        }
                    } else {
                        hh.resolved = true;
                    }
                }
            }
            hands[i] = Some(hh);
        }

        self.count(hole);

        let any_live = hands.iter().flatten().any(|h| !h.busted);
        let mut dealer_total = 0u8;
        if any_live {
            let mut d = Hand::two(up, hole, 0.0);
            loop {
                let b = d.best();
                let hit = b < 17 || (b == 17 && d.soft() && self.rules.hit_soft_17);
                if !hit {
                    break;
                }
                let c = self.draw_card();
                self.count(c);
                d.add(c);
                if d.best() > 21 {
                    break;
                }
            }
            dealer_total = d.best();
        }
        let dealer_bust = any_live && dealer_total > 21;

        let mut net = 0.0;
        let mut wagered = 0.0;
        for h in hands.iter().flatten() {
            wagered += h.bet;
            if h.busted {
                net -= h.bet;
            } else if dealer_bust {
                net += h.bet;
            } else {
                let pb = h.best();
                if pb > dealer_total {
                    net += h.bet;
                } else if pb < dealer_total {
                    net -= h.bet;
                }
            }
        }
        Outcome { net, wagered }
    }

    /// Soft 12 (unsplittable A,A) and soft 21 fall through to the hard chart.
    fn decide(&self, h: &Hand, up_col: usize) -> Action {
        let b = h.best();
        if h.soft() && (13..=20).contains(&b) {
            self.strategy.soft[(b - 13) as usize][up_col]
        } else if b >= 21 {
            Action::Stand
        } else {
            self.strategy.hard[(b.max(4) - 4) as usize][up_col]
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Hit,
    Stand,
    DoubleElseHit,
    DoubleElseStand, // soft 18/19, where a refused double must not hit
}

/// Columns by `up_index`. A `false` split cell falls through to `hard` or `soft`.
pub struct StrategyTable {
    pub hard: [[Action; 10]; 18], // total 4..=21
    pub soft: [[Action; 10]; 8],  // total 13..=20
    pub split: [[bool; 10]; 10],  // A,A at 0, then 2,2 through 10,10
}

/// Upcards 2..=9 to 0..=7, ten to 8, ace to 9.
#[inline]
pub fn up_index(rank: u8) -> usize {
    match rank {
        1 => 9,
        10 => 8,
        r => (r - 2) as usize,
    }
}

/// Basic strategy for `BjRules::locked`, transcribed from the Wizard of Odds
/// calculator.
pub fn locked_h17_strategy() -> &'static StrategyTable {
    static TABLE: OnceLock<StrategyTable> = OnceLock::new();
    TABLE.get_or_init(build_locked_h17)
}

fn build_locked_h17() -> StrategyTable {
    use Action::{DoubleElseHit as Dh, DoubleElseStand as Ds, Hit as H, Stand as S};

    // Columns: dealer 2..9, 10, A.
    let hard = [
        [H, H, H, H, H, H, H, H, H, H],           // 4
        [H, H, H, H, H, H, H, H, H, H],           // 5
        [H, H, H, H, H, H, H, H, H, H],           // 6
        [H, H, H, H, H, H, H, H, H, H],           // 7
        [H, H, H, H, H, H, H, H, H, H],           // 8
        [H, Dh, Dh, Dh, Dh, H, H, H, H, H],       // 9
        [Dh, Dh, Dh, Dh, Dh, Dh, Dh, Dh, H, H],   // 10
        [Dh, Dh, Dh, Dh, Dh, Dh, Dh, Dh, Dh, Dh], // 11
        [H, H, S, S, S, H, H, H, H, H],           // 12
        [S, S, S, S, S, H, H, H, H, H],           // 13
        [S, S, S, S, S, H, H, H, H, H],           // 14
        [S, S, S, S, S, H, H, H, H, H],           // 15
        [S, S, S, S, S, H, H, H, H, H],           // 16
        [S, S, S, S, S, S, S, S, S, S],           // 17
        [S, S, S, S, S, S, S, S, S, S],           // 18
        [S, S, S, S, S, S, S, S, S, S],           // 19
        [S, S, S, S, S, S, S, S, S, S],           // 20
        [S, S, S, S, S, S, S, S, S, S],           // 21
    ];

    let soft = [
        [H, H, H, Dh, Dh, H, H, H, H, H],    // 13 A,2
        [H, H, H, Dh, Dh, H, H, H, H, H],    // 14 A,3
        [H, H, Dh, Dh, Dh, H, H, H, H, H],   // 15 A,4
        [H, H, Dh, Dh, Dh, H, H, H, H, H],   // 16 A,5
        [H, Dh, Dh, Dh, Dh, H, H, H, H, H],  // 17 A,6
        [Ds, Ds, Ds, Ds, Ds, S, S, H, H, H], // 18 A,7
        [S, S, S, S, Ds, S, S, S, S, S],     // 19 A,8
        [S, S, S, S, S, S, S, S, S, S],      // 20 A,9
    ];

    let (t, f) = (true, false);
    let split = [
        [t, t, t, t, t, t, t, t, t, t], // A,A
        [t, t, t, t, t, t, f, f, f, f], // 2,2
        [t, t, t, t, t, t, f, f, f, f], // 3,3
        [f, f, f, t, t, f, f, f, f, f], // 4,4
        [f, f, f, f, f, f, f, f, f, f], // 5,5
        [t, t, t, t, t, f, f, f, f, f], // 6,6
        [t, t, t, t, t, t, f, f, f, f], // 7,7
        [t, t, t, t, t, t, t, t, t, t], // 8,8
        [t, t, t, t, t, f, t, t, f, f], // 9,9
        [f, f, f, f, f, f, f, f, f, f], // 10,10
    ];

    StrategyTable { hard, soft, split }
}

/// Parlay legs sharing one pairwise correlation (single-factor Gaussian copula).
pub struct Copula {
    thresholds: Vec<f64>, // probit(p_win) per leg
    rho: f64,
    sqrt_rho: f64,
    sqrt_one_minus_rho: f64,
    payout: f64, // net when every leg wins
}

impl Copula {
    /// Panics on no legs or `rho` outside `[0, 1)`.
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

    pub fn rho(&self) -> f64 {
        self.rho
    }
}

/// Eight decks, 5% commission; a tie pushes.
pub fn baccarat_banker() -> IidTable {
    IidTable::new(&[(0.458597, 0.95), (0.446247, -1.0), (0.095156, 0.0)])
}

pub fn baccarat_player() -> IidTable {
    IidTable::new(&[(0.446247, 1.0), (0.458597, -1.0), (0.095156, 0.0)])
}

/// Decimal odds include the stake.
pub fn american_to_decimal(odds: f64) -> f64 {
    if odds > 0.0 {
        1.0 + odds / 100.0
    } else {
        1.0 + 100.0 / -odds
    }
}

pub fn parlay_payout(leg_odds: &[f64]) -> f64 {
    leg_odds
        .iter()
        .map(|o| american_to_decimal(*o))
        .product::<f64>()
        - 1.0
}
