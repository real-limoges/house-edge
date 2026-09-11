//! Reference tests for the engine core.
//!
//! The split is the point ([`docs/reference/rust/02-implementing-the-engine-core.md`]):
//! the table is tested **exactly**, the sampler is tested **statistically**, and
//! never both at once. The exact tests carry the correctness claim; the
//! statistical ones only prove a sampler is faithful to the table it was handed.
//!
//! Every tolerance on a sampled quantity is derived from N and the table's own
//! spread. A hardcoded epsilon on a Monte Carlo assertion is either flaky or
//! vacuous, and which one it is changes with the seed.

use house_edge_engine::controller::{pass_line, run_trials, Round, TrialConfig};
use house_edge_engine::generators::{
    any_seven, baccarat_banker, baccarat_player, locked_h17_strategy, roulette_even_money,
    roulette_straight_up, BjRules, Generator, IidTable, Shoe,
};
use house_edge_engine::policies::{Bounds, Cap, Policy};
use house_edge_engine::rng::Rng;
use house_edge_engine::stats::{house_edge, realized_edge};

/// Blackjack house edge PER INITIAL BET (the published convention; the element
/// of risk on total wager is a different, smaller number) plus its standard
/// error, over `n` hands of one shoe at the given penetration and payout.
fn bj_edge(pen: f64, bj_pays: f64, n: u64, seed: u64) -> (f64, f64) {
    let mut rng = Rng::seed(seed);
    let mut rules = BjRules::locked(bj_pays);
    rules.penetration = pen;
    let mut shoe = Shoe::new(rules, locked_h17_strategy(), &mut rng);
    let (mut net, mut sq) = (0.0f64, 0.0f64);
    for _ in 0..n {
        let o = shoe.play_hand(&mut rng);
        net += o.net;
        sq += o.net * o.net;
    }
    let nf = n as f64;
    let mean = net / nf;
    let sd = (sq / nf - mean * mean).sqrt();
    (-mean, sd / nf.sqrt())
}

/// Exact: no RNG, no tolerance, microseconds. Catches a wrong paytable, a wrong
/// pocket count, a transposed probability. This is the test that protects the
/// site's credibility, and it samples nothing.
#[test]
fn published_house_edges() {
    // (label, table, published edge as a fraction)
    let cases: &[(&str, IidTable, f64)] = &[
        (
            "roulette straight-up, single zero",
            roulette_straight_up(1),
            0.0270,
        ),
        (
            "roulette straight-up, double zero",
            roulette_straight_up(2),
            0.0526,
        ),
        (
            "roulette straight-up, triple zero",
            roulette_straight_up(3),
            0.0769,
        ),
        (
            "roulette straight-up, quadruple zero",
            roulette_straight_up(4),
            0.1000,
        ),
        (
            "roulette even-money, single zero",
            roulette_even_money(1),
            0.0270,
        ),
        (
            "roulette even-money, double zero",
            roulette_even_money(2),
            0.0526,
        ),
        ("craps any seven", any_seven(), 0.1667),
        // Full precision on purpose: the familiar 1.06% and 1.24% are rounded,
        // and player at 0.0124 sits exactly on the 5e-5 tolerance.
        (
            "baccarat banker, ties included",
            baccarat_banker(),
            0.010579,
        ),
        (
            "baccarat player, ties included",
            baccarat_player(),
            0.012351,
        ),
    ];
    for (label, table, published) in cases {
        let edge = -table.expected_net();
        assert!(
            (edge - published).abs() < 5e-5,
            "{label}: computed {edge:.6}, published {published:.4}"
        );
    }
}

/// The ties-included convention, pinned rather than assumed. Quoted per bet
/// *decided* (ties excluded) banker is about 1.17%, so which denominator the
/// tables use has to be a test and not a comment.
#[test]
fn baccarat_uses_the_ties_included_denominator() {
    let ties = 0.095156;
    for (label, table, decided) in [
        ("banker", baccarat_banker(), 0.011692),
        ("player", baccarat_player(), 0.013650),
    ] {
        let included = -table.expected_net();
        let excluded = included / (1.0 - ties);
        assert!(
            (excluded - decided).abs() < 5e-5,
            "{label}: per-decision edge {excluded:.6}, expected {decided:.6}"
        );
        assert!(
            included < excluded,
            "{label}: ties-included edge must be the smaller of the two"
        );
    }
}

