//! Tables are tested exactly, samplers statistically, never both at once.
//! Sampled tolerances derive from N and the spread, never a fixed epsilon.

use house_edge_engine::controller::{pass_line, run_trials, Round, TrialConfig};
use house_edge_engine::generators::{
    any_seven, baccarat_banker, baccarat_player, locked_h17_strategy, roulette_even_money,
    roulette_straight_up, BjRules, Generator, IidTable, Shoe,
};
use house_edge_engine::policies::{Bounds, Cap, Policy};
use house_edge_engine::rng::Rng;
use house_edge_engine::stats::{house_edge, realized_edge};

/// (house edge, standard error) over `n` hands of one shoe.
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

/// Exact.
#[test]
fn published_house_edges() {
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
        // Unrounded: 0.0124 sits exactly on the 5e-5 tolerance.
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

/// Exact.
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

/// Exact.
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

    // N for +/- 0.05pp at 4 se.
    let n = |sd: f64| (4.0 * sd / 5e-4).powi(2);
    assert!((n(even.sd()) / 6.4e7 - 1.0).abs() < 0.05);
    assert!((n(straight.sd()) / 2.1e9 - 1.0).abs() < 0.05);
}

/// Statistical.
#[test]
fn sampler_reproduces_its_table() {
    const N: u32 = 4_000_000;
    let table = roulette_even_money(2);
    let mut rng = Rng::seed(0xC0FFEE);

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

/// Statistical.
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

/// Exact.
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
        p.update(false);
    }

    // $10, 20, 40, 80, 160, then the sixth bet is truncated.
    assert_eq!(caps.iter().position(|c| *c == Cap::Bankroll), Some(5));
    assert!(!caps.contains(&Cap::Table));
}

/// Exact.
#[test]
fn clamp_reports_the_bound_that_binds_first() {
    let bounds = Bounds {
        base: 10.0,
        table_max: 100.0,
    };
    let p = Policy::Martingale { streak: 5 }; // wants $320

    let bet = p.next_bet(1_000.0, &bounds);
    assert_eq!(bet.cap, Cap::Table);
    assert_eq!(bet.amount, 100.0);

    let bet = p.next_bet(60.0, &bounds);
    assert_eq!(bet.cap, Cap::Bankroll);
    assert_eq!(bet.amount, 60.0);

    let bet = Policy::Flat.next_bet(1_000.0, &bounds);
    assert_eq!(bet.cap, Cap::None);
    assert_eq!(bet.amount, 10.0);
}

/// Exact.
#[test]
fn policies_follow_their_progressions() {
    let bounds = Bounds {
        base: 10.0,
        table_max: f64::INFINITY,
    };
    let script = [false, false, false, true, true, false];

    let cases: [(&str, Policy, [f64; 7]); 4] = [
        ("flat", Policy::Flat, [10.0; 7]),
        (
            "martingale",
            Policy::Martingale { streak: 0 },
            [10.0, 20.0, 40.0, 80.0, 10.0, 10.0, 20.0],
        ),
        (
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

/// Statistical.
#[test]
fn pass_line_matches_published_edge() {
    const N: u32 = 20_000_000;
    let mut rng = Rng::seed(0x5EED);
    let (mut net, mut wagered) = (0.0, 0.0);
    for _ in 0..N {
        let o = pass_line(&mut rng, 0.0);
        net += o.net;
        wagered += o.wagered;
    }
    let edge = -net / wagered;
    // Net is +/-1, so sd is ~1.
    let se = 1.0 / f64::from(N).sqrt();
    assert!(
        (edge - 0.0141414).abs() < 4.0 * se,
        "pass line edge {edge:.6}, {:.2} se from 0.0141414",
        (edge - 0.0141414).abs() / se
    );
}

/// Statistical.
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
    let se = sd / n.sqrt() / (wagered / n);
    let edge = -net / wagered;
    assert!(
        (edge - 0.00326).abs() < 4.0 * se,
        "5x odds blended edge {edge:.6}, {:.2} se from 0.00326",
        (edge - 0.00326).abs() / se
    );
    assert!(
        edge < 0.0141414,
        "odds raised the blended edge to {edge:.6}"
    );
}

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
            assert!(o.net == 1.0 || o.net == -1.0, "come-out net {}", o.net);
        } else {
            // 1 plus 2 units at true odds of 2/1, 3/2, 6/5.
            assert!(
                o.net == -3.0 || o.net == 5.0 || o.net == 4.0 || (o.net - 3.4).abs() < 1e-12,
                "point net {}",
                o.net
            );
        }
    }
}

