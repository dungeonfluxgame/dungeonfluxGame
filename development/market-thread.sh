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
#
# Free roam (`@roam <id>`): one character and nobody answering. `@inventory <items.json>` sets what
# they hold at the start (`world inventory set <id> character <items.json>`); `@move <way>` lines are the moves in order and
# `@way <way> <situation>` says what each kind of move is; `@way_arrival <way> <situation>` says what
# they do on arriving by that kind of move, in place of the declaration and the study painting (a
# trade: `[trade NN] <line>`, judged against what they hold, then `gained: ..` or `no gain recorded`).
# Other kinds of move are as described below. Before each move the character says
# where they go next toward `@roam_goal` (`move act ... plate=NAME`: Jev reads the move, the place
# is designed and painted, and it becomes the active place with the things in sight there); then a
# frame watch of the new plate starts and they say one line toward `@roam_look` (`persona say`),
# whose close look the frame judge cuts to as an insert; the run waits until the insert has ended
# and the room is back, then (with `@roam_paint <situation>`) they say one line toward painting
# what they studied with what they hold, which the per-line judge turns into inventory changes;
# then it moves on. One `[move NN]` line per move (the line, Jev's reading, the
# place, the plate, the stage times of the move's trace) and one `[insert NN]` line per insert (the
# item, when it was shown and ended, from the host's `frame_timing` lines), one `[look]` and
# `[paint]` line for what they said, one `[refs NN]` line per move (what the design showed and
# the pictures the plate was drawn after or left out for want of room, from the host's `IMAGEGEN
# references` line), one `[inventory]` line per change to what they hold (the item, before and
# after, why and the line; `[inventory] refused` for a change the host refused) and one
# `[inventory NN]` line per move with what they held before and after.
#
# Free roam perception (`@roam_perceive on`): once a move's plate is live and before the look line,
# the character looks at the screen (`persona perceive`: the picture is sent to the model with the
# scan and their own state), `[see NN]` is what they see and what they attend to and `[thought NN]`
# what they are thinking about. That thought and what they attend to join the situation of the look
# and paint lines, so what they say is about what is really on screen.
#
# Free roam declares what the hands are doing (`@roam_declare <situation>`): after the look (and the
# perception before it) the character declares the physical action they are doing right now
# (`persona declare`), in the first person and the present tense, and that declaration is their
# painting line: it is spoken and judged for the supplies it uses like `@roam_paint`'s line (which it
# replaces), and handed to the frame watch as an emergency plate, a picture of the scene showing the
# action in progress that is made ahead of everything else and shown at once. `[declare NN] <words>`
# is what they said; once the plate has been up for its hold (`@roam_declare_hold <seconds>`, the
# watch's `declare_hold=`) a second `[declare NN] plate=.. shown_at_ms=.. made_ms=.. queue_wait_ms=..
# superseded=N ended_at_ms=.. up_ms=..` line says when it was shown and ended, from the host's
# `frame_timing` and `PERSONA declared` lines. The study painting of `@roam_art_*` is still made after
# it, as after a paint line. The situation says what to do without supplies; the facts decide.
#
# Free roam keeps memories (the host's standard memory API, `world memory ...`): with
# `@roam_scene_kind <kind>` every plate that goes live is captured as a memory of that kind (the
# place, the plate), `[memory NN] captured ..`; with `@roam_art_kind <kind>` the line toward
# `@roam_paint` (or the declaration) makes a painting, but only after a judged line toward
# `@roam_painted` (`[painted NN]`: the per-line judge reads it against what they hold, so the
# board, pigments, oil and rags are used up) used up the inventory item `@roam_art_board`; if it
# did not, the log says `not painted` and why. The move's plate is edited into a study of the
# place (`@roam_art_look` is its look statement and `{thing}` in `@roam_art_title` the thing
# studied; the plate is its only reference, the board is described in words), stored as
# `item-<owner>-<@roam_art_item>_NN.png`, checked for blankness (`world memory inspect`; a blank
# study is made once more with `@roam_art_retry` added, and if still blank is removed and the log
# says `study blank, not stored`; the board is used up either way), kept as
# a memory of the art kind linked to its item and place, and added to what they hold as a kept item
# `<@roam_art_item>_NN`, `[memory NN] painted ..`. `@roam_show <situation>` (last in the file) ends
# the day: a move to where the work is shown, every memory of the art kind composed into one gallery
# sheet (`[show] composed gallery of N pieces -> <name>`), the show plate edited from the move's
# plate after that sheet and the traveller's sheet (`@roam_show_look` is its look statement), and
# one line toward `@roam_show_say`.
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
ROAM=""
INVENTORY=""
ROAM_GOAL=""
ROAM_LOOK=""
ROAM_PAINT=""
ROAM_PAINTED=""
ART_RETRY=""
ROAM_DECLARE=""
ROAM_DECLARE_HOLD=""
SCENE_KIND=""
ART_KIND=""
ART_ITEM=""
ART_BOARD=""
ART_TITLE=""
ART_LOOK=""
ROAM_SHOW=""
ROAM_SHOW_LOOK=""
ROAM_SHOW_SAY=""
ROAM_PERCEIVE=""
MIND=""
MOVES=()
declare -A PLACE_PLATE PLACE_KEEPER PLACE_KEEPER_SITUATION PLACE_SITUATION PLACE_SCENE WAY ARRIVAL
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
    '@roam '*) ROAM=$value ;;
    '@inventory '*) INVENTORY=$value ;;
    '@roam_goal '*) ROAM_GOAL=$value ;;
    '@roam_look '*) ROAM_LOOK=$value ;;
    '@roam_paint '*) ROAM_PAINT=$value ;;
    '@roam_painted '*) ROAM_PAINTED=$value ;;
    '@roam_art_retry '*) ART_RETRY=$value ;;
    '@roam_declare '*) ROAM_DECLARE=$value ;;
    '@roam_declare_hold '*) ROAM_DECLARE_HOLD=$value ;;
    '@roam_perceive '*) ROAM_PERCEIVE=$value ;;
    '@roam_scene_kind '*) SCENE_KIND=$value ;;
    '@roam_art_kind '*) ART_KIND=$value ;;
    '@roam_art_item '*) ART_ITEM=$value ;;
    '@roam_art_board '*) ART_BOARD=$value ;;
    '@roam_art_title '*) ART_TITLE=$value ;;
    '@roam_art_look '*) ART_LOOK=$value ;;
    '@roam_show '*) ROAM_SHOW=$value ;;
    '@roam_show_look '*) ROAM_SHOW_LOOK=$value ;;
    '@roam_show_say '*) ROAM_SHOW_SAY=$value ;;
    '@way '*) WAY[${value%% *}]=${value#* } ;;
    '@way_arrival '*) ARRIVAL[${value%% *}]=${value#* } ;;
    '@move '*) MOVES+=("$value") ;;
  esac