/// Exact: the spread the minimum-N arithmetic is derived from
/// ([`docs/decisions/0013-minimum-n-is-derived.md`]). Same edge on both rows,
/// 33x the trials, because variance scales with the square of the payout.
#[test]
fn per_round_spread_matches_the_derivation() {
    let even = roulette_even_money(2);
    let straight = roulette_straight_up(2);

    assert!(
        (even.sd() - 0.9986).abs() < 5e-4,
        "even-money sd {:.4}, expected 0.9986",
        even.sd()
    );
    assert!(
        (straight.sd() - 5.7626).abs() < 5e-4,
        "straight-up sd {:.4}, expected 5.7626",
        straight.sd()
    );

    // N for +/- 0.05pp at 4 se, i.e. (4 * sd / 5e-4)^2.
    let n = |sd: f64| (4.0 * sd / 5e-4).powi(2);
    assert!((n(even.sd()) / 6.4e7 - 1.0).abs() < 0.05);
    assert!((n(straight.sd()) / 2.1e9 - 1.0).abs() < 0.05);
}

/// Statistical: asserts the sampler is unbiased, with a tolerance derived from
/// the table's own variance rather than guessed.
#[test]
fn sampler_reproduces_its_table() {
    const N: u32 = 4_000_000;
    let table = roulette_even_money(2);
    let mut rng = Rng::seed(0xC0FFEE); // fixed: a failure must reproduce

    let mut sum = 0.0;
    for _ in 0..N {
        sum += table.draw(&mut rng).net;
    }
    let mean = sum / f64::from(N);

    let se = table.sd() / f64::from(N).sqrt();
    assert!(
        (mean - table.expected_net()).abs() < 4.0 * se,
        "mean {mean:.6} is more than 4 se from {:.6}",
        table.expected_net()
    );
}

/// Statistical: a heavy-tailed table needs its own check, because the linear
/// scan terminating early on the common case is exactly where a sampler bug
/// would hide. Tolerance again from the table's sd, which is 5.76 here.
#[test]
fn sampler_reproduces_a_heavy_tailed_table() {
    const N: u32 = 4_000_000;
    let table = roulette_straight_up(2);
    let mut rng = Rng::seed(0x51075);

    let mut sum = 0.0;
    for _ in 0..N {
        sum += table.draw(&mut rng).net;
    }
    let mean = sum / f64::from(N);

    let se = table.sd() / f64::from(N).sqrt();
    assert!(
        (mean - table.expected_net()).abs() < 4.0 * se,
        "mean {mean:.6} is more than 4 se from {:.6}",
        table.expected_net()
    );
}

/// Every draw is a resolved bet of exactly one unit, ties included. The ledger's
/// third line divides by this, so a generator that quietly stopped reporting
/// wager would show up as a wrong cost per hour and nowhere else.
#[test]
fn every_draw_wagers_one_unit() {
    let mut rng = Rng::seed(0xBAC0);
    let table = baccarat_banker();
    for _ in 0..10_000 {
        let o = table.draw(&mut rng);
        assert_eq!(o.wagered, 1.0);
        assert!(
            o.net == 0.95 || o.net == -1.0 || o.net == 0.0,
            "net {}",
            o.net
        );
    }
}

/// Exact: the reveal ADR 0007 exists to show, asserted rather than assumed. If
/// someone retunes the defaults so the table max binds first, this fails and
/// points at the decision record.
#[test]
fn bankroll_binds_before_the_table_at_locked_defaults() {
    let bounds = Bounds {
        base: 10.0,
        table_max: 1000.0,
    };
    let mut p = Policy::Martingale { streak: 0 };
    let mut bankroll = 500.0;
    let mut caps = Vec::new();

    for _ in 0..8 {
        let bet = p.next_bet(bankroll, &bounds);
        caps.push(bet.cap);
        bankroll -= bet.amount;
        if bankroll <= 0.0 {
            break;
        }
        p.update(false); // an unbroken losing streak, which is the scenario
    }

    // $10, 20, 40, 80, 160 clears the bankroll; the sixth bet is truncated.
    assert_eq!(caps.iter().position(|c| *c == Cap::Bankroll), Some(5));
    assert!(!caps.contains(&Cap::Table));
}