/// Exact.
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

/// Exact.
#[test]
fn seeding_is_deterministic() {
    let draw = |seed| {
        let mut rng = Rng::seed(seed);
        (0..1_000).map(|_| rng.next_u64()).collect::<Vec<_>>()
    };
    assert_eq!(draw(0x5EED), draw(0x5EED));
    assert_ne!(draw(0x5EED), draw(0x5EEE));

    // `IidTable::draw` relies on u < 1.0.
    let mut rng = Rng::seed(1);
    for _ in 0..1_000_000 {
        let u = rng.uniform();
        assert!((0.0..1.0).contains(&u), "uniform out of range: {u}");
    }
}

/// Statistical.
#[test]
fn blackjack_reproduces_wizard_of_odds() {
    const N: u64 = 30_000_000;

    // WoO continuous shuffler, 6-deck H17.
    let (csm, se) = bj_edge(0.0, 1.5, N, 0x0B1A_CC51);
    assert!(
        (csm - 0.0061873).abs() < 4.0 * se,
        "CSM 3:2 house edge {csm:.6}, {:.1} se from 0.0061873",
        (csm - 0.0061873).abs() / se
    );

    // Same seed, same hands: the gap isolates the paid naturals.
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

    // 0.3 * natural rate (0.045323); se ~1e-5.
    let gap = e65 - e32;
    assert!(
        (gap - 0.0135969).abs() < 1e-4,
        "paid-natural gap {gap:.6} (rate {:.5}), expected 0.0135969",
        gap / 0.3
    );

    // Chapter 3's number. Loose: the gap is the sharp check.
    let ratio = e65 / e32;
    assert!(
        (ratio - 3.13).abs() < 0.15,
        "6:5 / 3:2 ratio {ratio:.3}, expected about 3.13"
    );
}

/// Statistical.
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

/// Statistical.
#[test]
fn run_trials_drives_a_blackjack_shoe() {
    let cfg = TrialConfig {
        trials: 40,
        rounds: 50_000,
        bankroll: 1e12, // unruinable
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
        let mut rng = Rng::seed(0x00C0_FFEE ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let shoe = Shoe::new(BjRules::locked(1.5), locked_h17_strategy(), &mut rng);
        (Round::Single(Generator::Shoe(shoe)), Policy::Flat)
    });

    assert_eq!(result.ruined, 0);
    let decisions = f64::from(cfg.trials) * f64::from(cfg.rounds);
    assert!((result.total_staked - decisions).abs() < 1e-6);
    assert!(result.total_wagered > result.total_staked);
    assert!(result.paths.iter().all(|x| x.is_finite()));

    let he = house_edge(&result);
    let eor = realized_edge(&result);
    assert!(he > 0.0 && he < 0.03, "house edge {he}");
    assert!(eor < he, "element of risk {eor} not below house edge {he}");
}

/// Exact.
#[test]
fn ruin_on_the_final_round_is_counted() {
    let run = |rounds: u32| {
        let cfg = TrialConfig {
            trials: 10,
            rounds,
            bankroll: 3.0,
            bounds: Bounds {
                base: 1.0,
                table_max: 1.0,
            },
            checkpoints: 1,
            seed: 7,
            rounds_per_hour: 60.0,
        };
        run_trials(&cfg, || {
            let always_lose = IidTable::new(&[(1.0, -1.0)]);
            (Round::Single(Generator::Iid(always_lose)), Policy::Flat)
        })
    };

    assert_eq!(run(2).ruined, 0, "one unit left still covers the base bet");
    assert_eq!(run(3).ruined, 10, "every trial busts on its last round");
}
