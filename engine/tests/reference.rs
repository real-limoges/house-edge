// tests/reference.rs
use house_edge_engine::*;

fn edge_of(table: &[(f64, f64)]) -> f64 {
    // house edge = -E[net per unit staked]
    -table.iter().map(|(p, net)| p * net).sum::<f64>()
}

#[test]
fn roulette_edges() {
    // single-zero straight-up: 1/37 win at 35:1
    let euro = [(1.0 / 37.0, 35.0), (36.0 / 37.0, -1.0)];
    assert!((edge_of(&euro) - 0.0270).abs() < 1e-3);
    let american = [(1.0 / 38.0, 35.0), (37.0 / 38.0, -1.0)];
    assert!((edge_of(&american) - 0.0526).abs() < 1e-3);
    let triple = [(1.0 / 39.0, 35.0), (38.0 / 39.0, -1.0)];
    assert!((edge_of(&triple) - 0.0769).abs() < 1e-3);
}

#[test]
fn craps_pass_line_edge() {
    // expect 1.41%
    let params = SimParams {
        trials: 2_000_000,
        rounds: 1,
        start_bankroll: 1e9,
        base_bet: 1.0,
        seed: 42,
    };
    let res = run_trials(
        &params,
        || Generator::Iid(IidTable::new(&[])),
        || Policy::Flat,
        Round::CrapsPassLine,
    );
    let ev = res.p50[1] - res.p50[0];
    assert!((ev + 0.0141).abs() < 2e-3);
}

#[test]
fn any_seven_prop_edge() {
    let any7 = [(6.0 / 36.0, 4.0), (30.0 / 36.0, -1.0)]; // 4:1
    assert!((edge_of(&any7) - 0.1667).abs() < 1e-3);
}