done < <(grep '^@' "$BEATS")
# The day's show is the last move of a roam: a move like the others, ended differently.
if [ -n "$ROAM_SHOW" ]; then
  MOVES+=("_show_")
  WAY[_show_]=$ROAM_SHOW
fi
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
stop_watch() { # [quiet: the transcript stays in its log]
  [ -z "$WATCH_PID" ] && return
  exec 4>&-
  wait "$WATCH_PID"
  if [ "$1" != quiet ]; then
    printf '\n--- frames (%s) ---\n' "$WATCH_LOG"
    cat "$WATCH_LOG"
  fi
  WATCH_PID=""
}
if [ ${#FRAMES[@]} -gt 0 ] && [ -z "$FLOW" ] && [ -z "$ROAM" ]; then
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
if [ -z "$FLOW" ] && [ -z "$ROAM" ] && { [ -z "$KEEPER" ] || [ -z "$KEEPER_SITUATION" ] ||
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

# Free roam: one `[inventory]` line per change a line made to what is held, with the line.
roam_inventory() { # line, output of the command that said it
  local change
  printf '%s\n' "$2" | sed -n 's/^ *inventory: //p' | while IFS= read -r change; do
    printf '[inventory] %s | %s\n' "$change" "$1"
  done
}
# What `$ROAM` holds now, one `<item id>=<count>` per line.
held_counts() {
  $D dev --port $P world inventory get "$ROAM" 2>/dev/null |
    sed -n 's/^  \([^ ]*\) x\([0-9]*\): .*$/\1=\2/p'
}
# The items whose counts differ between two `held_counts` snapshots, as `<id> <before>-><after>`
# separated by commas, or `unchanged`.
held_diff() { # before, after
  local item count was changed=""
  while IFS== read -r item count; do
    was=$(printf '%s\n' "$1" | sed -n "s/^$item=//p" | head -1)
    [ "$was" != "$count" ] && changed="${changed:+$changed, }$item ${was:-0}->$count"
  done <<< "$2"
  printf '%s\n' "${changed:-unchanged}"
}
# The `[inventory] refused` lines of the changes a line made that the host refused (an item not
# held, a count below zero), from the host's `WORLD refused verb=inventory_judged` lines after
# event $1.
roam_refused() { # after-sequence, move number
  local sequence text
  while read -r sequence _ text; do
    case "$text" in
      'WORLD refused verb=inventory_judged '*)
        printf '[inventory] refused (move %02d): %s\n' "$2" "$(field_of "$text" error)" ;;
    esac
  done < <($D dev --port $P events "$1" area=WORLD 2>/dev/null | grep -E '^ *[0-9]+ ')
}
# The `[refs NN]` line of a move: the items the design showed (from the move's output) and the
# pictures its plate was drawn after, from the host's `IMAGEGEN references` line for the plate
# after event $1.
roam_refs() { # after-sequence, plate, move number, the move's output
  local shown sequence text line=""
  shown=$(printf '%s\n' "$4" | sed -n 's/^items in sight: //p' | head -1)
  while read -r sequence _ text; do
    case "$text" in
      "IMAGEGEN references job="*" name=$2 "*) line=$text ;;
    esac
  done < <($D dev --port $P events "$1" area=IMAGEGEN 2>/dev/null | grep -E '^ *[0-9]+ ')
  if [ -z "$line" ]; then
    printf '[refs %02d] shown=%s attached=none\n' "$3" "${shown:--}"
    return
  fi
  printf '[refs %02d] shown=%s base=%s attached=%s count=%s maximum=%s left_out=%s\n' "$3" \
    "${shown:--}" "$(field_of "$line" base)" "$(field_of "$line" attached)" \
    "$(field_of "$line" count)" "$(field_of "$line" maximum)" "$(field_of "$line" left_out)"
}
# The newest sequence number of the host's event log.
latest_event() {
  $D dev --port $P events limit=1 2>/dev/null | grep -E '^ *[0-9]+ ' | tail -1 | awk '{print $1}'
}
# The value of `key=` in an audit line.
field_of() { # line, key
  printf '%s\n' "$1" | tr ' ' '\n' | sed -n "s/^$2=//p" | head -1
}
# The at_ms of a stage's first record in a move's trace, or nothing.
stage_at() { # trace, stage
  printf '%s\n' "$1" | sed -n "s/^TRANSITION id=[0-9]* stage=$2 at_ms=\([0-9]*\) .*/\1/p" | head -1
}
# The milliseconds from $1 to $2, or - when either is unknown.
span() {
  if [ -n "$1" ] && [ -n "$2" ]; then echo $(($2 - $1)); else echo -; fi
}
# Waits for the close look a line after event $1 leads to in the watch of frames $2: the frame
# judge's answer, then the insert's shown and ended moments (`PERSONA frame_timing`), and prints
# the `[insert NN]` line.
roam_insert() { # after-sequence, frames name, move number, the design's thing lines
  local after=$1 frames=$2 number=$3 things=$4 item="" name="" frame="" shown="" made=""
  local deadline=$((SECONDS + HUNG)) sequence text
  while [ "$SECONDS" -lt "$deadline" ]; do
    sleep 5
    while read -r sequence _ text; do
      after=$sequence
      case "$text" in
        'PERSONA frame_judged '*'reason=line'*)
          [ -n "$item" ] && continue
          item=$(field_of "$text" inspected)
          name=$(printf '%s\n' "$things" | sed -n "s/^thing $item (\([^)]*\)).*/\1/p" | head -1)
          if [ "$(field_of "$text" inspection)" != true ]; then
            printf '[insert %02d] none: the judge saw no new close look (inspected=%s)\n' \
              "$number" "$item"
            return
          fi ;;
        'PERSONA frame_timing '*"frame=$frames-"*'reason=inspection'*)
          frame=$(field_of "$text" frame)
          case "$(field_of "$text" moment)" in
            shown)
              shown=$(field_of "$text" at_ms)
              made=$(field_of "$text" took_ms) ;;
            ended)
              printf '[insert %02d] item=%s (%s) frame=%s shown_at_ms=%s made_ms=%s ended_at_ms=%s up_ms=%s\n' \
                "$number" "$item" "${name:-?}" "$frame" "${shown:--}" "${made:--}" \
                "$(field_of "$text" at_ms)" "$(field_of "$text" took_ms)"
              return ;;
            failed)
              printf '[insert %02d] item=%s (%s) frame=%s failed after %s ms\n' \
                "$number" "$item" "${name:-?}" "$frame" "$(field_of "$text" took_ms)"
              return ;;
          esac ;;
        'PERSONA refused verb=frame '*)
          printf '[insert %02d] none: the frame judge refused (%s)\n' "$number" \
            "$(field_of "$text" error)"
          return ;;
      esac
    done < <($D dev --port $P events "$after" area=PERSONA 2>/dev/null | grep -E '^ *[0-9]+ ')
  done
  printf '[insert %02d] none: no close look within %s s\n' "$number" "$HUNG"
}

