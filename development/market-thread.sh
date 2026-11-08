#!/bin/bash
# Plays a role-play scene from a beats file against a running host: every story, place and
# character is in the file, none in this script. Each beat is `speaker | goal`: the speaker says
# a line of their own toward the goal (`persona say`), and whoever it is put to answers it fully
# live (`persona ask`: coverage, backfill), the keeper when it is put to the room. Every call gets
# the last lines of the scene (`recent=`) and who is speaking (`from=`). Needs the scene's picture
# up (see development/runbook-dev-playground.md). Usage: bash development/market-thread.sh
# <beats-file> (development/market-threads/ holds one file per scene). A scripted beats file names
# who answers and where with `@keeper <id>`, `@keeper_situation <text>` and `@speaker_situation
# <text>` lines, and may set where the keeper's feelings and standing start with `@state <emotion>
# <level>` and `@rapport <toward> <-100..100>` lines; a `! <dev command>` line runs a dev command
# (a stance or a rule given to a persona for the scene). `@frames <option>` lines (one `persona watch` option each: `frames=<name>`,
# `scene=<what the picture shows>`, `hold=30`, ...) let the picture follow the thread: every line
# said is handed to `persona watch`, whose transcript is printed at the end and kept in
# $FRAMES_LOG. `@backfill sync` has the keeper's missing details (how a blade looks up close)
# backfilled before the answer, so the answer can say them (`persona ask backfill=sync`).
#
# Flowing play (`@flow <first place>`): no beats are scripted. Each turn asks the storyteller for
# the next move from everything said so far (`persona story next`), plays it and lets whoever it
# was put to answer, until the story's end is reached (or `@flow_moves <n>`, default 80, as a
# guard). Each place of the story has `@place <id> <plate>`, `@place_keeper <id> <keeper id>`,
# `@place_keeper_situation <id> <text>`, `@place_situation <id> <text>` and `@place_scene <id>
# <what the plate shows>` lines; on a move that sets off for a place, its plate is shown, its
# situations and keeper are taken, and the frame watch starts again there (`<frames>-<place>`),
# its scene joined by the plot's set dressing for that place (`persona story beats <place>`).
cd "$(dirname "$0")/.."
D=${DF_TABLE_EXE:-./artifacts/build/bubbles/debug/df-table.exe}
P=${DF_TABLE_PORT:-7879}
N=0
LAST=""
LAST_SPEAKER=""
MOVE_TO=""
HISTORY=()
BEATS=${1:?usage: bash development/market-thread.sh <beats-file>}
KEEPER=""
KEEPER_SITUATION=""
SPEAKER_SITUATION=""
STARTING_STATE=()
FRAMES=()
KEEPER_BACKFILL=true
EXHIBITS=()
DRESSING=()
FLOW=""
FLOW_MOVES=80
declare -A PLACE_PLATE PLACE_KEEPER PLACE_KEEPER_SITUATION PLACE_SITUATION PLACE_SCENE
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
    '@dressing '*) DRESSING+=("$value") ;;
    '@flow '*) FLOW=$value ;;
    '@flow_moves '*) FLOW_MOVES=$value ;;
    '@place '*) PLACE_PLATE[${value%% *}]=${value#* } ;;
    '@place_keeper '*) PLACE_KEEPER[${value%% *}]=${value#* } ;;
    '@place_keeper_situation '*) PLACE_KEEPER_SITUATION[${value%% *}]=${value#* } ;;
    '@place_situation '*) PLACE_SITUATION[${value%% *}]=${value#* } ;;
    '@place_scene '*) PLACE_SCENE[${value%% *}]=${value#* } ;;
  esac
done < <(grep '^@' "$BEATS")
# `@dressing <thing in sight>` lines (`persona story beats`, the plot's set dressing for the
# scene) join the frames' scene, so the camera can show them.
if [ "${#DRESSING[@]}" -gt 0 ]; then
  for i in "${!FRAMES[@]}"; do
    case "${FRAMES[$i]}" in
      scene=*) FRAMES[$i]="${FRAMES[$i]} In sight there: $(IFS=';'; echo "${DRESSING[*]}")." ;;
    esac
  done
