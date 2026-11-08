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
# her fruit stall. `@frames <option>` lines (one `persona watch` option each: `frames=<name>`,
# `scene=<what the picture shows>`, `hold=30`, ...) let the picture follow the thread: every line
# said is handed to `persona watch`, whose transcript is printed at the end and kept in
# $FRAMES_LOG. `@backfill sync` has the keeper's missing details (how a blade looks up close)
# backfilled before the answer, so the answer can say them (`persona ask backfill=sync`).
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
FRAMES=()
KEEPER_BACKFILL=true
EXHIBITS=()
while IFS= read -r setting; do
  value=${setting#* }
  case "$setting" in
    '@keeper '*) KEEPER=$value ;;
    '@keeper_situation '*) KEEPER_SITUATION=$value ;;
    '@speaker_situation '*) SPEAKER_SITUATION=$value ;;
    '@state '*) STARTING_STATE+=("set $value") ;;
    '@rapport '*) STARTING_STATE+=("rapport $value") ;;
    '@frames '*) FRAMES+=("$value") ;;
    '@backfill '*) KEEPER_BACKFILL=$value ;;
    '@exhibit '*) EXHIBITS+=("$value") ;;
  esac
done < <(grep '^@' "$BEATS")
# `@exhibit <what it is. Up close: what anyone sees>` lines (`persona story beats`) are on show
# for everyone there: they join every situation, so whoever looks closely sees what it truly is.
if [ "${#EXHIBITS[@]}" -gt 0 ]; then
  KEEPER_SITUATION="$KEEPER_SITUATION On show before everyone: ${EXHIBITS[*]}"
  SPEAKER_SITUATION="$SPEAKER_SITUATION On show before everyone: ${EXHIBITS[*]}"
fi
# `@state <emotion> <level>` and `@rapport <toward> <-100..100>` lines set where the keeper's
# feelings and standing start (a scene that begins mid-story), through the same state API as
# `persona state <id> set|rapport`.
for preset in "${STARTING_STATE[@]}"; do
  $D dev --port $P persona state "$KEEPER" $preset >/dev/null
done

# The frame watch reads one `<speaker id> | <line>` per line said, on fd 4.
WATCH_PID=""
if [ ${#FRAMES[@]} -gt 0 ]; then
  FRAMES_LOG=${FRAMES_LOG:-artifacts/tmp/market/frames.log}
  mkdir -p "$(dirname "$FRAMES_LOG")"
  exec 4> >($D dev --port $P persona watch "${FRAMES[@]}" >"$FRAMES_LOG" 2>&1)
  WATCH_PID=$!
fi
watch_line() { # speaker, line
  [ -n "$WATCH_PID" ] && [ -n "$2" ] && printf '%s | %s\n' "$1" "$2" >&4
}

# Mara's discount rules live in her persona (a trigger is a statement the model judges, never a
# keyword). Adding them again to a persona that has them is refused and ignored.
if [ "$KEEPER" = fruit_vendor ]; then
  $D dev --port $P persona behavior fruit_vendor add firm-price "Holds her price on a single piece of fruit and politely refuses any discount on just one; she says each piece is already priced fairly." >/dev/null 2>&1
  $D dev --port $P persona behavior fruit_vendor add bulk-discount "Offers a modest discount, warmly and with a little pride, when a customer buys five or more pieces of fruit." >/dev/null 2>&1
  $D dev --port $P persona trigger fruit_vendor add one-piece-haggle behavior=firm-price "A customer asks for a discount on a single piece of fruit" >/dev/null 2>&1
  $D dev --port $P persona trigger fruit_vendor add bulk-buyer behavior=bulk-discount "A customer buys five or more pieces of fruit" >/dev/null 2>&1
fi

# The last twenty lines of the thread, as repeated `recent=` options (a short window let lines
# repeat what was said a few exchanges back).
recent_args() {
  local start=$(( ${#HISTORY[@]} > 20 ? ${#HISTORY[@]} - 20 : 0 )) i
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
  LAST_INTENT="$instruction"
  HISTORY+=("$who: $line")
  watch_line "$who" "$line"
  sleep 3
}

# The keeper answers the line actually said: they see only what the speaker said, never what the
# beat hoped they would do, so a speaker whose persona refuses a beat cannot put words in theirs.
# The one the line was put to answers it: the addressee judge (`persona addressed`, a model call
# against the scene's speakers) reads whom it is directed at; nobody, the speaker themself or no
# answer leaves it to the keeper. When the next beat is theirs, that beat is their answer, so they
# do not answer twice (run 69: a question to the smith, then his own examination).
keeper() {
  local out line answerer situation
  recent_args
  # The judge is told what the speaker set out to do (the beat's goal, where they are): whom
  # they meant the line for counts as much as its words.
  answerer=$(timeout 120 $D dev --port $P persona addressed "$LAST_SPEAKER" "${RECENT[@]}" \
    "intent=$SPEAKER_SITUATION $LAST_INTENT" "$LAST" 2>/dev/null | head -1)
  case "$answerer" in
    ''|nobody|"$LAST_SPEAKER"|*' '*) answerer=$KEEPER ;;
  esac
  # A line the keeper put to no one is not answered by the keeper themself.
  [ "$answerer" = "$LAST_SPEAKER" ] && return
  [ "$answerer" = "$NEXT_SPEAKER" ] && return
  situation=$SPEAKER_SITUATION
  [ "$answerer" = "$KEEPER" ] && situation=$KEEPER_SITUATION
  out=$(timeout 280 $D dev --port $P persona ask "$answerer" "from=$LAST_SPEAKER" "${RECENT[@]}" \
    "backfill=$KEEPER_BACKFILL" "situation=$situation" "$LAST" 2>&1)
  line=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  N=$((N + 1))
  printf '[%02d] %s: %s\n' "$N" "$answerer" "${line:-<no line: $out>}"
  printf '%s\n' "$out" | sed -n '2,$p' | sed 's/^/       /'
  HISTORY+=("$answerer: $line")
  watch_line "$answerer" "$line"
  sleep 3
}

# Each line of the beats file is `speaker | goal`; `#` lines are comments, `@` lines settings, and
# a `! <dev command>` line runs that dev command between beats (a stance switched on mid-scene).
mapfile -t LINES < "$BEATS"

# The speaker of the first beat after line $1 of the beats file, or nothing.
next_speaker() {
  local next=$(($1 + 1)) raw
  while [ "$next" -lt "${#LINES[@]}" ]; do
    raw=${LINES[$next]}
    case "$raw" in
      '!'*|'@'*|'#'*|'') next=$((next + 1)); continue ;;
    esac
    printf '%s' "${raw%%|*}" | xargs
    return
  done
}

for index in "${!LINES[@]}"; do
  raw=${LINES[$index]}
  case "$raw" in
    '!'*) eval "$D dev --port $P ${raw#!}" < /dev/null | sed 's/^/       ! /'; continue ;;
    '@'*|'#'*|'') continue ;;
  esac
  who=${raw%%|*}
  goal=${raw#*|}
  [ "$who" = "$raw" ] && goal=""
  who=$(printf '%s' "$who" | xargs)
  case "$who" in ''|'#'*) continue ;; esac
  say_as "$who" "$(printf '%s' "$goal" | sed 's/^ *//;s/ *$//')"
  NEXT_SPEAKER=$(next_speaker "$index")
  keeper
done

if [ -n "$WATCH_PID" ]; then
  exec 4>&-
  wait "$WATCH_PID"
  printf '\n--- frames (%s) ---\n' "$FRAMES_LOG"
  cat "$FRAMES_LOG"
fi
