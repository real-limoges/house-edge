// fanchart.js — hand-rolled SVG/Canvas fan chart (no charting library).
// Draws percentile bands (5th/50th/95th) from the engine's SimResult, plus
// ruin probability. Each game's payout/variance structure must look distinct
// (35:1 straight-up vs. even-money outside bet), so this reads real buffers,
// not just a scalar edge.
//
// TODO: implement draw(container, simResult). Blackjack overlays a running-count
// line on the same x-axis; craps overlays the accumulating die-pip strip.

function drawFanChart(container, simResult) {
  // placeholder
}
