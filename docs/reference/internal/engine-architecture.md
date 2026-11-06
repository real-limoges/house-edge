# Simulation Engine Architecture

The shape of the Monte Carlo engine in `engine/`, and why it has that shape.

One observation carries the whole design: **dependence-in-the-draw and dependence-in-the-bet are orthogonal axes.**
Two small interfaces plus a round controller produce every chapter's simulation from one trial loop.
That loop is compiled once and runs in both runtimes: wasm in the page and a rustler NIF on the server.
Neither runtime changes how a round resolves or how a bet is sized; each is a thin wrapper over the same core crate.

## Axis 1: Outcome Generators (How a Round Resolves)

`Generator` in `engine/src/generators.rs` has three variants.

| Variant | State | Drives |
|---|---|---|
| `Iid(IidTable)` | None | Roulette, baccarat, craps single bets, video poker, lottery, slots |
| `Shoe(Shoe)` | Cards dealt, running count | Blackjack |
| `Correlated(Copula)` | None between rounds | Sports and event parlays |

**`IidTable`** samples from a fixed discrete (probability, net payout) table.
The table is **arbitrary rather than closed-form**, which is what lets slots be handled honestly with no special case: real machines use weighted virtual reels where blanks and near-misses are overrepresented relative to physical symbols, zero-inflated by design, and an arbitrary discrete table captures that for free.

**`Shoe`** samples cards without replacement and reshuffles at a penetration threshold (0.75 for the blackjack rules the site models).
The depletion is the point: it is the mechanism card counting exploits, and it produces genuine within-shoe runs that an i.i.d. model smears into a scalar drift.
The real complexity is the hand logic on top (hit, stand, double, split against a strategy table), not the depletion itself.
**True count** (running count divided by decks remaining) falls out of state already tracked, and it powers chapter 3's signature visual.

**`Copula`** draws a joint vector of leg outcomes through a single-factor **Gaussian copula**: correlated normals, each compared against its leg's `probit(p_win)` threshold, and the slip wins only if every leg does.
`rho` lies in `[0, 1)`.
It is a **reader-facing slider, not a hardcoded constant**, because real correlations are proprietary: books estimate them from historical data and they are not a constant of nature.
Exposing `rho` makes that uncertainty something the reader can see and vary, rather than a constant presented as fact.

## Axis 2: Staking Policies (How Bet Size Evolves with History)

Every policy is **state plus update**, never a pure function of full history: `next_bet` reads the state, `update` advances it after a decided round.
Streak-based systems fall out as the trivial case, so the interface never needs widening.
A push (zero net) leaves the state untouched.

`Policy` in `engine/src/policies.rs`:

| Policy | State | Rule |
|---|---|---|
| `Flat` | None | Constant bet |
| `Martingale { streak }` | Consecutive losses | Base times 2 to the streak; reset on a win |
| `Fibonacci { idx }` | Position in the sequence | Advance one on a loss, step back two on a win |
| `DAlembert { offset }` | Units above base | Plus one unit on a loss, minus one on a win |

Every policy is bounded.
The martingale multiplier is fixed at 2; starting bet, table max and bankroll are explicit parameters, and the policy never runs unbounded.
Whichever of table max and bankroll binds first ends the doubling, and the engine records which one it was.

`DAlembert` is linear rather than exponential, which is the reason it is in the set: it is the gentler system people convince themselves is safer.

Policies are swappable under any even-money `IID` chapter.

## The Round Controller (Composite Resolution)

Some bet *decisions* do not resolve on a single draw.
`Round` in `engine/src/controller.rs` wraps the draw for one decision:

- **`Round::Single(Generator)`**: one draw per decision.
  Parlays use this path: a slip is one `Correlated` draw that returns a single `Outcome`, since every leg resolves at once.
- **`Round::CrapsPassLine { odds_multiple }`**: a *geometric* stopping time.
  The come-out roll either resolves immediately (7 or 11 wins; 2, 3 or 12 loses) or establishes a point, after which dice are rolled until the point repeats (win) or a 7 shows (lose).
  Each roll is independent, so there is **no** dependence and no further generator is needed.
  What makes craps hard is resolution structure, not memory.

