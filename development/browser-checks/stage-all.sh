#!/bin/sh
# usage: stage-all.sh <output-dir>    Runs the PLAY-F06 / PLAY-N04 browser checks in order, one log per check.
# The text-model checks (replies, clean) need OPENAI_API_KEY in the repo-root .env (read by serve.sh, never printed).
# Evaluator tooling, not application code.
OUT=${1:-/tmp}
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# A first game right after a new client bundle is served can stall in the headless browser; play one throwaway game first.
node "$HERE/stage-offers.mjs" > "$OUT/warm-up.log" 2>&1
for check in sound-click stage-opening stage-midspeech stage-muted stage-quick stage-battle stage-intent stage-victory stage-clean stage-replies; do
  node "$HERE/$check.mjs" > "$OUT/$check.log" 2>&1
  echo "$check exit $?" >> "$OUT/summary.txt"
done
FIGHT=enemy node "$HERE/stage-fight.mjs" > "$OUT/stage-fight-enemy.log" 2>&1
echo "stage-fight enemy exit $?" >> "$OUT/summary.txt"
FIGHT=hero node "$HERE/stage-fight.mjs" > "$OUT/stage-fight-hero.log" 2>&1
echo "stage-fight hero exit $?" >> "$OUT/summary.txt"
echo done >> "$OUT/summary.txt"