/// The clamp's ordering is the load-bearing part: when both bounds bite, the one
/// reported is the one that binds *first* as the bet grows, which is the table
/// max only when the table max is itself affordable.
#[test]
fn clamp_reports_the_bound_that_binds_first() {
    let bounds = Bounds {
        base: 10.0,
        table_max: 100.0,
    };
    let p = Policy::Martingale { streak: 5 }; // wants $320

    // Table max is affordable, so the house rule is what the player hits.
    let bet = p.next_bet(1_000.0, &bounds);
    assert_eq!(bet.cap, Cap::Table);
    assert_eq!(bet.amount, 100.0);

    // Table max is not affordable: the wallet failed before the house rule did.
    let bet = p.next_bet(60.0, &bounds);
    assert_eq!(bet.cap, Cap::Bankroll);
    assert_eq!(bet.amount, 60.0);

    // Neither bites.
    let bet = Policy::Flat.next_bet(1_000.0, &bounds);
    assert_eq!(bet.cap, Cap::None);
    assert_eq!(bet.amount, 10.0);
}

/// Exact: each policy's sequence over a fixed win/loss script. State plus
/// update, no rescanning of history, so the whole claim is checkable by hand.
#[test]
fn policies_follow_their_progressions() {
    let bounds = Bounds {
        base: 10.0,
        table_max: f64::INFINITY,
    };
    // L L L W W L: long enough to exercise Fibonacci's step-back-two on a win
    // and to prove D'Alembert steps back one over the same script.
    let script = [false, false, false, true, true, false];

    let cases: [(&str, Policy, [f64; 7]); 4] = [
        ("flat", Policy::Flat, [10.0; 7]),
        (
            "martingale",
            Policy::Martingale { streak: 0 },
            [10.0, 20.0, 40.0, 80.0, 10.0, 10.0, 20.0],
        ),
        (
            // fib(n) is 1, 1, 2, 3, 5, 8 ...; a win steps the index back two,
            // and it saturates at zero rather than wrapping.
            "fibonacci",
            Policy::Fibonacci { idx: 0 },
            [10.0, 10.0, 20.0, 30.0, 10.0, 10.0, 10.0],
        ),
        (
            "d'alembert",
            Policy::DAlembert { offset: 0 },
            [10.0, 20.0, 30.0, 40.0, 30.0, 20.0, 30.0],
        ),
    ];

    for (label, mut policy, expected) in cases {
        for (i, want) in expected.iter().enumerate() {
            let bet = policy.next_bet(f64::INFINITY, &bounds);
            assert_eq!(bet.amount, *want, "{label}: bet {i} was {}", bet.amount);
            assert_eq!(bet.cap, Cap::None, "{label}: bet {i} was capped");
            if let Some(won) = script.get(i) {
                policy.update(*won);
            }
        }
    }
}

/// Statistical, and the first thing that exercises the round controller: the
/// bare pass line with no odds behind it, against the published 1.41%.
#[test]
fn pass_line_matches_published_edge() {
    const N: u32 = 20_000_000;
    let mut rng = Rng::seed(0x5EED);
    let (mut net, mut wagered) = (0.0, 0.0);
    for _ in 0..N {
        let o = pass_line(&mut rng, 0.0); // no odds: the bare 1.41% claim
        net += o.net;
        wagered += o.wagered;
    }
    let edge = -net / wagered;
    // Pass-line net is +/-1, so per-round sd is ~1.0 and se is 1/sqrt(N).
    // Derive the tolerance; do not pick one that looks tight.
    let se = 1.0 / f64::from(N).sqrt();
    assert!(
        (edge - 0.0141414).abs() < 4.0 * se,
        "pass line edge {edge:.6}, {:.2} se from 0.0141414",
        (edge - 0.0141414).abs() / se
    );
}

/// Statistical: free odds pay true odds, so they carry no edge of their own and
/// dilute the blended figure on total wager. 5x odds is the published 0.326%.
///
/// The tolerance is derived, and the sd here is *not* 1.0: a 5x-odds decision
/// swings up to +/-6 units, so the spread is measured from the sample rather
/// than assumed, and the assertion is against the standard error that implies.
#[test]
fn free_odds_dilute_the_blended_edge() {
    const N: u32 = 20_000_000;
    let mut rng = Rng::seed(0x0DD5);
    let (mut net, mut wagered, mut sq) = (0.0, 0.0, 0.0);
    for _ in 0..N {
        let o = pass_line(&mut rng, 5.0);
        net += o.net;
        wagered += o.wagered;
        sq += o.net * o.net;
    }
    let n = f64::from(N);
    let mean = net / n;
    let sd = (sq / n - mean * mean).sqrt();
    // The edge is per unit of total wager, so the se converts with it.
    let se = sd / n.sqrt() / (wagered / n);
    let edge = -net / wagered;
    assert!(
        (edge - 0.00326).abs() < 4.0 * se,
        "5x odds blended edge {edge:.6}, {:.2} se from 0.00326",
        (edge - 0.00326).abs() / se
    );
    // Odds are free money in the edge sense: they can only dilute.
    assert!(
        edge < 0.0141414,
        "odds raised the blended edge to {edge:.6}"
    );
}