# Free roam memories: the plate that went live, kept as a memory of the place (kind `@roam_scene_kind`).
roam_remember_scene() { # move number, plate, place name, what the plate shows
  [ -z "$SCENE_KIND" ] && return
  local kept
  kept=$($D dev --port $P world memory capture "$ROAM" "$SCENE_KIND" "$2.png" "${3:-$2}" \
    "place=$2" "description=${4:-${3:-$2}}" by=runner 2>&1 | head -1)
  case "$kept" in
    'ok memory='*)
      printf '[memory %02d] captured %s %s seq=%s place=%s picture=%s.png (%s)\n' "$1" \
        "$SCENE_KIND" "$(field_of "$kept" memory)" "$(field_of "$kept" seq)" "$2" "$2" "${3:--}" ;;
    *) printf '[memory %02d] not captured: %s\n' "$1" "$kept" ;;
  esac
}
# Whether the media library holds the picture $1.
library_holds() {
  $D dev --port $P media 2>/dev/null | grep -qF "$1"
}
# The file a `media generate ... wait=true` stored, from its output.
stored_file() { # output of media generate
  printf '%s\n' "$1" | sed -n 's/^ok (generation [0-9]*) stored \([^ ]*\) in the media library.*/\1/p' | head -1
}
# What `$ROAM` holds of the item $1 now (0 when none).
item_count() {
  local count
  count=$(held_counts | sed -n "s/^$1=//p" | head -1)
  printf '%s\n' "${count:-0}"
}
# Free roam: the painting made of what they studied. The line toward `@roam_painted` is said first and
# judged like any line (the board painted on, the pigments, the oil and the rags are used up, as the
# per-line judge reads it against what they really hold). The study is made only when that used up
# the item of `@roam_art_board` (its count is below $6, what they held before the paint lines), so
# the board becomes the painting; otherwise the log says why. The move's plate is then edited into
# the study (that plate is its only reference), stored as
# `item-<owner>-<item>.png`, kept as a memory of kind `@roam_art_kind` linked to its item and place,
# and added to what they hold as a kept item.
# Arguments: move number, plate, place id, thing name, thing look, the board count before, place name,
# the line said before.
roam_paint_study() {
  [ -z "$ART_KIND" ] && return
  local number=$1 id file prompt title request try seen made stored kept done_line done_out now
  if [ -n "$ROAM_PAINTED" ]; then
    done_out=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true \
      "recent=$ROAM: $8" "intent=$ROAM_PAINTED" "You are at $7.$MIND" 2>&1)
    done_line=$(printf '%s\n' "$done_out" | sed -n '1s/.*says: //p')
    printf '[painted %02d] %s\n' "$number" "${done_line:-<no line: $done_out>}"
    roam_inventory "$done_line" "$done_out"
  fi
  now=$(item_count "$ART_BOARD")
  if [ -n "$ART_BOARD" ] && [ "${now:-0}" -ge "${6:-0}" ]; then
    printf '[memory %02d] not painted: no %s was used up (had %s, have %s)\n' "$number" \
      "$ART_BOARD" "${6:-0}" "$now"
    return
  fi
  id=$(printf '%s_%02d' "${ART_ITEM:-artwork}" "$number")
  file="item-$ROAM-$id"
  title=${ART_TITLE//\{thing\}/$4}
  prompt="$ART_LOOK The subject of the study: $5"
  # The plate is the reference for the place; the board is only described in words (a board's sheet
  # as a reference made the model paint just the board). A study the host finds near blank is made
  # once more with `@roam_art_retry` added to the request, and kept out of the memories if still blank.
  request=$prompt
  try=1
  while :; do
    made=$(timeout $HUNG $D dev --port $P media generate "$request" "as=$file" purpose=plate \
      "base=$2.png" wait=true look=none 2>&1)
    stored=$(stored_file "$made")
    if [ -z "$stored" ]; then
      printf '[memory %02d] not painted: %s\n' "$number" "$(printf '%s\n' "$made" | head -1)"
      return
    fi
    seen=$($D dev --port $P world memory inspect "$stored" 2>&1 | head -1)
    case "$seen" in
      'inspect: '*) ;;
      *) printf '[memory %02d] study not checked for blankness: %s\n' "$number" "$seen" ;;
    esac
    [ "$(field_of "$seen" blank)" = true ] || break
    if [ "$try" -ge 2 ]; then
      printf '[memory %02d] study blank, not stored (%s)\n' "$number" "$seen"
      $D dev --port $P media remove "$stored" >/dev/null 2>&1
      return
    fi
    printf '[memory %02d] study blank (%s), made once more\n' "$number" "$seen"
    request="$prompt $ART_RETRY"
    try=2
  done
  kept=$($D dev --port $P world memory capture "$ROAM" "$ART_KIND" "$stored" "$title" "id=$id" \
    "place=$3" "description=$prompt" "link=item:$id" "link=place:$3" by=runner 2>&1 | head -1)
  case "$kept" in
    'ok memory='*)
      printf '[memory %02d] painted %s seq=%s picture=%s after=%s plate=%s (%s)\n' "$number" \
        "$(field_of "$kept" memory)" "$(field_of "$kept" seq)" "$stored" "-" "$2.png" "$title" ;;
    *) printf '[memory %02d] painted %s but not kept as a memory: %s\n' "$number" "$stored" "$kept" ;;
  esac
  $D dev --port $P world inventory add "$ROAM" "$id" 1 "name=$title" "look=$prompt" use=kept \
    why=made 2>&1 | head -1 | sed "s/^/       ~ item $id: /"
}
# Free roam: the day's work shown. Every memory of kind `@roam_art_kind` is composed into one gallery
# sheet; the show plate is the move's plate edited after that sheet and the traveller's own sheet,
# put behind the scene with the traveller located in it, and the traveller says a line.
roam_show() { # move number, plate, place name, line said when moving
  [ -z "$ART_KIND" ] && return
  local composed gallery count refs shown line made scan region
  composed=$($D dev --port $P world memory compose "$ROAM" "kind=$ART_KIND" 2>&1 | head -1)
  gallery=$(field_of "$composed" picture)
  if [ -z "$gallery" ]; then
    printf '[show] no gallery: %s\n' "$composed"
    return
  fi
  count=$(field_of "$composed" members)
  printf '[show] composed gallery of %s pieces -> %s\n' "$count" "$gallery"
  refs=$gallery
  library_holds "ref-$ROAM.png" && refs="$refs,ref-$ROAM.png"
  made=$(timeout $HUNG $D dev --port $P media generate "$ROAM_SHOW_LOOK" "as=$2-show" \
    purpose=plate "base=$2.png" "refs=$refs" wait=true look=none 2>&1)
  shown=$(stored_file "$made")
  if [ -z "$shown" ]; then
    printf '[show] plate not made: %s\n' "$(printf '%s\n' "$made" | head -1)"
    return
  fi
  printf '[show] plate %s after %s\n' "$shown" "$refs"
  scan=$($D dev --port $P media locate "$shown" masks=1 2>&1)
  printf '%s\n' "$scan" | head -3 | sed 's/^/       ~ /'
  # Regions are numbered from 0: the traveller is the first region the scan reports.
  region=$(printf '%s\n' "$scan" | sed -n 's/^ *\([0-9][0-9]*\): .*/\1/p' | head -1)
  if [ -n "$region" ]; then
    $D dev --port $P media locate label "$shown" "$region" "$ROAM" "${ROAM^}" 2>&1 | head -1 |
      sed 's/^/       ~ /'
  else
    printf '[show] no person found in %s to name\n' "$shown"
  fi
  $D dev --port $P scene image tv-backdrop "$shown" 2>&1 | head -1 | sed 's/^/       ~ /'
  line=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true "recent=$ROAM: $4" \
    "intent=$ROAM_SHOW_SAY" "You are at $3." 2>&1)
  printf '[show] %s\n' "$(printf '%s\n' "$line" | sed -n '1s/.*says: //p')"
  roam_inventory "$(printf '%s\n' "$line" | sed -n '1s/.*says: //p')" "$line"
}

