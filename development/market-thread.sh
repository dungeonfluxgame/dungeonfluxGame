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
# (a stance or a rule given to a persona for the scene). `@frames <option>` lines (one
# `persona watch` option each: `frames=<name>`,
# `scene=<what the picture shows>`, `hold=30`, ...) let the picture follow the thread: every line
# said is handed to `persona watch`, whose transcript is printed at the end and kept in
# $FRAMES_LOG. `@backfill sync` has the keeper's missing details (how a blade looks up close)
# backfilled before the answer, so the answer can say them (`persona ask backfill=sync`).
#
# Flowing play (`@flow <first place>`): no beats are scripted. Each turn asks the storyteller for
# the next move from everything said so far (`persona story next`), plays it and lets whoever it was
# put to answer, until the story's end is reached. Each place of the story has `@place <id>
# <plate>`, `@place_keeper <id> <keeper id>`, `@place_keeper_situation <id> <text>`,
# `@place_situation <id> <text>` and `@place_scene <id> <what the plate shows>` lines; on a move
# that sets off for a place, its plate is shown, its situations and keeper are taken, and the frame
# watch starts again there (`<frames>-<place>`), its scene joined by the plot's set dressing for
# that place (`persona story beats <place>`). After every line the clue judge says what it found of
# the plot's evidence (`persona story clues`): a piece seen is shown up close on screen (`@focus`
# to the frame watch) and what careful study can now work out joins everyone's situation. The host
# keeps what was seen and worked out in the stored story, where the storyteller and the narrator
# read it; `persona story reset` (or restoring story.json) starts a run clean. The plot's score for
# a place (`@music`) plays
# there (`scene music generate`, one track per place). The dungeon master, the narrator, may set
# the scene on arrival and say what a piece newly seen or worked out adds up to, between lines
# (`persona story narrate`, shown and voiced by `persona narrate`, printed as `DM:`), and gives a
# hint when the clue judge finds the party stuck (`DM (hint):`). A served place is not left at
# once: the host puts play on the way to the next place (the plot's `ways`, two or three stages
# each) and says after every move what comes next (`persona story phase`); each stage shows its
# plate made before play (`persona story way-plates`) at once, gets later plates of its own from
# the frame watch, a word from the narrator (`approach`) and a move or two, and only after the
# last does the next place's plate, music and arrival come.
cd "$(dirname "$0")/.."
D=${DF_TABLE_EXE:-./artifacts/build/bubbles/debug/df-table.exe}
P=${DF_TABLE_PORT:-7879}
# How long a model-backed command may take before the run calls it hung: the host bounds each model
# call itself (600 s), and a line's chain is several calls (dungeon run 13: at 60 s a call, a
# reply cut at 280 s came back empty).
HUNG=1800
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
PLACE=""
# What careful study of what was found can work out now (the host keeps what was found).
WORKABLE=""
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

# Shows a found clue up close: its reference sheet in the story (`persona story item`, kept or
# designed now) is what the close-up is drawn from, so the thing looks the same each time it is
# shown; the clue's words are its caption.
focus_on() { # what a close look shows
  [ -z "$WATCH_PID" ] && return
  local item
  item=$($D dev --port $P persona story item "$PLACE" "$1" 2>/dev/null | sed -n 's/^item=//p')
  printf '       ~ sheet %s\n' "${item:-<none>}"
  printf '@focus %s%s\n' "${item:+$item | }" "$1" >&4
}

# What a line said in flowing play found of the plot's evidence (the host commits it to the stored
# story): each piece seen is shown up close with what it shows (`@focus` to the frame watch), and
# what can now be worked out is kept for the situations.
clue_check() { # speaker, line
  # Nothing of the story's evidence is on show on the way between places.
  { [ -z "$PLACE" ] || [ -z "$2" ] || [ -n "$STAGE" ]; } && return
  local out found id text
  if ! out=$($D dev --port $P persona story clues "$PLACE" "$1: $2" 2>&1); then
    printf '       <clues: %s>\n' "$out"
    return
  fi
  while IFS= read -r found; do
    id=${found#*=}
    text=${id#* }
    id=${id%% *}
    case "$found" in
      seen=*)
        printf '       ? seen %s: %s\n' "$id" "$text"
        [ "$MOMENT" = worked_out ] || MOMENT=found
        focus_on "$text" ;;
      worked_out=*)
        MOMENT=worked_out
        printf '       ? worked out %s: %s\n' "$id" "$text" ;;
      # The party is stuck here (the host counts the player lines since the last piece seen or
      # worked out, and says so once a stretch): the dungeon master gives a hint.
      stalled=true) [ -n "$MOMENT" ] || MOMENT=stalled ;;
    esac
  done <<< "$out"
  WORKABLE=$(printf '%s\n' "$out" | sed -n 's/^workable=[^ ]* //p' | paste -sd ' ' -)
}

