#!/usr/bin/env bash
# build.sh — compile the engine to wasm and assemble the single, self-contained
# deliverable `index.html` from `src/`.
#
# The ONLY build step in the project. It does three things:
#   1. cargo build --release --target wasm32-unknown-unknown   (-> .wasm)
#   2. concatenate src/css/*.css and src/js/*.js into the template
#   3. base64-inline the .wasm between the WASM_B64 markers (which live in engine.js)
#
# Output: ./index.html (gitignored). Never hand-edit index.html — edit src/.
set -euo pipefail
cd "$(dirname "$0")"

CRATE=house_edge_engine
WASM="engine/target/wasm32-unknown-unknown/release/${CRATE}.wasm"

# Concatenation order matters: engine.js first (defines the wasm interface and the
# WASM_B64 markers), then modules that depend on it. reveal/rail are independent.
CSS_FILES=(src/css/tokens.css src/css/layout.css)
JS_FILES=(src/js/engine.js src/js/flipdigit.js src/js/fanchart.js src/js/reveal.js src/js/rail.js src/js/slider.js)

echo "==> cargo build (wasm32-unknown-unknown, release)"
cargo build --release --target wasm32-unknown-unknown -p "$CRATE" --manifest-path engine/Cargo.toml

# Optional extra shrink if wasm-opt is on PATH.
if command -v wasm-opt >/dev/null 2>&1; then
  echo "==> wasm-opt -Oz"
  wasm-opt -Oz "$WASM" -o "$WASM"
fi

echo "==> assembling index.html"
B64=$(base64 < "$WASM" | tr -d '\n')

CSS_LIST="${CSS_FILES[*]}" JS_LIST="${JS_FILES[*]}" B64="$B64" python3 - <<'PY'
import os, re, pathlib

def concat(files):
    return "\n".join(pathlib.Path(f).read_text() for f in files.split())

css = concat(os.environ["CSS_LIST"])
js  = concat(os.environ["JS_LIST"])
b64 = os.environ["B64"]

shell = pathlib.Path("src/index.template.html").read_text()
shell = shell.replace("/* CSS_INJECT */", css)
shell = shell.replace("/* JS_INJECT */", js)

# Splice the base64 payload between the markers (now inlined via engine.js).
shell, n = re.subn(
    r'(/\* WASM_B64_START \*/")[^"]*("/\* WASM_B64_END \*/)',
    lambda m: m.group(1) + b64 + m.group(2),
    shell,
)
assert n == 1, f"expected exactly one WASM_B64 marker pair, found {n}"

pathlib.Path("index.html").write_text(shell)
print(f"    inlined {len(b64)} b64 chars, {len(css)} css bytes, {len(js)} js bytes -> index.html")
PY

echo "==> done: index.html"
