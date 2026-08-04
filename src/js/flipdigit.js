// flipdigit.js — the tote-board reveal. HANDOFF §6.3: only ONE number per
// chapter (line 3 of the ledger, "the take") gets the flip-in; everything else
// is plain tabular mono. Sliders re-flip this line live.
//
// TODO: render a value as flip-digit cells and animate transitions between
// values (slider recompute calls flip(el, newValue)).

function flipDigit(el, value) {
  // placeholder: set text now; animate later.
  el.textContent = value;
}
