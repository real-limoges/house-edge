# The House Edge

An actuary's ledger that happens to be auditing a casino.

![The House Edge](assets/mr_house.webp)

A standalone, self-contained scrollytelling site that makes the statistics of gambling
forensically visible. Four chapters — roulette, blackjack, craps, sports parlays — rising
in structural complexity, each backed by a Monte Carlo engine and closing on a three-line
"ledger" reveal. The tone is cold and auditor, not moralizing: prose persuades, numbers
don't, and the site is built so the numbers do the arguing.

The throughline is that **edge ≠ cost**. What drains a bankroll isn't the number on the
sign — it's edge × bet size × rounds-per-hour, and, for the harder games, the *shape* of
the outcome distribution rather than its mean.

## Architecture

Three deliberately decoupled layers:

1. **Rust simulation engine** (`engine/`, crate `house-edge-engine`) — a pure library,
   native-tested first, that compiles to wasm. Dependence-in-the-draw and
   dependence-in-the-bet are orthogonal axes, composed by a round controller — one trial
   loop, not four bespoke simulators.
2. **WASM module** — hand-rolled `#[no_mangle]` exports over flat `f64`/`i32` buffers in
   linear memory. No wasm-bindgen, no threads. Base64-inlined into the HTML.
3. **Static shell** — vanilla HTML/CSS/JS. IntersectionObserver reveals, pocket-rail nav,
   hand-rolled SVG/Canvas charts. No framework, no build step for the shell, no server.

The deliverable is a single `index.html` (generated) that runs from `file://` or on any
static host.

## Build

```sh
# Validate the engine's math natively — do this first, and whenever the engine changes.
# tests/reference.rs checks edges & ruin probabilities against Wizard of Odds.
cd engine && cargo test

# Compile the engine to wasm and inline it into the shell -> index.html
./build.sh
```

## Layout

| Path                     | What it is                                                        |
| ------------------------ | ----------------------------------------------------------------- |
| `engine/`                | Rust simulation engine (generators, policies, controller, stats). |
| `src/index.template.html`| Source of truth for the shell. Never hand-edit generated `index.html`. |
| `assets/`                | Images and media referenced by this README.                       |
| `docs/`                  | Rendered build guides + `HANDOFF.md` spec. **Gitignored** — local reference only. |

> The complete scope, per-chapter specs, and the reasoning behind each decision live in
> `docs/HANDOFF.md`.
