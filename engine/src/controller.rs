use crate::policies::Bounds;

pub struct TrialConfig {
    pub trials: u32,
    pub rounds: u32,
    pub bankroll: f64,
    pub bounds: Bounds,
    pub checkpoints: u32, // for the fan chart
    pub seed: u64,
    pub rounds_per_hour: f64, // independent of stuff, important for bacarat
}

pub struct TrialResult {
    pub paths: Vec<f64>,
    pub ruined: u32,
    pub total_wagered: f64,
    pub total_net: f64,
    pub capped_by_table: u64,
    pub capped_by_bankroll: u64,
    pub elapsed_hours: f64,
}