fi
# `@exhibit <what it is shown as and what anyone sees up close>` lines (`persona story beats`)
# are on show for everyone there: they join every situation, so whoever looks closely sees what
# it truly is.
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
FRAMES_LOG=${FRAMES_LOG:-artifacts/tmp/market/frames.log}
start_watch() { # log, persona watch options...
  WATCH_LOG=$1
  shift
  mkdir -p "$(dirname "$WATCH_LOG")"
  exec 4> >($D dev --port $P persona watch "$@" >"$WATCH_LOG" 2>&1)
  WATCH_PID=$!
}
stop_watch() {
  [ -z "$WATCH_PID" ] && return
  exec 4>&-
  wait "$WATCH_PID"
  printf '\n--- frames (%s) ---\n' "$WATCH_LOG"
  cat "$WATCH_LOG"
  WATCH_PID=""
}
if [ ${#FRAMES[@]} -gt 0 ] && [ -z "$FLOW" ]; then
  start_watch "$FRAMES_LOG" "${FRAMES[@]}"
fi
watch_line() { # speaker, line
  [ -n "$WATCH_PID" ] && [ -n "$2" ] && printf '%s | %s\n' "$1" "$2" >&4
}

# A scripted scene names its keeper and where it is; a flowing one has them per place.
if [ -z "$FLOW" ] && { [ -z "$KEEPER" ] || [ -z "$KEEPER_SITUATION" ] ||
  [ -z "$SPEAKER_SITUATION" ]; }; then
  echo "$BEATS names no @keeper, @keeper_situation and @speaker_situation" >&2
  exit 2
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
# do not answer twice (run 69: a question to someone, then their own examination). An answer that
# itself puts a question to someone gets one reply from them (`keeper follow`; run 71: a keeper's
# question went unanswered as the next beat moved on), and only when the judge names someone.
keeper() { # [follow]
  local follow=$1 out line answerer situation
  recent_args
  # The judge is told what the speaker set out to do (the beat's goal, where they are): whom
  # they meant the line for counts as much as its words. A reply (`reply=true`) is put to someone
  # only when it leaves them a question or a demand to answer (run 73: an order was restated after
  # its listener simply agreed to it).
  local reply=false
  [ -n "$follow" ] && reply=true
  # A flowing move names whom it is put to (`MOVE_TO`, an id the storyteller chose from who is
  # there): they answer it (run 78: the judge gave answers to the wrong one of two lookalikes).
  if [ -z "$follow" ] && [ -n "$MOVE_TO" ] && [ "$MOVE_TO" != room ]; then
    answerer=$MOVE_TO
  else
    answerer=$(timeout 120 $D dev --port $P persona addressed "$LAST_SPEAKER" "${RECENT[@]}" \
      "intent=$SPEAKER_SITUATION $LAST_INTENT" "reply=$reply" "$LAST" 2>/dev/null | head -1)
  fi
  case "$answerer" in
    ''|nobody|"$LAST_SPEAKER"|*' '*)
      [ -n "$follow" ] && return
      answerer=$KEEPER ;;
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
  [ -n "$follow" ] || [ -z "$line" ] && return
  LAST="$line"
  LAST_SPEAKER="$answerer"
  LAST_INTENT=""
  keeper follow
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

# The frame options for a place: the shared ones, `frames=<name>-<place>`, and its scene joined by
# the plot's set dressing there.
place_frames() { # place
  local option dressing
  dressing=$($D dev --port $P persona story beats "$1" 2>/dev/null | sed -n 's/^@dressing //p' |
    paste -sd ';' -)
  PLACE_FRAMES=()
  for option in "${FRAMES[@]}"; do
    case "$option" in
      frames=*) PLACE_FRAMES+=("$option-$1") ;;
      scene=*) ;;
      *) PLACE_FRAMES+=("$option") ;;
    esac
  done
  PLACE_FRAMES+=("scene=${PLACE_SCENE[$1]}${dressing:+ In sight there: $dressing.}")
}

# Play moves to a place: its plate, keeper and situations, and a frame watch of its own.
enter_place() { # place
  PLACE=$1
  $D dev --port $P scene image tv-backdrop "${PLACE_PLATE[$1]}" > /dev/null
  KEEPER=${PLACE_KEEPER[$1]:-$KEEPER}
  KEEPER_SITUATION=${PLACE_KEEPER_SITUATION[$1]:-$KEEPER_SITUATION}
  SPEAKER_SITUATION=${PLACE_SITUATION[$1]:-$SPEAKER_SITUATION}
  printf '\n=== %s (%s)\n' "$1" "${PLACE_PLATE[$1]}"
  if [ ${#FRAMES[@]} -gt 0 ]; then
    place_frames "$1"
    start_watch "${FRAMES_LOG%.log}-$1.log" "${PLACE_FRAMES[@]}"
  fi
}

# Flowing play: the storyteller's next move each turn, from everything said so far.
play_flow() {
  local out speaker to goal move_to end moves=0 line all opening=true after=""
  enter_place "$FLOW"
  while [ "$moves" -lt "$FLOW_MOVES" ]; do
    moves=$((moves + 1))
    all=()
    for line in "${HISTORY[@]}"; do all+=("recent=$line"); done
    # Someone already there opens a place; after anyone who is not a player, a player moves
    # (the route checks it).
    out=$(timeout 300 $D dev --port $P persona story next "opening=$opening" \
      ${after:+"after=$after"} "${all[@]}" "$PLACE" 2>&1)
    speaker=$(printf '%s\n' "$out" | sed -n 's/^speaker=//p')
    to=$(printf '%s\n' "$out" | sed -n 's/^to=//p')
    goal=$(printf '%s\n' "$out" | sed -n 's/^goal=//p')
    move_to=$(printf '%s\n' "$out" | sed -n 's/^move_to=//p')
    end=$(printf '%s\n' "$out" | sed -n 's/^end=//p')
    if [ -z "$speaker" ]; then
      printf '       <no move: %s>\n' "$out"
      return
    fi
    # The move as the storyteller set it, so a line can be checked against what it was asked.
    printf '       > %s to %s: %s\n' "$speaker" "$to" "$goal"
    opening=false
    after=$speaker
    say_as "$speaker" "$goal"
    NEXT_SPEAKER=""
    MOVE_TO=$to
    keeper
    MOVE_TO=""
    if [ "$end" = true ]; then
      printf '\n=== the story ends\n'
      return
    fi
    if [ -n "$move_to" ] && [ "$move_to" != "-" ]; then
      stop_watch
      enter_place "$move_to"
      opening=true
      after=""
    fi
  done
  printf '\n=== stopped after %s moves (@flow_moves)\n' "$FLOW_MOVES"
}
[ -n "$FLOW" ] && play_flow

stop_watch