# The thing a `persona perceive` answer says is attended to: the words of the first `see:` line
# whose region or item is the answer's `attention:`, or nothing.
roam_attended() { # attention (region:<id>, item:<id> or none), output of persona perceive
  local kind=${1%%:*} id=${1#*:} line
  [ "$1" = none ] && return
  while IFS= read -r line; do
    case "$kind:$line" in
      "region:"*" | region=$id item="* | "item:"*" item=$id salience="*)
        line=${line#see: }
        printf '%s' "${line%% | region=*}"
        return ;;
    esac
  done < <(printf '%s\n' "$2" | grep '^see: ')
}
# Free roam: what the character sees of the plate that went live and thinks of it (`persona
# perceive`: the picture on screen). Prints the `[see NN]` and `[thought NN]` lines and sets MIND to
# what the look and paint lines carry in their situation: what is on their mind and what they have
# their eye on.
roam_perceive() { # move number, line said when moving, what the plate shows
  MIND=""
  [ "$ROAM_PERCEIVE" = on ] || return
  local out thought attention what sees header
  out=$(timeout $HUNG $D dev --port $P persona perceive "$ROAM" "recent=$ROAM: $2" "scene=$3" 2>&1)
  thought=$(printf '%s\n' "$out" | sed -n 's/^thought: //p' | head -1)
  if [ -z "$thought" ]; then
    printf '[see %02d] <no sight: %s>\n' "$1" "$(printf '%s\n' "$out" | head -1)"
    return
  fi
  header=$(printf '%s\n' "$out" | head -1)
  attention=$(printf '%s\n' "$out" | sed -n 's/^attention: //p' | head -1)
  what=$(roam_attended "$attention" "$out")
  sees=$(printf '%s\n' "$out" | sed -n 's/^see: \(.*\) | region=.*$/\1/p' | paste -sd ';' - |
    sed 's/;/; /g')
  printf '[see %02d] vision=%s %s | attending: %s\n' "$1" "$(field_of "$header" vision)" \
    "${sees:--}" "${what:-nothing in particular}"
  printf '[thought %02d] %s\n' "$1" "$thought"
  MIND=" What is on your mind: $thought${what:+ You have your eye on: $what.}"
}

# Free roam: the character declares the physical action they are doing right now (`persona
# declare`). The declaration is spoken (voice, bubble) and its `watch:` line is handed to the frame
# watch, which makes an emergency plate of it. Prints `[declare NN] <words>` and sets DECLARED_LINE to
# the words said (empty when none were); the plate is waited for by roam_declared_plate.
roam_declare() { # move number, the line before it, the place
  DECLARED_LINE=""
  local after out line relay
  after=$(latest_event)
  out=$(timeout $HUNG $D dev --port $P persona declare "$ROAM" speak=true "recent=$ROAM: $2" \
    "$ROAM_DECLARE You are at $3.$MIND" 2>&1)
  line=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  printf '[declare %02d] %s\n' "$1" "${line:-<no line: $(printf '%s\n' "$out" | head -1)>}"
  roam_inventory "$line" "$out"
  [ -z "$line" ] && return
  DECLARED_LINE=$line
  relay=$(printf '%s\n' "$out" | sed -n 's/^watch: //p' | head -1)
  if [ -z "$relay" ] || [ -z "$WATCH_PID" ]; then
    printf '[declare %02d] no emergency plate: the declaration reported no action to draw\n' "$1"
    return
  fi
  printf '%s\n' "$relay" >&4
  roam_declared_plate "$after" "$plate-look" "$1"
}
# Waits for the emergency plate of the declaration made after event $1 in the watch of frames $2:
# started, shown (with the `PERSONA declared` line: how many room plates it superseded, how long it
# waited for the screen, how long it took to make) and ended after its hold, from the host's
# `frame_timing` lines, and prints the second `[declare NN]` line.
roam_declared_plate() { # after-sequence, frames name, move number
  local after=$1 frames=$2 number=$3 frame="" shown="" made="-" waited="-" superseded="-"
  local deadline=$((SECONDS + HUNG)) sequence text
  while [ "$SECONDS" -lt "$deadline" ]; do
    sleep 5
    while read -r sequence _ text; do
      after=$sequence
      case "$text" in
        'PERSONA declared who='*"plate=$frames-"*)
          superseded=$(field_of "$text" superseded)
          waited=$(field_of "$text" queue_wait_ms)
          made=$(field_of "$text" made_ms) ;;
        'PERSONA frame_timing '*"frame=$frames-"*'reason=declared'*)
          frame=$(field_of "$text" frame)
          case "$(field_of "$text" moment)" in
            shown) shown=$(field_of "$text" at_ms) ;;
            ended)
              printf '[declare %02d] plate=%s shown_at_ms=%s made_ms=%s queue_wait_ms=%s superseded=%s ended_at_ms=%s up_ms=%s\n' \
                "$number" "$frame" "${shown:--}" "$made" "$waited" "$superseded" \
                "$(field_of "$text" at_ms)" "$(field_of "$text" took_ms)"
              return ;;
            failed)
              printf '[declare %02d] plate=%s failed after %s ms\n' "$number" "$frame" \
                "$(field_of "$text" took_ms)"
              return ;;
          esac ;;
      esac
    done < <($D dev --port $P events "$after" area=PERSONA 2>/dev/null | grep -E '^ *[0-9]+ ')
  done
  printf '[declare %02d] no emergency plate within %s s\n' "$number" "$HUNG"
}

