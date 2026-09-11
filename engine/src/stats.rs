use crate::controller::TrialResult;

pub struct Bands {
    pub levels: Vec<f64>,
    pub checkpoints: usize,
    pub values: Vec<f64>,
}

pub fn percentile_bands(paths: &[f64], trials: usize, checkpoints: usize, levels: &[f64]) -> Bands {
    assert_eq!(
        paths.len(),
        trials * checkpoints,
        "paths is not trials by checkpoints"
    );
    assert!(trials > 0, "no trials to take pct of");

    let mut column = vec![0.0f64; trials];
    let mut values = vec![0.0f64; levels.len() * checkpoints];

    for c in 0..checkpoints {
        for (t, slot) in column.iter_mut().enumerate() {
            *slot = paths[t * checkpoints + c];
        }
        column.sort_by(|a, b| a.partial_cmp(b).expect("NaN in a trajetory"));

        for (l, level) in levels.iter().enumerate() {
            values[l * checkpoints + c] = quantile_sorted(&column, *level);
        }
    }

    Bands {
        levels: levels.to_vec(),
        checkpoints,
        values,
    }
}

fn quantile_sorted(sorted: &[f64], level: f64) -> f64 {
    assert!(
        (0.0..=1.0).contains(&level),
        "percentile {level} not in [0, 1)",
    );
    let n = sorted.len();
    if n == 1 {
        return sorted[0];
    }
    let h = level * (n - 1) as f64;
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    sorted[lo] + (h - lo as f64) * (sorted[hi] - sorted[lo])
}

pub fn ruin_probability(r: &TrialResult) -> f64 {
    f64::from(r.ruined) / f64::from(r.trials)
}

/// Element-of-risk edge: expected loss per unit of *total* money wagered, double
/// and split and free-odds money included. This is the published convention for
/// craps combined-odds (the 0.326% at 5x odds is per total wager).
pub fn realized_edge(r: &TrialResult) -> f64 {
    -r.total_net / r.total_wagered
}

/// House edge: expected loss per unit of the *initial* bet, ignoring the extra
/// money a double or split puts up mid-decision. This is the published
/// convention for blackjack (0.64%, not the ~0.56% element of risk) and it
/// coincides with `realized_edge` for any game whose decisions stake exactly
/// one unit.
pub fn house_edge(r: &TrialResult) -> f64 {
    -r.total_net / r.total_staked
}

pub fn cost_per_hour(r: &TrialResult) -> f64 {
    r.total_net / (f64::from(r.trials) * r.elapsed_hours)
}

pub fn cap_attribution(r: &TrialResult) -> (f64, f64) {
    let total = (r.capped_by_table + r.capped_by_bankroll) as f64;
    if total == 0.0 {
        return (0.0, 0.0);
    }
    (
        r.capped_by_table as f64 / total,
        r.capped_by_bankroll as f64 / total,
    )
}
