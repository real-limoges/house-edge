// policies.rs

pub enum Policy {
    Flat,
    Martingale { streak: u32, base: f64, mult: f64, table_max: f64, bankroll_cap: f64 },
    Fibonacci { a: f64, b: f64, base: f64 },
    DAlembert { offset: i32, base: f64 }  // +/- 1 unit per loss/win
}

impl Policy {
    // this is the bet to place now given the accumulated state
    #[inline] pub fn next_bet(&self) -> f64 {
        match self {
            Policy::Flat => 1.0,
            Policy::Martingale { streak, base, mult, table_max, .. } =>
                (base * mult.powi(*streak as i32)).min(*table_max),
            Policy::Fibonacci { b, .. } => *b,
            Policy::DAlembert { offset, base } => (base + (*offset as f64)  * base).max(*base)
        }
    }
    #[inline] pub fn update(&mut self, net: f64) {
        let win = net > 0.0;
        match self {
            Policy::Flat => {},
            Policy::Martingale { streak, .. } =>
                *streak = if win { 0 } else { *streak + 1 },
            Policy::Fibonacci { a, b, base } => {
                if win { *a = 0.0; *b = *base; }
                else   { let n = *a + *b; *a = *b; *b = n.max(*base); }
            }
            Policy::DAlembert { offset, .. } => {
                *offset = if win { (*offset - 1).max(0) } else { *offset + 1 };
            }
        }
    }
}
