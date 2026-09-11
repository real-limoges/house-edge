//! generators.rs
//!
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
    /// it's `&mut self` because it gives a uniform signature
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
///
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

/// Blackjack Rules
#[derive(Clone, Copy, Debug)]
pub struct BjRules {
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

// Shoe: Stateful, sampling *without* replacement, reshuffles.
pub struct Shoe {
    cards: Vec<u8>,
    pos: usize,
    rules: BjRules, // resplits, 3:2 vs 6:5, etc...
    strategy: &'static StrategyTable,
    pub running_count: i32,
}

/// One player hand inside a decision. Tracked incrementally (no card list) so
/// the whole `[Option<Hand>; 4]` split tree is `Copy` and allocation-free.
///
/// `total` sums ranks with every ace as 1; `aces` lets `best` promote one to 11
/// when it fits. `resolved` and `busted` are distinct from `Option` presence: a
/// slot can be occupied and still mid-decision, and a resolved-but-busted hand
/// does not keep the dealer drawing (point 7) the way a resolved stand does.
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

    /// Best total <= 21 if possible, promoting one ace to 11.
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
    /// Build a fresh shoe for `rules`, shuffled once at construction. The deck
    /// is `52 * decks` cards: four each of ranks 1..=9 and **sixteen** of rank
    /// 10 per deck. That sixteen is the whole reason the game is countable.
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

    /// Next card off the top. No counting here; the caller counts a card when it
    /// becomes visible, which is not always when it is dealt (the hole card).
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

    /// Fold one now-visible card into the running count.
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
    /// One blackjack decision: deal, resolve naturals, play the player's hands
    /// (splits included) against basic strategy, play the dealer, and settle.
    /// Returns a single `Outcome` for the whole tree, `net` in base-bet units
    /// and `wagered` the sum of every bet placed. See the 10-point contract in
    /// `docs/reference/rust/03-implementing-the-engine-core.md`.
    pub fn play_hand(&mut self, rng: &mut Rng) -> Outcome {
        self.maybe_reshuffle(rng);
        debug_assert!(
            self.rules.max_hands <= 4,
            "split array assumes max_hands <= 4"
        );

        // Table order: player, dealer up, player, dealer hole.
        let p1 = self.draw_card();
        let up = self.draw_card();
        let p2 = self.draw_card();
        let hole = self.draw_card();
        // Only the three face-up cards are counted now; the hole waits.
        self.count(p1);
        self.count(up);
        self.count(p2);

        let dealer_bj = (up == 1 && hole == 10) || (up == 10 && hole == 1);
        let player_bj = (p1 == 1 && p2 == 10) || (p1 == 10 && p2 == 1);

        // The dealer peeks on a ten or an ace. A natural ends the decision with
        // exactly one unit at risk: no split, no double money goes up.
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
            // Dealer cannot have a natural here (peeked, or upcard is 2..9).
            self.count(hole);
            return Outcome {
                net: self.rules.blackjack_pays,
                wagered: 1.0,
            };
        }

        // Play the player's hands. A split leaves its current slot occupied and
        // writes the second child into a free slot, so a rescan loop is used
        // rather than a play cursor. Split always consumes a free slot and every
        // other action is terminal, which is what makes the loop terminate.
        let mut hands: [Option<Hand>; 4] = [None, None, None, None];
        hands[0] = Some(Hand::two(p1, p2, 1.0));
        let up_col = up_index(up);