The composition lives in shared machinery, not bespoke logic in each chapter.

## Play Rate: A Declared Parameter, Not a Third Axis

Without a clock, "edge times bet size times rounds per hour" is not expressible anywhere.
The site's argument is that cost per hour is quoted nowhere, so an engine without rate would have nothing to say about it.

**`rounds_per_hour` is a declared trial parameter** on `TrialConfig`.
It does not affect the draw, the policy or the round controller, and it does not touch the hot loop.
It converts rounds into `elapsed_hours` on the way out, so the chart axis and the ledger cannot disagree about it.

This is deliberately **not** a third axis: the two orthogonal axes are still the outcome generator and the staking policy.
Rate is a unit conversion attached to the trial, in the same category as the starting bankroll.

Consequences:

- A fan chart can put wall-clock time on the x-axis.
  Baccarat is the one chapter that does, because its cost is in the tempo rather than the edge.
- Cost per hour is computed from simulated results (`stats::cost_per_hour`), never from `edge * bet * rate` in closed form.
  The closed form is right for even-money flat betting and wrong everywhere else, including every chapter with a staking policy or a composite round, and asserting it would be the site making exactly the kind of uncheckable claim it exists to expose.

## Statistical-Honesty Constraint

This is a correctness constraint, not a performance one.
Heavy-tailed bets (straight-up roulette at 35:1, jackpot slots) need **real trial volume** before percentile bands mean anything.
At low N, paths that never hit the rare big payout look like a flat decay, and the 95th-percentile band is meaningless until enough trials have sampled the rare event.

**The floor attaches to the bet, not the generator**, because the generator sets the shape of the draw and the paytable sets the variance.
Two `IID` bets at an identical 5.263% edge, even money and straight up at 35:1, need 6.4e7 and 2.1e9 rounds respectively for the same precision.

Three clauses, all read from the bet at run time rather than hardcoded:

1. **Estimator precision.**
   `N >= (k * sd / tol)^2`, with `k` of 4 and `tol` the precision the page's copy claims.
   For an `IidTable`, `sd` is exact from `IidTable::sd()`.
   For blackjack, which has no closed form, `sd` is estimated from sampled hands.
2. **Rare-event coverage.**
   `trials * rounds * p_min >= 30`, where `p_min` is the least likely outcome the chart claims to show.
   Clause 1 can be satisfied while no trial has sampled the event the upper band is meant to represent; this clause catches that.
3. **Band stability.**
   Empirical: sweep `N` with replicate seeds and take the point where the 95th-percentile estimate stops moving between replicates.
   The sweep runs in the native harness, before either runtime is involved.

Blackjack's per-round cost is heavier than a table lookup (card draws plus hand evaluation) but still trivial for single-threaded wasm at the trial counts the page runs.
The large sweeps run on the server.

## What "Clustering in the Tails" Means

Two distinct things get conflated under that phrase, and they need different treatment.

**Heavy-tailed marginals under independence.**
A single-number roulette bet or a jackpot slot pull produces wildly skewed trajectories *even under perfect independence*: most paths grind down, a few spike.
The fan chart looks clustered with no serial dependence in the process at all.
That is an honestly shaped marginal plus enough trials, not a modeling gap.

**Genuine serial dependence.**
Three sources are in scope:

1. **Blackjack shoe depletion**: a finite shoe sampled without replacement, so the conditional edge shifts hand to hand.
2. **Same-game parlay leg correlation**: legs are not independent, and the naive independent-legs model is *less* honest than the correlated one.
   Handled by the copula and the `rho` slider.
3. **Progressive betting**: the games stay independent; what becomes path-dependent is bet *size* as a function of bankroll history.
   That is the staking-policy axis, not a change to the draw distribution.

Everything else in scope (craps pass line, baccarat, video poker, lottery) is legitimately i.i.d. per decision.
