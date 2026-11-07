#!/bin/bash
# 20-message thread at Mara's fruit stall: her name, where the apples come from, the price, a
# discount on one apple (should be refused), a discount on five (should be granted). The elf and
# the warrior speak through `persona say` from a one-line goal per beat; Mara answers through
# `persona ask` (coverage, backfill), fully live. Every call gets the last lines of the thread
# (`recent=`) and who is speaking to her (`from=`), so nobody repeats themselves; the standing
# context is stated once, in the first beat. Needs the market scene up (see
# development/runbook-dev-playground.md). Usage: bash development/market-thread.sh [beats-file]
# (default development/market-threads/haggle.beats; one file per variation). A beats file may set
# who answers and where with `@keeper <id>`, `@keeper_situation <text>` and
# `@speaker_situation <text>` lines, and where the keeper's feelings and standing start with
# `@state <emotion> <level>` and `@rapport <toward> <-100..100>` lines; without them it is Mara at
# her fruit stall.
cd "$(dirname "$0")/.."
D=${DF_TABLE_EXE:-./artifacts/build/bubbles/debug/df-table.exe}
P=${DF_TABLE_PORT:-7879}
N=0
LAST=""
LAST_SPEAKER=""
HISTORY=()
BEATS=${1:-development/market-threads/haggle.beats}
KEEPER=fruit_vendor
KEEPER_SITUATION="At her fruit stall in the town market."
SPEAKER_SITUATION="At Mara's fruit stall in the town market, with your travelling companion."
STARTING_STATE=()
while IFS= read -r setting; do
  value=${setting#* }
  case "$setting" in
    '@keeper '*) KEEPER=$value ;;
    '@keeper_situation '*) KEEPER_SITUATION=$value ;;
    '@speaker_situation '*) SPEAKER_SITUATION=$value ;;
    '@state '*) STARTING_STATE+=("set $value") ;;
    '@rapport '*) STARTING_STATE+=("rapport $value") ;;
  esac
done < <(grep '^@' "$BEATS")
# `@state <emotion> <level>` and `@rapport <toward> <-100..100>` lines set where the keeper's
# feelings and standing start (a scene that begins mid-story), through the same state API as
# `persona state <id> set|rapport`.
for preset in "${STARTING_STATE[@]}"; do
  $D dev --port $P persona state "$KEEPER" $preset >/dev/null
done

# Mara's discount rules live in her persona (a trigger is a statement the model judges, never a
# keyword). Adding them again to a persona that has them is refused and ignored.
if [ "$KEEPER" = fruit_vendor ]; then
  $D dev --port $P persona behavior fruit_vendor add firm-price "Holds her price on a single piece of fruit and politely refuses any discount on just one; she says each piece is already priced fairly." >/dev/null 2>&1
  $D dev --port $P persona behavior fruit_vendor add bulk-discount "Offers a modest discount, warmly and with a little pride, when a customer buys five or more pieces of fruit." >/dev/null 2>&1
  $D dev --port $P persona trigger fruit_vendor add one-piece-haggle behavior=firm-price "A customer asks for a discount on a single piece of fruit" >/dev/null 2>&1
  $D dev --port $P persona trigger fruit_vendor add bulk-buyer behavior=bulk-discount "A customer buys five or more pieces of fruit" >/dev/null 2>&1
fi

# The last six lines of the thread, as repeated `recent=` options.
recent_args() {
  local start=$(( ${#HISTORY[@]} > 6 ? ${#HISTORY[@]} - 6 : 0 )) i
  RECENT=()
  for ((i = start; i < ${#HISTORY[@]}; i++)); do RECENT+=("recent=${HISTORY[$i]}"); done
}

say_as() { # speaker, instruction
  local who="$1" instruction="$2" out line
  recent_args
  out=$(timeout 200 $D dev --port $P persona say "$who" "${RECENT[@]}" "intent=$instruction" \
    "$SPEAKER_SITUATION" 2>&1)
  line=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  N=$((N + 1))
  printf '[%02d] %s: %s\n' "$N" "$who" "${line:-<no line: $out>}"
  printf '%s\n' "$out" | sed -n '2,$p' | grep -E 'quoted|fired triggers' | sed 's/^/       /'
  LAST="$line"
  LAST_SPEAKER="$who"
  HISTORY+=("$who: $line")
  sleep 3
}

# The keeper answers the line actually said: they see only what the speaker said, never what the
# beat hoped they would do, so a speaker whose persona refuses a beat cannot put words in theirs.
keeper() {
  local out line
  recent_args
  out=$(timeout 280 $D dev --port $P persona ask "$KEEPER" "from=$LAST_SPEAKER" "${RECENT[@]}" \
    "situation=$KEEPER_SITUATION" "$LAST" 2>&1)
  line=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  N=$((N + 1))
  printf '[%02d] %s: %s\n' "$N" "$KEEPER" "${line:-<no line: $out>}"
  printf '%s\n' "$out" | sed -n '2,$p' | sed 's/^/       /'
  HISTORY+=("$KEEPER: $line")
  sleep 3
}

# Each line of the beats file is `speaker | goal`; `#` lines are comments, `@` lines settings.
while IFS="|" read -r -u 3 who goal; do
  case "$who" in '@'*) continue ;; esac
  who=$(printf '%s' "$who" | xargs)
  case "$who" in ''|'#'*) continue ;; esac
  say_as "$who" "$(printf '%s' "$goal" | sed 's/^ *//;s/ *$//')"
  keeper
done 3< "$BEATS"