# A situation with what careful study of what was seen can work out now, when there is any: only
# the one who makes a move is told it, so the players work things out rather than whoever answers
# (dungeon run 1: the torchbearer answered with both findings).
studied() { # situation
  printf '%s%s' "$1" "${WORKABLE:+ What careful study of what was found can show now: $WORKABLE}"
}

# The dungeon master, the narrator (flowing play only): at a moment of play (arrival at a place, a
# piece newly seen or newly worked out, `MOMENT` kept by the clue check until the turn's line is
# done) it is asked for a word (`persona story narrate`, from what was found, the lines said at
# the place and its own lines there, which the story keeps, so each adds something new), keeps
# silent or says one, which is shown in its box and voiced (`persona narrate`). Its word is
# composed in the background while the next move is planned (dungeon run 16: composing it took 66
# to 112 s, and play waited for it), and spoken just before the next line is voiced
# (`narrate_spoken`), once the last line is heard out; the run waits until its own line is heard
# before the next one: no voice cuts across another. When the party is stuck (`stalled`), its word
# is a hint, printed as `DM (hint):`.
MOMENT=""
NARRATING=""
NARRATING_MOMENT=""
NARRATED=${FRAMES_LOG%.log}-narrated.txt
HEARD=0
heard_out() { # line just voiced: when it has been heard out (3 s, as after every line, and 70 ms a
  # character for the voice)
  HEARD=$((SECONDS + 3 + ${#1} * 70 / 1000))
}
wait_heard() {
  local left=$((HEARD - SECONDS))
  [ "$left" -gt 0 ] && sleep "$left"
}
narrate() { # moment: starts composing the narrator's word, from play as it stands now
  MOMENT=""
  { [ -z "$FLOW" ] || [ -z "$PLACE" ]; } && return
  narrate_spoken
  recent_args "${ARRIVED:-0}"
  timeout $HUNG $D dev --port $P persona story narrate "${RECENT[@]}" "$PLACE" \
    "$1" > "$NARRATED" 2>&1 &
  NARRATING=$!
  NARRATING_MOMENT=$1
}
narrate_spoken() { # speaks the narrator's word being composed, if any, once it is ready
  [ -z "$NARRATING" ] && return
  local line out
  if ! wait "$NARRATING"; then
    NARRATING=""
    printf '       <narrate: %s>\n' "$(cat "$NARRATED")"
    return
  fi
  NARRATING=""
  line=$(cat "$NARRATED")
  [ -z "$line" ] && return
  wait_heard
  printf '       DM%s: %s\n' "$([ "$NARRATING_MOMENT" = stalled ] && printf ' (hint)')" "$line"
  if ! out=$($D dev --port $P persona narrate speak=true "$line" 2>&1); then
    printf '       <narrate: %s>\n' "$out"
    return
  fi
  heard_out "$line"
  wait_heard
}

# A scripted scene names its keeper and where it is; a flowing one has them per place.
if [ -z "$FLOW" ] && { [ -z "$KEEPER" ] || [ -z "$KEEPER_SITUATION" ] ||
  [ -z "$SPEAKER_SITUATION" ]; }; then
  echo "$BEATS names no @keeper, @keeper_situation and @speaker_situation" >&2
  exit 2
fi

# The last twenty lines of the thread, as repeated `recent=` options (a short window let lines
# repeat what was said a few exchanges back), none before line $1 when it is given.
recent_args() { # [first line]
  local start=$(( ${#HISTORY[@]} > 20 ? ${#HISTORY[@]} - 20 : 0 )) i
  [ "$start" -lt "${1:-0}" ] && start=$1
  RECENT=()
  for ((i = start; i < ${#HISTORY[@]}; i++)); do RECENT+=("recent=${HISTORY[$i]}"); done
}

say_as() { # speaker, instruction, [strike]
  local who="$1" instruction="$2" strike="${3:-false}" out line
  narrate_spoken
  recent_args
  # Every line is voiced when the playground's TTS is on (run 97: no line was ever voiced). The
  # storyteller's strike move is passed typed (`strike=true`): the line where the speaker's
  # purpose shows (run 24: the culprit's guidance kept him hedging, so it never struck).
  out=$(timeout $HUNG $D dev --port $P persona say "$who" speak=true "${RECENT[@]}" \
    "strike=$strike" "intent=$instruction" "$(studied "$SPEAKER_SITUATION")" 2>&1)
  line=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  N=$((N + 1))
  printf '[%02d] %s: %s\n' "$N" "$who" "${line:-<no line: $out>}"
  printf '%s\n' "$out" | sed -n '2,$p' | grep -E 'quoted|fired triggers' | sed 's/^/       /'
  LAST="$line"
  LAST_SPEAKER="$who"
  LAST_INTENT="$instruction"
  HISTORY+=("$who: $line")
  heard_out "$line"
  watch_line "$who" "$line"
  clue_check "$who" "$line"
  sleep 3
  [ -n "$MOMENT" ] && narrate "$MOMENT"
}

# The keeper answers the line actually said: they see only what the speaker said, never what the
# beat hoped they would do, so a speaker whose persona refuses a beat cannot put words in theirs.
# The one the line was put to answers it: the addressee judge (`persona addressed`, a model call
# against the scene's speakers) reads whom it is directed at; nobody, the speaker themself or no
# answer leaves it to the keeper. When the next beat is theirs, that beat is their answer, so they
# do not answer twice (run 69: a question to someone, then their own examination). An answer that
# itself puts a question to someone gets a reply from them (run 71: a keeper's question went
# unanswered as the next beat moved on), and that reply, when it asks again, an answer (runs 89
# and 90: an unanswered follow-up was asked again by the next player); a reply comes only when
# the judge names someone. No line is answered when the judge finds no answer owed: an answer
# that would only agree or say the line back. `follow` is how many replies are left, set on the
# replies only.
keeper() { # [follow]
  local follow=$1 out line answerer situation
  # Whoever answers hears only what was said where they are (runs 94 and 96: people at the inn
  # spoke of what was said at the market); the one who makes a move remembers it all.
  recent_args "${ARRIVED:-0}"
  # The judge is told what the speaker set out to do (the beat's goal, where they are): whom
  # they meant the line for counts as much as its words. A reply (`reply=true`) is put to someone
  # only when it leaves them a question or a demand to answer (run 73: an order was restated after
  # its listener simply agreed to it).
  local reply=false judged
  [ -n "$follow" ] && reply=true
  judged=$(timeout $HUNG $D dev --port $P persona addressed "$LAST_SPEAKER" "${RECENT[@]}" \
    "intent=$SPEAKER_SITUATION $LAST_INTENT" "reply=$reply" "$LAST" 2>/dev/null)
  # The judge also says whether an answer would add anything the talk needs (a fact, a decision,
  # an action, the answer to a question or a demand): when it would only acknowledge or echo the
  # line, nobody answers (dungeon runs: a torchbearer said "Yes, sir" to nearly every player line,
  # doubling the lines at each place). A judge that gave no answer leaves the answer owed.
  if [ "$(printf '%s\n' "$judged" | sed -n 's/^answer_owed=//p')" = false ]; then
    printf '       - no answer owed\n'
    return
  fi
  answerer=$(printf '%s\n' "$judged" | head -1)
  # A flowing move names whom it is put to (`MOVE_TO`, an id the storyteller chose from who is
  # there): they answer it (run 78: the judge gave answers to the wrong one of two lookalikes).
  if [ -z "$follow" ] && [ -n "$MOVE_TO" ] && [ "$MOVE_TO" != room ]; then
    answerer=$MOVE_TO
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
  narrate_spoken
  out=$(timeout $HUNG $D dev --port $P persona ask "$answerer" speak=true "from=$LAST_SPEAKER" \
    "${RECENT[@]}" "backfill=$KEEPER_BACKFILL" "situation=$situation" "$LAST" 2>&1)
  line=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  N=$((N + 1))
  printf '[%02d] %s: %s\n' "$N" "$answerer" "${line:-<no line: $out>}"
  printf '%s\n' "$out" | sed -n '2,$p' | sed 's/^/       /'
  HISTORY+=("$answerer: $line")
  heard_out "$line"
  watch_line "$answerer" "$line"
  clue_check "$answerer" "$line"
  sleep 3
  [ -n "$MOMENT" ] && narrate "$MOMENT"
  [ -z "$line" ] && return
  [ -n "$follow" ] && [ "$follow" -le 1 ] && return
  LAST="$line"
  LAST_SPEAKER="$answerer"
  LAST_INTENT=""
  keeper $((${follow:-3} - 1))
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

# This run's own stamp on its frames' names: a frame name already in the library is shown again
# rather than made, so a run that took an earlier run's names (or a plate's) showed old pictures.
RUN_STAMP=$(date +%m%d%H%M)
# The frame options for a place: the shared ones, `frames=<name>-<stamp>-<place>`, and its scene
# joined by the plot's set dressing there.
place_frames() { # place
  local option dressing
  dressing=$($D dev --port $P persona story beats "$1" 2>/dev/null | sed -n 's/^@dressing //p' |
    paste -sd ';' -)
  PLACE_FRAMES=()
  for option in "${FRAMES[@]}"; do
    case "$option" in
      frames=*) PLACE_FRAMES+=("$option-$RUN_STAMP-$1") ;;
      scene=*) ;;
      *) PLACE_FRAMES+=("$option") ;;
    esac
  done
  PLACE_FRAMES+=("scene=${PLACE_SCENE[$1]}${dressing:+ In sight there: $dressing.}" "place=$1")
}

# Play moves to a place: its plate, keeper and situations, and a frame watch of its own.
enter_place() { # place
  PLACE=$1
  STAGE=""
  # The lines said before play came here: a place is left only once every player has spoken there.
  ARRIVED=${#HISTORY[@]}
  $D dev --port $P scene image tv-backdrop "${PLACE_PLATE[$1]}" > /dev/null
  KEEPER=${PLACE_KEEPER[$1]:-$KEEPER}
  KEEPER_SITUATION=${PLACE_KEEPER_SITUATION[$1]:-$KEEPER_SITUATION}
  SPEAKER_SITUATION=${PLACE_SITUATION[$1]:-$SPEAKER_SITUATION}
  # What is on show there (the plot's exhibits for the place), as anyone sees it up close.
  local shown
  shown=$($D dev --port $P persona story beats "$1" 2>/dev/null | sed -n 's/^@exhibit //p' |
    paste -sd ' ' -)
  if [ -n "$shown" ]; then
    KEEPER_SITUATION="$KEEPER_SITUATION On show before everyone: $shown"
    SPEAKER_SITUATION="$SPEAKER_SITUATION On show before everyone: $shown"
  fi
  # Each thing on show gets its reference sheet now (`persona story item`), so every plate of the
  # place draws it the same way from the first one on; designed while play goes on (one at a time,
  # as the route holds them), never holding up the first line.
  local exhibit
  while IFS= read -r exhibit; do
    [ -n "$exhibit" ] && $D dev --port $P persona story item "$1" "$exhibit" > /dev/null 2>&1
  done < <($D dev --port $P persona story beats "$1" 2>/dev/null | sed -n 's/^@exhibit //p') &
  printf '\n=== %s (%s)\n' "$1" "${PLACE_PLATE[$1]}"
  # The place's score from the plot, a new track only when play comes to a new place: it is named
  # after the place's plate, so a place played again (or a replay) reuses its track.
  local music plate=${PLACE_PLATE[$1]}
  music=$($D dev --port $P persona story beats "$1" 2>/dev/null | sed -n 's/^@music //p' | head -1)
  if [ -n "$music" ]; then
    $D dev --port $P scene music generate "as=music-${plate%.*}" "$music" 2>&1 |
      sed 's/^/       ~ /' &
  fi
  if [ ${#FRAMES[@]} -gt 0 ]; then
    place_frames "$1"
    start_watch "${FRAMES_LOG%.log}-$1.log" "${PLACE_FRAMES[@]}"
  fi
  # Whoever this place's picture shows gets their part of the plot now: a character who first
  # appears here (a culprit waiting at the last place) was left out of the earlier apply, and
  # without it plays as a stranger with no purpose (dungeon run 24: the rival was "left for later").
  # Asked once more after a short wait before it counts as failed: a model refusal (dungeon run 25:
  # a backfill judged to give away a secret) passes on a second ask, and who was given their part
  # is not asked again.
  $D dev --port $P persona story apply > /dev/null 2>&1 ||
    { sleep 5; $D dev --port $P persona story apply > /dev/null 2>&1; } ||
    echo '       <story apply failed>'
  # The dungeon master sets the scene once the place's plate is up, before the first move.
  narrate arrival
}

# Play goes on to a stage of the way: plates of the stage (its sight, the thing worth a look and the
# party, as the host words them), what everyone says there set by it, and the narrator's word.
STAGE=""
enter_stage() { # stage id, what its plates show, where everyone is, its plate made before play
  STAGE=$1
  # The lines said before play came to this stage: what the stage judge weighs is said after.
  ARRIVED=${#HISTORY[@]}
  KEEPER_SITUATION=$3
  SPEAKER_SITUATION=$3
  printf '\n=== on the way: %s%s\n' "$1" "${4:+ ($4)}"
  # The stage's plate made before play goes up at once, as a place's does, before its frame watch
  # reads who is in the picture: a fresh plate takes 90 to 400 s, and most stages were over before
  # theirs came (dungeon run 24). A stage without one keeps the picture up until its watch paints.
  if [ -n "$4" ]; then
    $D dev --port $P scene image tv-backdrop "$4" > /dev/null
  fi
  if [ ${#FRAMES[@]} -gt 0 ]; then
    local option
    PLACE_FRAMES=()
    for option in "${FRAMES[@]}"; do
      case "$option" in
        frames=*) PLACE_FRAMES+=("$option-$RUN_STAMP-$1") ;;
        scene=*) ;;
        *) PLACE_FRAMES+=("$option") ;;
      esac
    done
    start_watch "${FRAMES_LOG%.log}-$1.log" "${PLACE_FRAMES[@]}" "scene=$2"
  fi
  narrate approach
}

# What comes next, as the host keeps it (`persona story phase`): play stays where it is, goes on to
# a stage of the way or arrives at the next place. The runner shows what the host says; it never
# decides it. Sets ENTERED to `place` on an arrival.
onward() {
  local out at key line all=()
  ENTERED=""
  for line in "${HISTORY[@]}"; do all+=("recent=$line"); done
  if ! out=$(timeout $HUNG $D dev --port $P persona story phase "arrived=$ARRIVED" "${all[@]}" 2>&1)
  then
    printf '       <phase: %s>\n' "$out"
    return 1
  fi
  at=$(printf '%s\n' "$out" | sed -n 's/^at=//p')
  case "$at" in
    place) key=$(printf '%s\n' "$out" | sed -n 's/^place=//p') ;;
    way) key=$(printf '%s\n' "$out" | sed -n 's/^stage=//p') ;;
    *) return 0 ;;
  esac
  # Nothing new: still at this place, or at this stage of the way.
  [ "$key" = "${STAGE:-$PLACE}" ] && return 0
  narrate_spoken
  stop_watch
  if [ "$at" = place ]; then
    enter_place "$key"
    ENTERED=place
  else
    enter_stage "$key" "$(printf '%s\n' "$out" | sed -n 's/^scene=//p')" \
      "$(printf '%s\n' "$out" | sed -n 's/^situation=//p')" \
      "$(printf '%s\n' "$out" | sed -n 's/^plate=//p')"
  fi
}

# Flowing play: the storyteller's next move each turn, from everything said so far.
play_flow() {
  local out speaker to goal served end strike line all opening=true after=""
  enter_place "$FLOW"
  while true; do
    all=()
    for line in "${HISTORY[@]}"; do all+=("recent=$line"); done
    # Someone already there opens a place; after anyone who is not a player, a player moves
    # (the route checks it).
    out=$($D dev --port $P persona story next "opening=$opening" \
      ${after:+"after=$after"} "arrived=$ARRIVED" "${all[@]}" "$PLACE" 2>&1)
    speaker=$(printf '%s\n' "$out" | sed -n 's/^speaker=//p')
    to=$(printf '%s\n' "$out" | sed -n 's/^to=//p')
    goal=$(printf '%s\n' "$out" | sed -n 's/^goal=//p')
    # What kept earlier answers from standing, so the log shows why a move took long.
    printf '%s\n' "$out" | sed -n 's/^refused=/       ! refused: /p'
    served=$(printf '%s\n' "$out" | sed -n 's/^served=//p')
    end=$(printf '%s\n' "$out" | sed -n 's/^end=//p')
    strike=$(printf '%s\n' "$out" | sed -n 's/^strike=//p')
    if [ -z "$speaker" ]; then
      printf '       <no move: %s>\n' "$out"
      return
    fi
    # The move as the storyteller set it, so a line can be checked against what it was asked.
    printf '       > %s to %s: %s%s%s\n' "$speaker" "$to" "$goal" \
      "$([ "$served" = true ] && printf ' (place served)')" \
      "$([ "$strike" = true ] && printf ' (strike)')"
    opening=false
    after=$speaker
    say_as "$speaker" "$goal" "${strike:-false}"
    NEXT_SPEAKER=""
    MOVE_TO=$to
    keeper
    MOVE_TO=""
    if [ "$end" = true ]; then
      narrate_spoken
      printf '\n=== the story ends\n'
      # What play found, as the host kept it.
      $D dev --port $P persona story show 2>/dev/null |
        sed -n 's/^  \(seen\|worked out\) /       found: \1 /p'
      return
    fi
    # A served place is left for the way to the next, and each stage of it for the next stage
    # or the arrival, as the host says.
    onward || return
    if [ "$ENTERED" = place ]; then
      opening=true
      after=""
    fi
  done
}
[ -n "$FLOW" ] && play_flow

stop_watch