# The items whose counts rose between two `held_counts` snapshots, as `<id> <before>-><after>`
# separated by commas, or nothing.
held_gains() { # before, after
  local item count was gains=""
  while IFS== read -r item count; do
    [ -z "$item" ] && continue
    was=$(printf '%s\n' "$1" | sed -n "s/^$item=//p" | head -1)
    [ "${count:-0}" -gt "${was:-0}" ] && gains="${gains:+$gains, }$item ${was:-0}->$count"
  done <<< "$2"
  printf '%s\n' "$gains"
}
# Free roam: the arrival step of a kind of move that has one (`@way_arrival`). The character says one
# line toward the situation (`persona say`), the per-line judge applies what it trades to what they
# really hold, and the log says what rose, or that nothing did.
roam_trade() { # move number, the line before it, the place, the situation
  local before after out line gained
  before=$(held_counts)
  out=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true "recent=$ROAM: $2" \
    "intent=$4" "You are at $3.$MIND" 2>&1)
  line=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  printf '[trade %02d] %s\n' "$1" "${line:-<no line: $(printf '%s\n' "$out" | head -1)>}"
  roam_inventory "$line" "$out"
  after=$(held_counts)
  gained=$(held_gains "$before" "$after")
  printf '[trade %02d] changes: %s; gained: %s\n' "$1" "$(held_diff "$before" "$after")" \
    "${gained:-no gain recorded}"
}