        while let Some(i) = (0..4).find(|&i| matches!(hands[i], Some(h) if !h.resolved)) {
            let h = hands[i].unwrap();

            // Pair: split if the chart says so and a slot remains.
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
                        // Split aces take one card each and close: no resplit, no
                        // double, and a resulting 21 is not a natural (handled at
                        // settlement, since these hands never pay blackjack_pays).
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

        // The hole card is now revealed, whatever happens next.
        self.count(hole);

        // The dealer draws only if at least one player hand is still live.
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

        // Settle per hand: a busted hand loses its own bet whatever the dealer
        // does; otherwise compare totals, equal is a push.
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

    /// Basic-strategy action for a non-pair hand: soft chart when an ace can be
    /// eleven (soft 13..20), otherwise the hard chart on the best total. Soft 12
    /// (A,A played without a split slot) and soft 21 fall through to hard.
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

/// Strategy table
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Hit,
    Stand,
    // Double if two-card hand and doubling is *legal*, otherwise hit.
    // Every hard-total double carries this as the fallback.
    DoubleElseHit,
    // Double if legal. Only on soft 18/19. Only where a forbidden
    // double *must not* turn into a hit.
    DoubleElseStand,
}

/// I didn't put `split` in here, it is its own bool.
/// `split` doesn't fall through to hard/soft.
pub struct StrategyTable {
    /// Player hard total 4..=21. Index `total - 4`
    pub hard: [[Action; 10]; 18],
    /// Player soft total 13..=20. Index `total - 13`
    /// Soft 12 is A,A and unreachable. It needs to be split
    /// or played as a hard 12 if no slot remains.
    pub soft: [[Action; 10]; 8],
    /// Pairs ranked. A,A is `0`. `1..=9` are the other cards
    pub split: [[bool; 10]; 10],
}

/// Dealer upcard to column. 2..=9 maps to 0..=7.
/// Ten goes to 8, Ace goes to 9.
#[inline]
pub fn up_index(rank: u8) -> usize {
    match rank {
        1 => 9,
        10 => 8,
        r => (r - 2) as usize,
    }
}

/// The total-dependent basic-strategy chart for the locked rule set: six decks,
/// dealer hits soft 17, double any two, double after split, resplit to four,
/// split aces one card, no surrender. Transcribed from the Wizard of Odds
/// calculator (columns are dealer 2,3,4,5,6,7,8,9,10,A via `up_index`).
///
/// Validated only by the edge it reproduces, not by review: a wrong cell in a
/// cold part of the chart moves the edge too little to catch by eye. See the
/// reference tests.
pub fn locked_h17_strategy() -> &'static StrategyTable {
    static TABLE: OnceLock<StrategyTable> = OnceLock::new();
    TABLE.get_or_init(build_locked_h17)
}

fn build_locked_h17() -> StrategyTable {
    use Action::{DoubleElseHit as Dh, DoubleElseStand as Ds, Hit as H, Stand as S};

    // Player hard total 4..=21 (index total - 4). Columns 2..A.
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

    // Player soft total 13..=20, i.e. A,2 through A,9 (index total - 13).
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

    // Pairs, indexed by rank: 0 is A,A, 1..=9 are 2,2 through 10,10.
    let (t, f) = (true, false);
    let split = [
        [t, t, t, t, t, t, t, t, t, t], // A,A
        [t, t, t, t, t, t, f, f, f, f], // 2,2
        [t, t, t, t, t, t, f, f, f, f], // 3,3
        [f, f, f, t, t, f, f, f, f, f], // 4,4
        [f, f, f, f, f, f, f, f, f, f], // 5,5  (hard 10, never split)
        [t, t, t, t, t, f, f, f, f, f], // 6,6
        [t, t, t, t, t, t, f, f, f, f], // 7,7
        [t, t, t, t, t, t, t, t, t, t], // 8,8
        [t, t, t, t, t, f, t, t, f, f], // 9,9
        [f, f, f, f, f, f, f, f, f, f], // 10,10 (stand, never split)
    ];

    StrategyTable { hard, soft, split }
}

/// Parlay Stuff (Copula of a bunch of normals)
pub struct Copula {
    /// per-leg thresh on std norm, `probit(p_win)`.
    /// let wins when the draw falls below its threshold
    thresholds: Vec<f64>,
    rho: f64, // correlation
    sqrt_rho: f64,
    sqrt_one_minus_rho: f64,
    payout: f64, // Net paid per unit staked when *every* leg wins
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
///
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

/// Helper functions
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
