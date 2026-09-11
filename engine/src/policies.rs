//! `policies.rs`

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    None,     // got the bet it asked for
    Table,    // table max truncated it
    Bankroll, // Wallet failed before the house rule
}

#[derive(Clone, Copy, Debug)]
pub struct Bet {
    pub amount: f64,
    pub cap: Cap,
}

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub base: f64,
    pub table_max: f64,
}

/// Clamp once for consistency
fn clamp(desired: f64, bankroll: f64, b: &Bounds) -> Bet {
    if desired > b.table_max && b.table_max <= bankroll {
        Bet {
            amount: b.table_max,
            cap: Cap::Table,
        }
    } else if desired > bankroll {
        Bet {
            amount: bankroll,
            cap: Cap::Bankroll,
        }
    } else {
        Bet {
            amount: desired,
            cap: Cap::None,
        }
    }
}

/// Shortcut for stepping back two on a win a subtraction
/// instead of a two-variable unwind (I make a lot of mistakes)
#[inline]
fn fib(n: u32) -> f64 {
    let (mut a, mut b) = (1u64, 1u64);
    for _ in 0..n {
        let next = a + b;
        a = b;
        b = next;
    }
    a as f64
}

pub enum Policy {
    Flat,
    Martingale { streak: u32 }, // state: consecutive losses
    Fibonacci { idx: u32 },     // state: position in sequence
    DAlembert { offset: u32 },  // state: units above base
}

impl Policy {
    pub fn next_bet(&self, bankroll: f64, b: &Bounds) -> Bet {
        let desired = match self {
            Policy::Flat => b.base,
            // I suppose someone could be ridiculously unlucky...
            Policy::Martingale { streak } => b.base * 2f64.powi((*streak).min(63) as i32),
            Policy::Fibonacci { idx } => b.base * fib(*idx),
            Policy::DAlembert { offset } => b.base * f64::from(offset + 1),
        };
        clamp(desired, bankroll, b)
    }

    pub fn update(&mut self, won: bool) {
        match self {
            Policy::Flat => {}
            Policy::Martingale { streak } => {
                *streak = if won { 0 } else { *streak + 1 };
            }
            Policy::Fibonacci { idx } => {
                *idx = if won { idx.saturating_sub(2) } else { *idx + 1 };
            }
            Policy::DAlembert { offset } => {
                *offset = if won {
                    offset.saturating_sub(1)
                } else {
                    *offset + 1
                };
            }
        }
    }
}