/// Exact-ish and cheap: the controller's shape, not its edge. Come-out sevens
/// and craps resolve in one roll at 1 unit; a point decision wagers the odds too
/// and pays true odds on the point. Asserting the *pairs* catches a controller
/// that got the right net with the wrong denominator, which is the failure the
/// blended figure would otherwise hide.
#[test]
fn pass_line_outcomes_are_well_formed() {
    let mut rng = Rng::seed(0x1CE);
    for _ in 0..200_000 {
        let o = pass_line(&mut rng, 2.0);
        assert!(
            o.wagered == 1.0 || o.wagered == 3.0,
            "wagered {}",
            o.wagered
        );
        if o.wagered == 1.0 {
            // Come-out decision: no odds were ever placed.
            assert!(o.net == 1.0 || o.net == -1.0, "come-out net {}", o.net);
        } else {
            // Point decision: lose the lot, or win 1 plus true odds on 2 units.
            // True odds are 2/1, 3/2 and 6/5, so the wins are 5, 4 and 3.4.
            assert!(
                o.net == -3.0 || o.net == 5.0 || o.net == 4.0 || (o.net - 3.4).abs() < 1e-12,
                "point net {}",
                o.net
            );
        }
    }
}

/// The generator sees no bet size, so scaling the stake scales net and wager
/// linearly and leaves the edge untouched. That invariant is what lets the trial
/// loop own bet sizing and the policies own progression, with neither able to
/// change a house edge by accident.
#[test]
fn edge_is_invariant_to_stake() {
    let table = roulette_even_money(2);
    let mut a = Rng::seed(0x5CA1E);
    let mut b = Rng::seed(0x5CA1E);

    let (mut net_a, mut wag_a) = (0.0, 0.0);
    let (mut net_b, mut wag_b) = (0.0, 0.0);
    for _ in 0..100_000 {
        let o = table.draw(&mut a);
        net_a += o.net;
        wag_a += o.wagered;

        let o = table.draw(&mut b);
        net_b += 25.0 * o.net;
        wag_b += 25.0 * o.wagered;
    }
    assert!(((-net_a / wag_a) - (-net_b / wag_b)).abs() < 1e-12);
}

/// A fixed seed must give a fixed stream, or every statistical test above is
/// unreproducible and the site's numbers cannot be regenerated from a config.
#[test]
fn seeding_is_deterministic() {
    let draw = |seed| {
        let mut rng = Rng::seed(seed);
        (0..1_000).map(|_| rng.next_u64()).collect::<Vec<_>>()
    };
    assert_eq!(draw(0x5EED), draw(0x5EED));
    assert_ne!(draw(0x5EED), draw(0x5EEE));

    // Uniforms stay in [0, 1), which the linear scan in `draw` relies on: a
    // uniform of exactly 1.0 would walk past the last cumulative entry.
    let mut rng = Rng::seed(1);
    for _ in 0..1_000_000 {
        let u = rng.uniform();
        assert!((0.0..1.0).contains(&u), "uniform out of range: {u}");
    }
}