# Free roam: the `@move` lines in order. Each move is said by the roaming character toward the goal
# and the way's situation (where they are, where they have been and what they hold now), run as a
# move with a plate of its own, and the close look at the new place waited out before the next.
play_roam() {
  local stamp number=0 way out said reading place picture plate things trace look line seq
  local here="" visited="" holding situation interpreted designed briefed painted live
  local events_at held_before held_after paint paint_line insert_line inspected thing_name thing_look
  local board_at
  stamp=$(date +%m%d%H%M)
  if [ -n "$INVENTORY" ]; then
    $D dev --port $P world inventory set "$ROAM" character "$INVENTORY" | head -1 | sed 's/^/       ~ /'
  fi
  for way in "${MOVES[@]}"; do
    number=$((number + 1))
    plate=$(printf 'roam-%s-%02d' "$stamp" "$number")
    holding=$($D dev --port $P world inventory get "$ROAM" 2>/dev/null |
      sed -n 's/^  [^ ]* x\([0-9]*\): \([^(]*\) (.*$/\2 x\1/p' | paste -sd ',' - | sed 's/,/, /g')
    situation="$ROAM_GOAL ${WAY[$way]}${here:+ You are at $here now.}"
    situation="$situation${visited:+ Places you have been: $visited.}"
    situation="$situation${holding:+ What you hold now: $holding.}"
    stop_watch quiet
    events_at=$(latest_event)
    held_before=$(held_counts)
    out=$(timeout $HUNG $D dev --port $P move act "$ROAM" "plate=$plate" "$situation" 2>&1)
    said=$(printf '%s\n' "$out" | sed -n "s/^$ROAM says: //p" | head -1)
    reading=$(printf '%s\n' "$out" | sed -n 's/^reading: move //p; s/^\(not moved: .*\)/\1/p' |
      head -1)
    place=$(printf '%s\n' "$out" | sed -n 's/^design: place=\(.*\) mood=.*/\1/p' | head -1)
    picture=$(printf '%s\n' "$out" | sed -n 's/^picture: //p' | head -1)
    things=$(printf '%s\n' "$out" | grep '^thing ')
    trace=$(printf '%s\n' "$out" | grep '^TRANSITION ')
    interpreted=$(stage_at "$trace" designing)
    designed=$(stage_at "$trace" briefing)
    briefed=$(stage_at "$trace" generating)
    painted=$(stage_at "$trace" locating)
    live=$(stage_at "$trace" live)
    printf '[move %02d] %s | said: %s | read: %s | place: %s | plate: %s | jev_ms=%s design_ms=%s plate_ms=%s live_at_ms=%s\n' \
      "$number" "$way" "${said:-<none>}" "${reading:-<none>}" "${place:-<none>}" "$plate" \
      "${interpreted:--}" "$(span "$interpreted" "$designed")" "$(span "$briefed" "$painted")" \
      "${live:--}"
    roam_inventory "$said" "$out"
    roam_refs "${events_at:-0}" "$plate" "$number" "$out"
    printf '%s\n' "$out" | grep -E '^(sheet |place )' | sed 's/^/       /'
    if [ -z "$live" ]; then
      printf '       <move %02d did not go live: %s>\n' "$number" "$(printf '%s\n' "$out" | tail -1)"
      continue
    fi
    here=$place
    visited="${visited:+$visited; }$place"
    roam_remember_scene "$number" "$plate" "$place" "$picture"
    if [ "$way" = _show_ ]; then
      roam_show "$number" "$plate" "$place" "$said"
      continue
    fi
    start_watch "${FRAMES_LOG%.log}-$plate.log" "frames=$plate-look" "scene=$picture" \
      ${ROAM_DECLARE_HOLD:+"declare_hold=$ROAM_DECLARE_HOLD"} "${FRAMES[@]}"
    roam_perceive "$number" "$said" "$picture"
    seq=$(latest_event)
    look=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true "recent=$ROAM: $said" \
      "intent=$ROAM_LOOK" "You are at $place.$MIND" 2>&1)
    line=$(printf '%s\n' "$look" | sed -n '1s/.*says: //p')
    printf '[look %02d] %s\n' "$number" "${line:-<no line: $look>}"
    roam_inventory "$line" "$look"
    watch_line "$ROAM" "$line"
    inspected=""
    if [ -n "$line" ]; then
      insert_line=$(roam_insert "${seq:-0}" "$plate-look" "$number" "$things")
      printf '%s\n' "$insert_line"
      inspected=$(field_of "$insert_line" item)
    fi
    # What the study is of: the thing looked at closely, else the place as the design describes it.
    thing_name=$(printf '%s\n' "$things" | sed -n "s/^thing $inspected (\([^)]*\)).*/\1/p" | head -1)
    thing_look=$(printf '%s\n' "$things" | sed -n "s/^thing $inspected ([^)]*): //p" | head -1)
    if [ -z "$thing_name" ]; then
      thing_name=$place
      thing_look=$(printf '%s\n' "$out" | sed -n 's/^elements: //p' | head -1)
    fi
    board_at=$(item_count "$ART_BOARD")
    if [ -n "${ARRIVAL[$way]:-}" ]; then
      # This kind of move has its own arrival step (a trade at a market), not the declaration.
      roam_trade "$number" "${line:-$said}" "$place" "${ARRIVAL[$way]}"
    elif [ -n "$ROAM_DECLARE" ]; then
      # The declaration is the painting line: spoken, judged for the supplies it uses and drawn.
      roam_declare "$number" "${line:-$said}" "$place"
      paint_line=$DECLARED_LINE
      [ -n "$paint_line" ] && roam_paint_study "$number" "$plate" "$plate" "$thing_name" \
        "$thing_look" "$board_at" "$place" "$paint_line"
    elif [ -n "$ROAM_PAINT" ]; then
      paint=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true \
        "recent=$ROAM: ${line:-$said}" "intent=$ROAM_PAINT" "You are at $place.$MIND" 2>&1)
      paint_line=$(printf '%s\n' "$paint" | sed -n '1s/.*says: //p')
      printf '[paint %02d] %s\n' "$number" "${paint_line:-<no line: $paint>}"
      roam_inventory "$paint_line" "$paint"
      [ -n "$paint_line" ] && roam_paint_study "$number" "$plate" "$plate" "$thing_name" \
        "$thing_look" "$board_at" "$place" "$paint_line"
    fi
    roam_refused "${events_at:-0}" "$number"
    held_after=$(held_counts)
    printf '[inventory %02d] %s\n' "$number" "$(held_diff "$held_before" "$held_after")"
  done
  stop_watch quiet
  if [ -n "$SCENE_KIND$ART_KIND" ]; then
    $D dev --port $P world memory 2>&1 | sed 's/^/[memory] /'
  fi
}
[ -n "$ROAM" ] && play_roam

stop_watch