/// Statistical, and the whole blackjack correctness claim: the transcribed
/// basic-strategy chart reproduces the Wizard of Odds figures for the locked
/// rule set. Everything is PER INITIAL BET, which is how blackjack edges are
/// published; the element of risk on total wager is ~0.56% and is a different
/// number (see `run_trials_drives_a_blackjack_shoe`).
///
/// Continuous shuffle (penetration 0.0, reshuffle every hand) is the clean check,
/// because it is independent of the cut-card penetration assumption. Closing Q7:
/// these WoO figures are the source the composed 0.63% / 2.02% were checked
/// against; the engine models total-dependent basic strategy with a cut card, so
/// it reproduces the "cut card" basis, and the printed ratio is 3.13x, not the
/// 3.21x of the composition-dependent basis.
#[test]
fn blackjack_reproduces_wizard_of_odds() {
    const N: u64 = 30_000_000;

    // WoO "continuous shuffler", 6-deck H17: 0.61873%. Penetration-independent.
    let (csm, se) = bj_edge(0.0, 1.5, N, 0x0B1A_CC51);
    assert!(
        (csm - 0.0061873).abs() < 4.0 * se,
        "CSM 3:2 house edge {csm:.6}, {:.1} se from 0.0061873",
        (csm - 0.0061873).abs() / se
    );

    // The shipped cut-card game (penetration 0.75). WoO: 3:2 0.63873%, 6:5
    // 1.99842%. 3:2 and 6:5 share one seed, so they deal identical hands and
    // their difference is 0.3 * the paid-natural rate with almost no sampling
    // noise, pinning the natural/payout handling a single edge cannot.
    let (e32, se32) = bj_edge(0.75, 1.5, N, 0x5EED_BEEF);
    let (e65, se65) = bj_edge(0.75, 1.2, N, 0x5EED_BEEF);
    assert!(
        (e32 - 0.0063873).abs() < 4.0 * se32,
        "cut-card 3:2 house edge {e32:.6}, {:.1} se from 0.0063873 (WoO)",
        (e32 - 0.0063873).abs() / se32
    );
    assert!(
        (e65 - 0.0199842).abs() < 4.0 * se65,
        "cut-card 6:5 house edge {e65:.6}, {:.1} se from 0.0199842 (WoO)",
        (e65 - 0.0199842).abs() / se65
    );

    // Paid-natural gap: (1.5 - 1.2) * rate = e65 - e32. Six-deck theory is
    // 0.045323, so the gap is 0.0135969. Its se here is ~1e-5, so 1e-4 is a wide
    // guard that still trips on a misordered peek or a dropped both-natural push.
    let gap = e65 - e32;
    assert!(
        (gap - 0.0135969).abs() < 1e-4,
        "paid-natural gap {gap:.6} (rate {:.5}), expected 0.0135969",
        gap / 0.3
    );

    // The number chapter 3 prints. Loose because the ratio's denominator carries
    // the 3:2 sampling noise; the gap above is the sharp check.
    let ratio = e65 / e32;
    assert!(
        (ratio - 3.13).abs() < 0.15,
        "6:5 / 3:2 ratio {ratio:.3}, expected about 3.13"
    );
}

/// The tight chart check: a few wrong cells in a cold part of the chart move the
/// edge too little for the 4-se window at 3e7 to catch. This runs 2e8 hands, so
/// the se on the CSM figure is ~6e-5 and a stray cell shows up. Ignored by
/// default; run with `cargo test --release -- --ignored`.
#[test]
#[ignore = "high-N chart validation; run with --release -- --ignored (a few minutes)"]
fn blackjack_chart_high_precision() {
    const N: u64 = 200_000_000;
    let (csm, se) = bj_edge(0.0, 1.5, N, 0xC0FD_CE11);
    assert!(
        (csm - 0.0061873).abs() < 4.0 * se,
        "CSM 3:2 house edge {csm:.7}, {:.1} se from 0.0061873",
        (csm - 0.0061873).abs() / se
    );
}

/// The blackjack path through the real trial loop, and the one place `total_staked`
/// and the two edge denominators are exercised together: a `Shoe` carries state,
/// so `run_trials` builds a fresh one per trial through the closure.
#[test]
fn run_trials_drives_a_blackjack_shoe() {
    let cfg = TrialConfig {
        trials: 40,
        rounds: 50_000,
        bankroll: 1e12, // effectively unruinable: this measures the edge, not ruin
        bounds: Bounds {
            base: 1.0,
            table_max: 1e12,
        },
        checkpoints: 20,
        seed: 0xB1AC_C0DE,
        rounds_per_hour: 70.0,
    };

    let mut k = 0u64;
    let result = run_trials(&cfg, || {
        k += 1;
        // Each trial's shoe gets its own construction shuffle; the play stream is
        // the per-trial rng `run_trials` owns, so the trials stay independent.
        let mut rng = Rng::seed(0x00C0_FFEE ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let shoe = Shoe::new(BjRules::locked(1.5), locked_h17_strategy(), &mut rng);
        (Round::Single(Generator::Shoe(shoe)), Policy::Flat)
    });

    // Flat one-unit bets, never ruined: exactly one unit staked per decision.
    assert_eq!(result.ruined, 0);
    let decisions = f64::from(cfg.trials) * f64::from(cfg.rounds);
    assert!((result.total_staked - decisions).abs() < 1e-6);
    // Doubles and splits put extra money up, so total wagered exceeds staked.
    assert!(result.total_wagered > result.total_staked);
    assert!(result.paths.iter().all(|x| x.is_finite()));

    // House edge per initial bet is the published ~0.6% figure; the element of
    // risk on total wager is strictly smaller, because its denominator is larger.
    let he = house_edge(&result);
    let eor = realized_edge(&result);
    assert!(he > 0.0 && he < 0.03, "house edge {he}");
    assert!(eor < he, "element of risk {eor} not below house edge {he}");
}
