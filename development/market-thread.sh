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
# Time in free roam: the study painting is made in the background from the moment the finishing line
# has been said while the declared plate is on screen (`roam_study_start`), its lines and result wait
# in files and are printed, in their usual order, when the declared plate has ended
# (`roam_study_finish`); `[time NN] move took <s> s (plate .., look .., declare .., study .., reveal ..)`
# says where the seconds went (`study` is what was still to wait for after the declaration).
#
# Walking about town (`@walk <way> <sights> [town=<seeds|within|leaves>]`, `@walk_hold <seconds>`):
# a move of a kind with a `@walk` line is asked for up to that many sights on the way to its place
# (`move act ... walk=N town=ROLE hold=S`); each is made together with the place, shown in order
# for at least the hold with a fade, and the place follows the last of them. `[walk NN] stage k/N
# <sight> shown_at_ms=.. up_ms=..` says when each went up and for how long, `[walk NN] stage k/N ..
# skipped reason=late|failed` a sight passed over, `[walk NN] arrived stages=S/N walk_ms=..` the
# end, `[walk NN] memory ..` each sight kept as a memory of kind `walk` linked to the place, and
# `[town NN]` the look of the town that every sight and place of it is painted after (the first
# place of a `town=seeds` move gives it, `town=leaves` ends it). `@tempo walk <kind> <strength>`
# makes each sight post that impulse as it goes up. `[time NN]` says how much of the plate's time
# was the walk.
#
# Pacing in free roam (data in the beats): a plate that goes live is left on screen for `@roam_dwell
# <seconds>` while the perception of it runs in parallel (`[dwell NN]`); each painting made is shown
# full frame with its title and then the view it was painted from (`@roam_reveal <painting seconds>
# <view seconds>`, `@roam_view_caption`; `@show <picture> | <caption> | <seconds>` to the frame watch,
# `[reveal NN]`, and `[pace NN] painting <picture> up <ms> of <held ms>, view <picture> up <ms> of <held ms>`
# from the host's `PERSONA revealed` and `frame_timing moment=ended .. reason=reveal` lines, by order); the show waits `@roam_show_dwell` seconds on the town square, then shows the
# last `@roam_show_each <N> <seconds>` paintings with their titles after the show plate and ends on
# it (`[show] piece ..`, `[pace show] ..`). `@refs max=<n>` is the soft cap on the pictures a plate
# follows: the runner asks for no more (a study follows its plate alone; the show plate its base, the
# gallery sheet and the traveller's sheet) and passes `refs_max=<n>` to the frame watch when the host
# takes it.
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
# Free roam score (`@tempo_table <music table>`, `@tempo <event> <kind> <strength>`): the host's tempo
# engine and music director play the table's tracks as the journey's tempo moves. The runner loads
# the table, runs `tempo prepare` (`[music] prepared: ..`) and starts the base track, then posts the
# events it already has (`arrival`, `look`, `declare`, `painted`, `trade`, `show`; an arrival is tried
# as `arrival_<way>` first) as the kinds the `@tempo` lines name, and prints `[tempo NN]` and `[music NN]`.
#
# The dice gauntlet (`@gauntlet <character>`): one character meets the stations of `@gauntlet_stations`
# in the order of the `@station <id> | player|narrator | <the dungeon master's scene line> | <what he
# sets out to do>` lines (a narrator's station has no fourth field: the dungeon master calls the
# check). At each station the plate replaces the backdrop with a fade (the page's fade, set once from
# `@gauntlet_fade <ms>`), the dungeon master sets the scene (`persona narrate`), the character says
# what he does (`persona say`) and `persona check run` judges the line, calls the check aloud, rolls
# real dice, shows the roll card and reads the result ("The check judge and the DM's checks" in the
# runbook). `@station_sight <id> <what is in sight>` joins the judge's facts (the ground of an
# advantage or a disadvantage), `@station_inspire <id> <die place>` spends Heroic Inspiration on that
# die, `@station_plate <id> <plate>` and `@station_place <id> <words>` replace the stations file's
# plate and setting for every run of that id (the judge's `place=` and the character's surroundings),
# and a station id may be listed again (a repeat). `@gauntlet_persona`, `@gauntlet_sheet` and `@gauntlet_stations` name the files,
# `@gauntlet_breath <seconds>` is the pause between stations and `@gauntlet_limit <seconds>` the time
# a station may take before it is logged `[station NN] stuck` and left. The log: `[station NN]`,
# `[dm NN]`, `[said NN]`, `[check NN]`, `[roll NN]`, `[hold NN]`, `[inspire NN]`, `[host NN]`,
# `[time NN]` and a closing `[summary]`, from the command's answer and the host's own `CHECK`,
# `TIMING roll`, `RULES rolled` and `PERSONA check_*` audit lines. `--plan` after the beats file (or
# `DF_RUN_PLAN=1`) prints and checks every station's steps and calls no host.
# Before the first station the prep gives every plate (once) a layout with the character named
# (`media locate show`, `media locate`, `media locate identify` by `@gauntlet_look` and
# `@gauntlet_portrait`, `[prep] ..`): a plate where he is not found makes its stations narrator-only.
# A station that runs out of time is cleared (`scene roll-clear`) and the host waited for before the
# next. Once this host's `persona check` usage shows `intent=`, a narrator station passes its own
# test as `intent=<kind[:ability[:skill]]>`.
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
BEATS=${1:?usage: bash development/market-thread.sh <beats-file> [--plan]}
PLAN=${DF_RUN_PLAN:-}
[ "${2:-}" = --plan ] && PLAN=1
# A run never turns on a switch that spends money (owner, 2026-11-10): the toolbar keeps TTS and
# Music from one run to the next, and the host calls its voice and composer only while they are on.
# `DF_RUN_TTS` and `DF_RUN_MUSIC` (`on` or `off`) set one for this run alone; unset, what the host
# remembers stands. The line printed says which it was: `[settings] tts=<on|off> (from <stored|flag>)`.
run_setting() {
  local key=$1 flag=$2 source=stored state
  case "$flag" in
    on) $D dev --port $P playground set "$key=true" >/dev/null 2>&1; source=flag ;;
    off) $D dev --port $P playground set "$key=false" >/dev/null 2>&1; source=flag ;;
  esac
  state=$($D dev --port $P playground 2>/dev/null | sed -n "s/^$key: //p")
  case "$state" in true) state=on ;; false) state=off ;; *) state=unknown ;; esac
  printf '[settings] %s=%s (from %s)\n' "$key" "$state" "$source"
}
if [ -z "$PLAN" ]; then
  run_setting tts "${DF_RUN_TTS:-}"
  run_setting music "${DF_RUN_MUSIC:-}"
fi
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
ROAM_DWELL=6
ROAM_SHOW_DWELL=12
SHOW_EACH_N=6
SHOW_EACH_SECONDS=5
REVEAL_PAINTING_SECONDS=10
REVEAL_VIEW_SECONDS=6
VIEW_CAPTION="The view"
REFS_MAX=""
WATCH_REFS=""
MIND=""
PERCEIVED_THINGS=""
MOVES=()
TEMPO_TABLE=""
declare -A PLACE_PLATE PLACE_KEEPER PLACE_KEEPER_SITUATION PLACE_SITUATION PLACE_SCENE WAY ARRIVAL
declare -A TEMPO_EVENT WALK_STAGES WALK_TOWN STATION_SIGHT STATION_INSPIRE STATION_PLATE STATION_PLACE
GAUNTLET=""
GAUNTLET_PERSONA=""
GAUNTLET_SHEET=""
GAUNTLET_STATIONS=""
GAUNTLET_FADE=800
GAUNTLET_BREATH=1.5
GAUNTLET_LIMIT=180
GAUNTLET_LOOK=""
GAUNTLET_PORTRAIT=""
STATIONS=()
WALK_HOLD=
WALK_SECONDS=0
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
    '@roam_dwell '*) ROAM_DWELL=$value ;;
    '@roam_show_dwell '*) ROAM_SHOW_DWELL=$value ;;
    '@roam_show_each '*) SHOW_EACH_N=${value%% *}; SHOW_EACH_SECONDS=${value#* } ;;
    '@roam_reveal '*) REVEAL_PAINTING_SECONDS=${value%% *}; REVEAL_VIEW_SECONDS=${value#* } ;;
    '@roam_view_caption '*) VIEW_CAPTION=$value ;;
    '@refs '*) REFS_MAX=${value#max=} ;;
    '@roam_scene_kind '*) SCENE_KIND=$value ;;
    '@roam_art_kind '*) ART_KIND=$value ;;
    '@roam_art_item '*) ART_ITEM=$value ;;
    '@roam_art_board '*) ART_BOARD=$value ;;
    '@roam_art_title '*) ART_TITLE=$value ;;
    '@roam_art_look '*) ART_LOOK=$value ;;
    '@roam_show '*) ROAM_SHOW=$value ;;
    '@roam_show_look '*) ROAM_SHOW_LOOK=$value ;;
    '@roam_show_say '*) ROAM_SHOW_SAY=$value ;;
    '@tempo_table '*) TEMPO_TABLE=$value ;;
    '@tempo '*) TEMPO_EVENT[${value%% *}]=${value#* } ;;
    '@way '*) WAY[${value%% *}]=${value#* } ;;
    '@way_arrival '*) ARRIVAL[${value%% *}]=${value#* } ;;
    '@walk_hold '*) WALK_HOLD=$value ;;
    '@walk '*) read -r walk_way walk_n walk_town <<< "$value"
      WALK_STAGES[$walk_way]=$walk_n; WALK_TOWN[$walk_way]=$walk_town ;;
    '@move '*) MOVES+=("$value") ;;
    '@gauntlet '*) GAUNTLET=$value ;;
    '@gauntlet_persona '*) GAUNTLET_PERSONA=$value ;;
    '@gauntlet_sheet '*) GAUNTLET_SHEET=$value ;;
    '@gauntlet_stations '*) GAUNTLET_STATIONS=$value ;;
    '@gauntlet_fade '*) GAUNTLET_FADE=$value ;;
    '@gauntlet_breath '*) GAUNTLET_BREATH=$value ;;
    '@gauntlet_limit '*) GAUNTLET_LIMIT=$value ;;
    '@gauntlet_look '*) GAUNTLET_LOOK=$value ;;
    '@gauntlet_portrait '*) GAUNTLET_PORTRAIT=$value ;;
    '@station '*) STATIONS+=("$value") ;;
    '@station_sight '*) STATION_SIGHT[${value%% *}]+="${value#* }"$'\n' ;;
    '@station_inspire '*) STATION_INSPIRE[${value%% *}]=${value#* } ;;
    '@station_plate '*) STATION_PLATE[${value%% *}]=${value#* } ;;
    '@station_place '*) STATION_PLACE[${value%% *}]=${value#* } ;;
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
if [ -z "$FLOW" ] && [ -z "$ROAM" ] && [ -z "$GAUNTLET" ] && { [ -z "$KEEPER" ] || [ -z "$KEEPER_SITUATION" ] ||
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
    sleep 1
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
# Free roam: the painting made of what they studied, in two parts so that it can be made while the
# declared plate is on screen. `roam_study_start` runs `roam_study_job` in the background; its output
# and result wait in files until `roam_study_finish` prints them, so every line comes in the order it
# always did. The job: the line toward `@roam_painted` is said first and judged like any line (the board
# painted on, the pigments, the oil and the rags are used up, as the per-line judge reads it against
# what they really hold). The study is made only when that used up the item of `@roam_art_board` (its
# count is below the count before the paint lines), so the board becomes the painting; otherwise the
# log says why. The move's plate is then edited into the study (that plate is its only reference),
# stored as `item-<owner>-<item>.png`; a study the host finds near blank is made once more with
# `@roam_art_retry` added, and removed if still blank. The finish, once the job is done, keeps the
# study as a memory of kind `@roam_art_kind` linked to its item and place, adds it to what they hold
# as a kept item and shows it (`roam_reveal`). A job that does not end within three hung-bounds is
# stopped, its file removed and the log says so.
# Arguments: move number, plate, thing name, thing look, the board count before, place name, the line
# said before, the files' common name.
roam_study_job() {
  local number=$1 id file prompt title request try seen made stored done_line done_out now
  if [ -n "$ROAM_PAINTED" ]; then
    done_out=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true \
      "recent=$ROAM: $7" "intent=$ROAM_PAINTED" "You are at $6.$MIND" 2>&1)
    done_line=$(printf '%s\n' "$done_out" | sed -n '1s/.*says: //p')
    printf '[painted %02d] %s\n' "$number" "${done_line:-<no line: $done_out>}"
    roam_inventory "$done_line" "$done_out"
  fi
  now=$(item_count "$ART_BOARD")
  if [ -n "$ART_BOARD" ] && [ "${now:-0}" -ge "${5:-0}" ]; then
    printf '[memory %02d] not painted: no %s was used up (had %s, have %s)\n' "$number" \
      "$ART_BOARD" "${5:-0}" "$now"
    return
  fi
  id=$(printf '%s_%02d' "${ART_ITEM:-artwork}" "$number")
  file="item-$ROAM-$id"
  title=${ART_TITLE//\{thing\}/$3}
  prompt="$ART_LOOK The subject of the study: $4"
  # The plate is the reference for the place; the board is only described in words (a board's sheet
  # as a reference made the model paint just the board).
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
  printf 'stored=%q id=%q title=%q prompt=%q\n' "$stored" "$id" "$title" "$prompt" > "$8.state"
}
# Starts the study of move $1 in the background (see roam_study_job).
roam_study_start() { # move number, plate, thing name, thing look, board count before, place name, line
  [ -z "$ART_KIND" ] && return
  local folder
  folder=$(dirname "$FRAMES_LOG")
  mkdir -p "$folder"
  STUDY_BASE="$folder/study-$2"
  rm -f "$STUDY_BASE.out" "$STUDY_BASE.state" "$STUDY_BASE.done"
  STUDY_STARTED=$SECONDS
  # The job does not hold the frame watch's input open.
  ( exec 4>&-
    roam_study_job "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$STUDY_BASE"
    : > "$STUDY_BASE.done" ) > "$STUDY_BASE.out" 2>&1 &
  STUDY_PID=$!
}
# Waits for the study started for move $1 (for three hung-bounds at most), prints what the job
# printed, keeps the study as a memory and an item and shows it. Sets TIME_STUDY (seconds from here
# to the study kept, so what the overlap saved shows) and TIME_REVEAL (the reveal).
roam_study_finish() { # move number, plate, place id
  TIME_STUDY=0
  TIME_REVEAL=0
  [ -n "${STUDY_PID:-}" ] || return
  local number=$1 began=$SECONDS deadline=$((SECONDS + 3 * HUNG)) stored="" id="" title="" prompt=""
  local kept
  while [ ! -e "$STUDY_BASE.done" ] && [ "$SECONDS" -lt "$deadline" ]; do
    sleep 1
  done
  if [ ! -e "$STUDY_BASE.done" ]; then
    kill "$STUDY_PID" 2>/dev/null
    cat "$STUDY_BASE.out" 2>/dev/null
    printf '[memory %02d] study abandoned after %s s\n' "$number" "$((SECONDS - STUDY_STARTED))"
    $D dev --port $P media remove "item-$ROAM-${ART_ITEM:-artwork}_$(printf '%02d' "$number").png" \
      >/dev/null 2>&1
    rm -f "$STUDY_BASE.out" "$STUDY_BASE.state" "$STUDY_BASE.done"
    STUDY_PID=""
    return
  fi
  cat "$STUDY_BASE.out"
  # shellcheck disable=SC1090
  [ -e "$STUDY_BASE.state" ] && . "$STUDY_BASE.state"
  rm -f "$STUDY_BASE.out" "$STUDY_BASE.state" "$STUDY_BASE.done"
  STUDY_PID=""
  if [ -z "$stored" ]; then
    TIME_STUDY=$((SECONDS - began))
    return
  fi
  kept=$($D dev --port $P world memory capture "$ROAM" "$ART_KIND" "$stored" "$title" "id=$id" \
    "place=$3" "description=$prompt" "link=item:$id" "link=place:$3" by=runner 2>&1 | head -1)
  case "$kept" in
    'ok memory='*)
      printf '[memory %02d] painted %s seq=%s picture=%s after=%s plate=%s (%s)\n' "$number" \
        "$(field_of "$kept" memory)" "$(field_of "$kept" seq)" "$stored" "-" "$2.png" "$title"
      roam_tempo "$number" painted ;;
    *) printf '[memory %02d] painted %s but not kept as a memory: %s\n' "$number" "$stored" "$kept"
      kept="" ;;
  esac
  $D dev --port $P world inventory add "$ROAM" "$id" 1 "name=$title" "look=$prompt" use=kept \
    why=made 2>&1 | head -1 | sed "s/^/       ~ item $id: /"
  TIME_STUDY=$((SECONDS - began))
  if [ -n "$kept" ]; then
    began=$SECONDS
    roam_reveal "$number" "$stored" "$title" "$2"
    TIME_REVEAL=$((SECONDS - began))
  fi
}
# Free roam: the day's work shown. Every memory of kind `@roam_art_kind` is composed into one gallery
# sheet; the show plate is the move's plate edited after that sheet and the traveller's own sheet,
# put behind the scene with the traveller located in it, and the traveller says a line.
roam_show() { # move number, plate, place name, line said when moving
  [ -z "$ART_KIND" ] && return
  local composed gallery count refs shown line made scan region began=$SECONDS held
  composed=$($D dev --port $P world memory compose "$ROAM" "kind=$ART_KIND" 2>&1 | head -1)
  gallery=$(field_of "$composed" picture)
  if [ -z "$gallery" ]; then
    printf '[show] no gallery: %s\n' "$composed"
    return
  fi
  count=$(field_of "$composed" members)
  printf '[show] composed gallery of %s pieces -> %s\n' "$count" "$gallery"
  # The gallery sheet and the traveller's single sheet, and the base: no more than `@refs max=`.
  refs=$gallery
  if library_holds "ref-$ROAM.png" && [ "${REFS_MAX:-8}" -ge 3 ]; then
    refs="$refs,ref-$ROAM.png"
  fi
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
  # The square has been on screen all this while; it stays up at least for the show's dwell.
  held=$((SECONDS - began))
  [ "$held" -lt "$ROAM_SHOW_DWELL" ] && sleep $((ROAM_SHOW_DWELL - held))
  printf '[dwell %02d] the town square was up %s s before the show plate (dwell %s s)\n' "$1" \
    "$((SECONDS - began))" "$ROAM_SHOW_DWELL"
  $D dev --port $P scene image tv-backdrop "$shown" plate=show 2>&1 | head -1 | sed 's/^/       ~ /'
  start_watch "${FRAMES_LOG%.log}-$2-show.log" "frames=$2-show" "scene=$ROAM_SHOW_LOOK" \
    ${WATCH_REFS:+"$WATCH_REFS"} "${FRAMES[@]}"
  sleep "$ROAM_DWELL"
  roam_show_pieces
  stop_watch quiet
  line=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true "recent=$ROAM: $4" \
    "intent=$ROAM_SHOW_SAY" "You are at $3." 2>&1)
  printf '[show] %s\n' "$(printf '%s\n' "$line" | sed -n '1s/.*says: //p')"
  roam_inventory "$(printf '%s\n' "$line" | sed -n '1s/.*says: //p')" "$line"
}

# Free roam: the new plate is left on screen for `@roam_dwell` seconds so it can be taken in, and the
# perception of it runs in parallel, so the dwell costs no time when the perception takes longer.
# `roam_view_start` is called the moment the plate is live (the perception goes on in the background
# while the move's bookkeeping runs); `roam_view_finish` waits out the dwell, counted from then, and
# the perception, prints what `roam_perceive` printed and sets MIND and PERCEIVED_THINGS as it did.
roam_view_start() { # move number, line said when moving, what the plate shows
  local folder
  folder=$(dirname "$FRAMES_LOG")
  mkdir -p "$folder"
  VIEW_BEGAN=$SECONDS
  VIEW_STEM="$folder/view-$plate"
  ( roam_perceive "$1" "$2" "$3"
    printf '%s' "$MIND" > "$VIEW_STEM.mind"
    printf '%s' "$PERCEIVED_THINGS" > "$VIEW_STEM.things"
  ) > "$VIEW_STEM.see" 2>&1 &
  VIEW_PID=$!
}
roam_view_finish() { # move number
  local held=$((SECONDS - VIEW_BEGAN))
  [ "$held" -lt "$ROAM_DWELL" ] && sleep $((ROAM_DWELL - held))
  wait "$VIEW_PID"
  cat "$VIEW_STEM.see"
  MIND=$(cat "$VIEW_STEM.mind" 2>/dev/null)
  PERCEIVED_THINGS=$(cat "$VIEW_STEM.things" 2>/dev/null)
  rm -f "$VIEW_STEM.mind" "$VIEW_STEM.things" "$VIEW_STEM.see"
  printf '[dwell %02d] the view was up %s s before he spoke of it (dwell %s s)\n' "$1" \
    "$((SECONDS - VIEW_BEGAN))" "$ROAM_DWELL"
}
# What the host says of the `@show` pictures shown after event $1, in order: REVEAL_PICTURES (the
# picture of each `PERSONA revealed picture=.. hold_ms=..` line) and REVEAL_UPS (the `took_ms` of each
# `PERSONA frame_timing moment=ended .. reason=reveal` line, the time that picture was up). Waits for
# $2 pictures to have ended, at most $3 seconds of showing and 30 more, or until the watch has ended
# (the events it left are read first); they are matched by order.
roam_reveal_wait() { # after-sequence, how many, seconds of showing
  local after=$1 want=$2 deadline=$((SECONDS + $3 + 30)) sequence text
  REVEAL_PICTURES=()
  REVEAL_UPS=()
  while [ "$SECONDS" -lt "$deadline" ] && [ "${#REVEAL_UPS[@]}" -lt "$want" ]; do
    sleep 1
    while read -r sequence _ text; do
      after=$sequence
      case "$text" in
        'PERSONA revealed '*) REVEAL_PICTURES+=("$(field_of "$text" picture)") ;;
        'PERSONA frame_timing '*)
          if [ "$(field_of "$text" reason)" = reveal ] && [ "$(field_of "$text" moment)" = ended ]; then
            REVEAL_UPS+=("$(field_of "$text" took_ms)")
          fi ;;
      esac
    done < <($D dev --port $P events "$after" area=PERSONA 2>/dev/null | grep -E '^ *[0-9]+ ')
    kill -0 "$WATCH_PID" 2>/dev/null || break
  done
}
# Free roam: the finished painting is shown full frame with its title, then the view it was painted
# from (`@show <picture> | <caption> | <seconds>` to the frame watch), and the time each was up is
# printed as `[pace NN]`.
roam_reveal() { # move number, the painting's picture, its title, the move's plate
  [ -n "$WATCH_PID" ] || return
  local after caption=${3//|/-}
  after=$(latest_event)
  printf '@show %s | %s | %s\n' "$2" "$caption" "$REVEAL_PAINTING_SECONDS" >&4
  printf '[reveal %02d] painting %s | %s | %s s\n' "$1" "$2" "$caption" "$REVEAL_PAINTING_SECONDS"
  printf '@show %s.png | %s | %s\n' "$4" "$VIEW_CAPTION" "$REVEAL_VIEW_SECONDS" >&4
  printf '[reveal %02d] view %s.png | %s | %s s\n' "$1" "$4" "$VIEW_CAPTION" "$REVEAL_VIEW_SECONDS"
  roam_reveal_wait "$after" 2 $((REVEAL_PAINTING_SECONDS + REVEAL_VIEW_SECONDS))
  printf '[pace %02d] painting %s up %s of %s ms, view %s up %s of %s ms\n' "$1" \
    "${REVEAL_PICTURES[0]:-$2}" "${REVEAL_UPS[0]:--}" "$((REVEAL_PAINTING_SECONDS * 1000))" \
    "${REVEAL_PICTURES[1]:-$4.png}" "${REVEAL_UPS[1]:--}" "$((REVEAL_VIEW_SECONDS * 1000))"
}
# Free roam: the last `@roam_show_each` paintings, one after the other with their titles, for
# `@roam_show_each` seconds each; the watch goes back to the show plate after the last.
roam_show_pieces() {
  [ "${SHOW_EACH_N:-0}" -gt 0 ] && [ -n "$WATCH_PID" ] || return
  local rows id title picture after number count=0
  rows=$($D dev --port $P world memory list "$ROAM" "kind=$ART_KIND" limit=1000 2>/dev/null |
    sed -n 's/^ *[0-9][0-9]*  \([^ ]*\)  ([^)]*)  \(.*\)  place=[^ ]*  pictures=\([^ ,]*\).*$/\1|\2|\3/p' |
    tail -n "$SHOW_EACH_N")
  after=$(latest_event)
  while IFS='|' read -r id title picture; do
    [ -z "$id" ] && continue
    printf '@show %s | %s | %s\n' "$picture" "${title//|/-}" "$SHOW_EACH_SECONDS" >&4
    printf '[show] piece %s %s | %s | %s s\n' "$id" "$picture" "$title" "$SHOW_EACH_SECONDS"
    count=$((count + 1))
  done <<< "$rows"
  [ "$count" -gt 0 ] || return
  roam_reveal_wait "$after" "$count" $((count * SHOW_EACH_SECONDS))
  for number in $(seq 0 $((count - 1))); do
    printf '[pace show] piece %s up %s of %s ms\n' "${REVEAL_PICTURES[$number]:--}" \
      "${REVEAL_UPS[$number]:--}" "$((SHOW_EACH_SECONDS * 1000))"
  done
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
# perceive`: the picture on screen). Prints the `[see NN]` and `[thought NN]` lines and one `[thing
# NN] <id> (<name>): <look>` for each thing they noticed and added to the place, sets PERCEIVED_THINGS
# to those as `thing <id> (<name>): <look>` lines (the shape of the design's, so a close look at one
# has its name), and sets MIND to what the look and paint lines carry in their situation: what is on
# their mind and what they have their eye on.
roam_perceive() { # move number, line said when moving, what the plate shows
  MIND=""
  PERCEIVED_THINGS=""
  [ "$ROAM_PERCEIVE" = on ] || return
  local out thought attention what sees header
  out=$(timeout $HUNG $D dev --port $P persona perceive "$ROAM" "recent=$ROAM: $2" "scene=$3" 2>&1)
  thought=$(printf '%s\n' "$out" | sed -n 's/^thought: //p' | head -1)
  if [ -z "$thought" ]; then
    printf '[see %02d] <no sight: %s>\n' "$1" "$(printf '%s\n' "$out" | head -1)"
    return
  fi
  PERCEIVED_THINGS=$(printf '%s\n' "$out" | grep '^thing ')
  header=$(printf '%s\n' "$out" | head -1)
  attention=$(printf '%s\n' "$out" | sed -n 's/^attention: //p' | head -1)
  what=$(roam_attended "$attention" "$out")
  sees=$(printf '%s\n' "$out" | grep -v ' subject=\(self\|carried\)$' |
    sed -n 's/^see: \(.*\) | region=.*$/\1/p' | paste -sd ';' - | sed 's/;/; /g')
  printf '[see %02d] vision=%s %s | attending: %s\n' "$1" "$(field_of "$header" vision)" \
    "${sees:--}" "${what:-nothing in particular}"
  printf '[thought %02d] %s\n' "$1" "$thought"
  printf '%s\n' "$PERCEIVED_THINGS" | sed -n "s/^thing /[thing $(printf '%02d' "$1")] /p"
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
  # The study is made while the declared plate is on screen.
  roam_study_start "$1" "$plate" "$STUDY_THING_NAME" "$STUDY_THING_LOOK" "$STUDY_BOARD_AT" "$3" "$line"
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
    sleep 1
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
  roam_tempo "$1" trade
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

# Free roam tempo and music (`@tempo_table <music table>`, `@tempo <event> <kind> <strength>`): the
# host's tempo engine and music director do the work; the runner only posts events. The event is
# posted as the kind and strength its `@tempo` line names (`<event>_<way>` is tried before
# `<event>`, so an arrival can differ by the kind of move) and the answer is printed as
# `[tempo NN] phase=.. intensity=..` and `[music NN] cue=<profile> track=<name> reused|composed |
# decision=..`. Nothing waits on it and a failure is only printed.
roam_tempo() { # move number, event, [way]
  [ -n "$TEMPO_TABLE" ] || return 0
  local mapped=${TEMPO_EVENT[${2}_${3:-}]:-${TEMPO_EVENT[$2]:-}} out
  [ -n "$mapped" ] || return 0
  # shellcheck disable=SC2086
  out=$(timeout 60 $D dev --port $P tempo impulse $mapped 2>&1)
  case "$out" in
    'tempo phase='*) printf '%s\n' "$out" |
      sed -n "s/^tempo /[tempo $1] /p; s/^music /[music $1] /p" ;;
    *) printf '[tempo %s] not posted (%s): %s\n' "$1" "$2" "$(printf '%s\n' "$out" | head -1)" ;;
  esac
}
# Free roam, before the first move: load the music table, find or compose every profile's track
# (`tempo prepare` is the only place a track is made, the run plays only those it finds) and start
# the journey on the base track. Without a table, or if it does not load, the roam is silent.
roam_tempo_start() {
  [ -n "$TEMPO_TABLE" ] || return 0
  local out
  if ! out=$($D dev --port $P tempo load "$TEMPO_TABLE" 2>&1); then
    printf '[music] table not loaded: %s\n' "$(printf '%s\n' "$out" | head -1)"
    TEMPO_TABLE=""
    return 0
  fi
  out=$(timeout $HUNG $D dev --port $P tempo prepare 2>&1)
  printf '[music] prepared: %s\n' "$(printf '%s\n' "$out" | head -1 | sed 's/^prepared //')"
  printf '%s\n' "$out" | sed -n '2,$p' | sed 's/^/       ~ /'
  out=$($D dev --port $P tempo reset 2>&1)
  printf '%s\n' "$out" | sed -n 's/^tempo /[tempo 00] /p; s/^music /[music 00] /p'
}

# Free roam walk (`@walk <way> <sights> [town=<seeds|within|leaves>]`, `@walk_hold <seconds>`): the
# options `move act` takes for a kind of move that walks: `walk=<sights>`, its `town=` role,
# `hold=` and, when the tempo table is loaded and has an `@tempo walk <kind> <strength>` line, the
# `impulse=<kind>:<strength>` each sight posts as it goes up. Prints nothing for a kind of move
# with no `@walk` line.
roam_walk_options() { # way
  [ -n "${WALK_STAGES[$1]:-}" ] || return 0
  local options="walk=${WALK_STAGES[$1]}" mapped=${TEMPO_EVENT[walk]:-}
  [ -n "${WALK_TOWN[$1]:-}" ] && options="$options ${WALK_TOWN[$1]}"
  [ -n "$WALK_HOLD" ] && options="$options hold=$WALK_HOLD"
  [ -n "$TEMPO_TABLE" ] && [ -n "$mapped" ] && options="$options impulse=${mapped% *}:${mapped#* }"
  printf '%s' "$options"
}
# Prints the `walk ..` lines of a move's output as `[walk NN] ..` and the look of its town as
# `[town NN] ..`, and sets WALK_SECONDS to the seconds from the first sight going up to the place
# going up (0 without a walk).
roam_walk_lines() { # move number, output of move act
  local ms number
  number=$(printf '%02d' "$1")
  printf '%s\n' "$2" | sed -n "s/^walk /[walk $number] /p; s/^town: /[town $number] /p"
  ms=$(printf '%s\n' "$2" | sed -n 's/^walk arrived .*walk_ms=\([0-9]*\).*/\1/p' | head -1)
  WALK_SECONDS=$(( ${ms:-0} / 1000 ))
}

# Free roam: the `@move` lines in order. Each move is said by the roaming character toward the goal
# and the way's situation (where they are, where they have been and what they hold now), run as a
# move with a plate of its own, and the close look at the new place waited out before the next.
play_roam() {
  local stamp number=0 way out said reading place picture plate things trace look line seq
  local here="" visited="" holding situation interpreted designed briefed painted live
  local events_at held_before held_after paint paint_line insert_line inspected thing_name thing_look
  local board_at t_move t_plate t_look_from t_declare_from t_look t_declare
  stamp=$(date +%m%d%H%M)
  # The soft cap on the pictures a frame follows goes to the watch only when this host takes it.
  if [ -n "$REFS_MAX" ] &&
    timeout 30 $D dev --port $P persona watch </dev/null 2>&1 | grep -q 'refs_max'; then
    WATCH_REFS="refs_max=$REFS_MAX"
  fi
  if [ -n "$INVENTORY" ]; then
    $D dev --port $P world inventory set "$ROAM" character "$INVENTORY" | head -1 | sed 's/^/       ~ /'
  fi
  # The TV shows what the traveller holds, supplies going down as they are used up.
  $D dev --port $P world inventory show "$ROAM" | head -1 | sed 's/^/       ~ /'
  # The soft cap holds for every plate the host builds, the move plates before the first watch too.
  if [ -n "$REFS_MAX" ] && [ -n "$WATCH_REFS" ]; then
    $D dev --port $P media backend "refs=$REFS_MAX" | head -1 | sed 's/^/       ~ /'
  fi
  roam_tempo_start
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
    t_move=$SECONDS
    out=$(timeout $HUNG $D dev --port $P move act "$ROAM" "plate=$plate" $(roam_walk_options "$way") \
      "$situation" 2>&1)
    t_plate=$((SECONDS - t_move))
    # The plate is on screen, so the perception of it starts at once; the move's bookkeeping below
    # runs while the model looks (the bookkeeping is a score of short commands that took seconds).
    said=$(printf '%s\n' "$out" | sed -n "s/^$ROAM says: //p" | head -1)
    picture=$(printf '%s\n' "$out" | sed -n 's/^picture: //p' | head -1)
    trace=$(printf '%s\n' "$out" | grep '^TRANSITION ')
    live=$(stage_at "$trace" live)
    if [ -n "$live" ] && [ "$way" != _show_ ]; then
      roam_view_start "$number" "$said" "$picture"
    fi
    reading=$(printf '%s\n' "$out" | sed -n 's/^reading: move //p; s/^\(not moved: .*\)/\1/p' |
      head -1)
    place=$(printf '%s\n' "$out" | sed -n 's/^design: place=\(.*\) mood=.*/\1/p' | head -1)
    things=$(printf '%s\n' "$out" | grep '^thing ')
    interpreted=$(stage_at "$trace" designing)
    designed=$(stage_at "$trace" briefing)
    briefed=$(stage_at "$trace" generating)
    painted=$(stage_at "$trace" locating)
    printf '[move %02d] %s | said: %s | read: %s | place: %s | plate: %s | jev_ms=%s design_ms=%s plate_ms=%s live_at_ms=%s\n' \
      "$number" "$way" "${said:-<none>}" "${reading:-<none>}" "${place:-<none>}" "$plate" \
      "${interpreted:--}" "$(span "$interpreted" "$designed")" "$(span "$briefed" "$painted")" \
      "${live:--}"
    roam_inventory "$said" "$out"
    roam_refs "${events_at:-0}" "$plate" "$number" "$out"
    printf '%s\n' "$out" | grep -E '^(sheet |place )' | sed 's/^/       /'
    roam_walk_lines "$number" "$out"
    if [ -z "$live" ]; then
      printf '       <move %02d did not go live: %s>\n' "$number" "$(printf '%s\n' "$out" | tail -1)"
      continue
    fi
    here=$place
    visited="${visited:+$visited; }$place"
    roam_remember_scene "$number" "$plate" "$place" "$picture"
    roam_tempo "$number" arrival "$way"
    if [ "$way" = _show_ ]; then
      roam_tempo "$number" show
      roam_show "$number" "$plate" "$place" "$said"
      continue
    fi
    t_look_from=$SECONDS
    start_watch "${FRAMES_LOG%.log}-$plate.log" "frames=$plate-look" "scene=$picture" \
      ${ROAM_DECLARE_HOLD:+"declare_hold=$ROAM_DECLARE_HOLD"} ${WATCH_REFS:+"$WATCH_REFS"} \
      "${FRAMES[@]}"
    roam_view_finish "$number"
    # What he noticed and added to the place is a thing to look at closely like the designed ones.
    things="$things${PERCEIVED_THINGS:+${things:+$'\n'}$PERCEIVED_THINGS}"
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
      [ -n "$inspected" ] && roam_tempo "$number" look
    fi
    # What the study is of: the thing looked at closely, else the place as the design describes it.
    thing_name=$(printf '%s\n' "$things" | sed -n "s/^thing $inspected (\([^)]*\)).*/\1/p" | head -1)
    thing_look=$(printf '%s\n' "$things" | sed -n "s/^thing $inspected ([^)]*): //p" | head -1)
    if [ -z "$thing_name" ]; then
      thing_name=$place
      thing_look=$(printf '%s\n' "$out" | sed -n 's/^elements: //p' | head -1)
    fi
    board_at=$(item_count "$ART_BOARD")
    t_declare_from=$SECONDS
    t_look=$((t_declare_from - t_look_from))
    TIME_STUDY=0
    TIME_REVEAL=0
    STUDY_PID=""
    t_declare=""
    STUDY_THING_NAME=$thing_name
    STUDY_THING_LOOK=$thing_look
    STUDY_BOARD_AT=$board_at
    if [ -n "${ARRIVAL[$way]:-}" ]; then
      # This kind of move has its own arrival step (a trade at a market), not the declaration.
      roam_trade "$number" "${line:-$said}" "$place" "${ARRIVAL[$way]}"
    elif [ -n "$ROAM_DECLARE" ]; then
      # The declaration is the painting line: spoken, judged for the supplies it uses and drawn.
      roam_tempo "$number" declare
      roam_declare "$number" "${line:-$said}" "$place"
      paint_line=$DECLARED_LINE
      t_declare=$((SECONDS - t_declare_from))
      if [ -n "$paint_line" ]; then
        [ -n "${STUDY_PID:-}" ] ||
          roam_study_start "$number" "$plate" "$thing_name" "$thing_look" "$board_at" "$place" \
            "$paint_line"
        roam_study_finish "$number" "$plate" "$plate"
      fi
    elif [ -n "$ROAM_PAINT" ]; then
      paint=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true \
        "recent=$ROAM: ${line:-$said}" "intent=$ROAM_PAINT" "You are at $place.$MIND" 2>&1)
      paint_line=$(printf '%s\n' "$paint" | sed -n '1s/.*says: //p')
      printf '[paint %02d] %s\n' "$number" "${paint_line:-<no line: $paint>}"
      roam_inventory "$paint_line" "$paint"
      t_declare=$((SECONDS - t_declare_from))
      if [ -n "$paint_line" ]; then
        roam_study_start "$number" "$plate" "$thing_name" "$thing_look" "$board_at" "$place" \
          "$paint_line"
        roam_study_finish "$number" "$plate" "$plate"
      fi
    fi
    t_declare=${t_declare:-$((SECONDS - t_declare_from))}
    roam_refused "${events_at:-0}" "$number"
    held_after=$(held_counts)
    printf '[inventory %02d] %s\n' "$number" "$(held_diff "$held_before" "$held_after")"
    printf '[time %02d] move took %s s (plate %s, of which walk %s, look %s, declare %s, study %s, reveal %s)\n' \
      "$number" "$((SECONDS - t_move))" "$t_plate" "$WALK_SECONDS" "$t_look" "$t_declare" \
      "$TIME_STUDY" "$TIME_REVEAL"
  done
  stop_watch quiet
  if [ -n "$SCENE_KIND$ART_KIND" ]; then
    $D dev --port $P world memory 2>&1 | sed 's/^/[memory] /'
  fi
}
[ -n "$ROAM" ] && play_roam

# The dice gauntlet (`@gauntlet`, see the header): the counters of the closing summary, then the
# helpers, one station and the run. Everything it reads of a station is in `@gauntlet_stations`.
declare -A G_KINDS G_DICE G_MODES G_NARRATOR_KINDS
G_RUN=0
G_SKIPPED=0
G_STUCK=0
G_FAILED=0
G_NO_ROLL=0
G_NO_CHECK_FIRST=0
G_ROLLED=0
G_PLAYER=0
G_NARRATOR=0
G_MATCH=0
G_DIFFER=0
G_JUDGE_RETRIES=0
G_LINE_RETRIES=0
G_REFUSALS=0
G_INSPIRED=0
G_SHORT=0
G_CARD_ONE=0
G_CARD_TWENTY=0
G_D20_ONE=0
G_D20_TWENTY=0
G_TOTAL_SECONDS=0
G_LONGEST=0
G_SHORTEST=0
G_STUCK_IDS=""
G_PROBLEMS=0
STATION_ID=""
STATION_BEGAN=0
GAUNTLET_DEADLINE=0
CHECK_ARGS=()
CHECK_LAUNCHED=""
GAUNTLET_INTENT=""
GAUNTLET_TOLD=""
declare -A NO_WARRIOR
G_PREP_FAILED=0
G_PREP_ONLY=0

gauntlet_trim() { # text without the spaces around it
  printf '%s' "$1" | sed 's/^ *//;s/ *$//'
}
# A string field of the station $1 in the stations file (`null` for a null, nothing for no field).
gauntlet_field() { # station id, key
  awk -v id="$1" -v key="$2" '
    $0 == "      \"id\": \"" id "\"," { inside = 1; next }
    inside && /^      "id": / { exit }
    inside && index($0, "      \"" key "\": ") == 1 {
      value = substr($0, length("      \"" key "\": ") + 1)
      sub(/,$/, "", value); gsub(/^"|"$/, "", value); print value; exit }
  ' "$GAUNTLET_STATIONS"
}
# The station ids of the stations file, in its order.
gauntlet_ids() {
  sed -n 's/^      "id": "\(.*\)",$/\1/p' "$GAUNTLET_STATIONS"
}
# The plate and the place of a station: its own in the stations file, or what `@station_plate` and
# `@station_place` give every run of that id (a bout at the ford for the attack and damage stations).
gauntlet_plate_of() { # station id
  printf '%s\n' "${STATION_PLATE[$1]:-$(gauntlet_field "$1" plate)}"
}
gauntlet_setting_of() { # station id
  printf '%s\n' "${STATION_PLACE[$1]:-$(gauntlet_field "$1" setting)}"
}
# The kinds the stations file allows.
gauntlet_kinds() {
  sed -n '/"kinds": \[/,/\]/p' "$GAUNTLET_STATIONS" | sed -n 's/^ *"\([a-z_]*\)",\{0,1\}$/\1/p'
}
# Sleeps for $1 milliseconds.
gauntlet_pause() {
  sleep "$(($1 / 1000)).$(printf '%03d' $(($1 % 1000)))"
}
# Milliseconds as seconds with one decimal, or - when unknown.
gauntlet_s() {
  case "$1" in
    ''|-|*[!0-9]*) printf '%s' - ;;
    *) printf '%s.%s' "$((($1 + 50) / 1000))" "$(((($1 + 50) % 1000) / 100))" ;;
  esac
}
# Runs a command for what is left of the station's time; 124 when that has run out (stuck).
gauntlet_timed() {
  local left=$((GAUNTLET_DEADLINE - SECONDS))
  [ "$left" -gt 0 ] || return 124
  timeout "$left" "$@"
}
# The text of the host's audit lines of an area after event $1, one to a line.
gauntlet_lines() { # after-sequence, area
  local sequence text
  $D dev --port $P events "${1:-0}" "area=$2" limit=2000 2>/dev/null | grep -E '^ *[0-9]+ ' |
    while read -r sequence _ text; do printf '%s\n' "$text"; done
}
# The value of key $3 in the last of the lines $1 that begin with $2.
gauntlet_last() { # lines, prefix, key
  field_of "$(printf '%s\n' "$1" | grep -F "$2" | tail -1)" "$3"
}
# How many of the lines $1 begin with $2.
gauntlet_count() { # lines, prefix
  printf '%s\n' "$1" | grep -c "^$2" || true
}
# The test a station was designed for as `intent=` writes it: `<kind>[:<ability>[:<skill>]]`, the
# abilities by the roll vocabulary's keys (the stations file has the short ones).
gauntlet_intent() { # station id
  local intent ability skill
  intent=$(gauntlet_field "$1" kind)
  ability=$(gauntlet_field "$1" ability)
  skill=$(gauntlet_field "$1" skill)
  case "$ability" in
    str) ability=strength ;;
    dex) ability=dexterity ;;
    con) ability=constitution ;;
    int) ability=intelligence ;;
    wis) ability=wisdom ;;
    cha) ability=charisma ;;
    *) ability="" ;;
  esac
  [ -n "$ability" ] && intent="$intent:$ability"
  [ -n "$ability" ] && [ -n "$skill" ] && [ "$skill" != null ] && intent="$intent:$skill"
  printf '%s' "$intent"
}
# The options of `persona check run` for the station $1 (initiated by $2, who said $3) in CHECK_ARGS:
# the files, the moment, what is in sight and the die Heroic Inspiration is spent on.
gauntlet_check_args() { # station id, player|narrator, what he said, the scene line just spoken
  local sight
  CHECK_ARGS=("sheet=$GAUNTLET_SHEET" "stations=$GAUNTLET_STATIONS" "station=$1")
  if [ "$2" = player ]; then
    CHECK_ARGS+=("moment=player" "action=$3")
  else
    CHECK_ARGS+=("moment=narrator")
    # The narrator's own authored test, once this host's `persona check` takes `intent=`.
    [ -n "$GAUNTLET_INTENT" ] && CHECK_ARGS+=("intent=$(gauntlet_intent "$1")")
  fi
  while IFS= read -r sight; do
    [ -n "$sight" ] && CHECK_ARGS+=("sight=$sight")
  done <<< "${STATION_SIGHT[$1]:-}"
  [ -n "${STATION_INSPIRE[$1]:-}" ] && CHECK_ARGS+=("inspiration=${STATION_INSPIRE[$1]}")
  [ -n "$GAUNTLET_TOLD" ] && [ -n "$4" ] && CHECK_ARGS+=("told=$4")
  [ -n "${STATION_PLACE[$1]:-}" ] && CHECK_ARGS+=("place=${STATION_PLACE[$1]}")
}
gauntlet_problem() {
  printf '[plan] PROBLEM: %s\n' "$*"
  G_PROBLEMS=$((G_PROBLEMS + 1))
}

# `--plan`: every station's steps and their checks against the files, with no call to the host. The
# exit status is 0 when nothing is wrong.
gauntlet_plan() {
  local number=0 spec id who scene goal title kind plate seen=" " kinds file heard plates=0 repeat
  local media=${DF_PLAYGROUND:-artifacts/tmp/persona-live/playground}/media
  printf '[plan] %s with %s stations of %s (media %s)\n' "$GAUNTLET" "${#STATIONS[@]}" \
    "$GAUNTLET_STATIONS" "$media"
  for file in "$GAUNTLET_PERSONA" "$GAUNTLET_SHEET" "$GAUNTLET_STATIONS"; do
    [ -f "$file" ] || gauntlet_problem "$file does not exist"
  done
  if [ ! -f "$GAUNTLET_STATIONS" ]; then
    printf 'PLAN RESULT FAIL (%s problems)\n' "$G_PROBLEMS"
    exit 1
  fi
  kinds=$(gauntlet_kinds)
  # `intent=` and `told=` are shown as they are sent once this host's `persona check` usage text has them.
  GAUNTLET_INTENT=yes
  GAUNTLET_TOLD=yes
  [ -n "$GAUNTLET_LOOK" ] || gauntlet_problem "no @gauntlet_look: the prep has nothing to name him by"
  [ -f "$media/${GAUNTLET_PORTRAIT:-ref-$GAUNTLET.png}" ] ||
    gauntlet_problem "portrait ${GAUNTLET_PORTRAIT:-ref-$GAUNTLET.png} is not in $media"
  for spec in "${STATIONS[@]}"; do
    number=$((number + 1))
    IFS='|' read -r id who scene goal <<< "$spec"
    id=$(gauntlet_trim "$id"); who=$(gauntlet_trim "$who")
    scene=$(gauntlet_trim "$scene"); goal=$(gauntlet_trim "$goal")
    title=$(gauntlet_field "$id" title); kind=$(gauntlet_field "$id" kind)
    plate=$(gauntlet_plate_of "$id")
    if [ -z "$title" ]; then
      gauntlet_problem "station $number: no station $id in the stations file"
      continue
    fi
    repeat=""
    case "$seen" in *" $id "*) repeat=" (repeat)" ;; esac
    seen="$seen$id "
    printf '%s\n' "$kinds" | grep -qx "$kind" ||
      gauntlet_problem "station $id: kind $kind is not one of the file's kinds"
    case "$who" in
      player)
        [ -n "$goal" ] || gauntlet_problem "station $id: a player station needs what he sets out to do"
        G_PLAYER=$((G_PLAYER + 1)) ;;
      narrator)
        [ -z "$goal" ] || gauntlet_problem "station $id: a narrator station has no fourth field"
        G_NARRATOR=$((G_NARRATOR + 1))
        G_NARRATOR_KINDS[$kind]=$((${G_NARRATOR_KINDS[$kind]:-0} + 1)) ;;
      *) gauntlet_problem "station $id: the initiator is player or narrator, not '$who'" ;;
    esac
    [ -n "$scene" ] || gauntlet_problem "station $id: no scene line for the dungeon master"
    case "$scene" in *';'*) gauntlet_problem "station $id: the scene line has a semicolon" ;; esac
    if [ -f "$media/$plate" ]; then
      plates=$((plates + 1))
    else
      gauntlet_problem "station $id: plate $plate is not in $media"
    fi
    heard=$((3 + ${#scene} * 70 / 1000))
    printf '[plan %02d] %s%s | %s | %s | %s | ability=%s skill=%s | plate %s\n' "$number" "$id" \
      "$repeat" "$title" "$kind" "$who" "$(gauntlet_field "$id" ability)" "$(gauntlet_field "$id" skill)" "$plate"
    printf '          1 plate  scene image tv-backdrop %s (page fade %s ms)\n' "$plate" "$GAUNTLET_FADE"
    printf '          2 dm     persona narrate speak=true "%s" (heard out in %s s)\n' "$scene" "$heard"
    if [ "$who" = player ]; then
      printf '          3 says   persona say %s speak=true "intent=%s"\n' "$GAUNTLET" "$goal"
    else
      printf '          3 says   nothing: the dungeon master calls the check\n'
    fi
    gauntlet_check_args "$id" "$who" "<the line he said>" "$scene"
    printf '          4 check  persona check run %s' "$GAUNTLET"
    printf ' "%s"' "${CHECK_ARGS[@]}"
    printf '\n'
  done
  printf '[plan] prep, before the first station (%s plates, once each):\n' \
    "$(gauntlet_plates | wc -l | tr -d ' ')"
  printf '          media locate show <plate>: a layout with %s named means already; no layout: media locate <plate>\n' \
    "$GAUNTLET"
  printf '          then media locate identify <plate> "%s=%s" "portrait.%s=%s"\n' "$GAUNTLET" \
    "$GAUNTLET_LOOK" "$GAUNTLET" "${GAUNTLET_PORTRAIT:-ref-$GAUNTLET.png}"
  printf '          plates:'
  for plate in $(gauntlet_plates); do printf ' %s' "$plate"; done
  printf '\n          a plate where he is not found makes its stations narrator-only (no persona say)\n'
  for id in $(gauntlet_ids); do
    case "$seen" in
      *" $id "*) ;;
      *) gauntlet_problem "station $id of the stations file is not in the beats" ;;
    esac
  done
  for id in "${!STATION_SIGHT[@]}" "${!STATION_INSPIRE[@]}" "${!STATION_PLATE[@]}" "${!STATION_PLACE[@]}"; do
    case "$seen" in
      *" $id "*) ;;
      *) gauntlet_problem "a @station_ line names $id, which is not a station" ;;
    esac
  done
  [ "${#STATION_INSPIRE[@]}" -le 1 ] ||
    gauntlet_problem "Heroic Inspiration is spent once, ${#STATION_INSPIRE[@]} stations spend it"
  printf '[plan] stations=%s of %s in the file, player=%s narrator=%s, plates found=%s\n' \
    "${#STATIONS[@]}" "$(gauntlet_ids | wc -l | tr -d ' ')" "$G_PLAYER" "$G_NARRATOR" "$plates"
  printf '[plan] narrator-initiated by kind:'
  for kind in $(printf '%s\n' "${!G_NARRATOR_KINDS[@]}" | sort); do
    printf ' %s=%s' "$kind" "${G_NARRATOR_KINDS[$kind]}"
  done
  printf '\n[plan] sights on: %s | Heroic Inspiration on: %s\n' \
    "$(printf '%s\n' "${!STATION_SIGHT[@]}" | sort | paste -sd ' ' -)" \
    "$(printf '%s\n' "${!STATION_INSPIRE[@]}" | sort | paste -sd ' ' -)"
  if [ "$G_PROBLEMS" -eq 0 ]; then
    printf 'PLAN RESULT PASS\n'
    exit 0
  fi
  printf 'PLAN RESULT FAIL (%s problems)\n' "$G_PROBLEMS"
  exit 1
}

# The plates of the stations, each once, in the order they are met.
gauntlet_plates() {
  local spec id
  for spec in "${STATIONS[@]}"; do
    IFS='|' read -r id _ <<< "$spec"
    gauntlet_plate_of "$(gauntlet_trim "$id")"
  done | awk 'NF && !seen[$0]++'
}

# Before the first station every plate must hold a layout with the character named, or the host
# refuses his line (`no_layout`, `unknown_character`). A plate whose `media locate show` already names
# him is `already`; one with no layout is scanned (`media locate`); one without him named is asked
# (`media locate identify`, by his look and his reference sheet, never a name). A plate where he is
# not found, or the prep failed, is remembered in NO_WARRIOR: its stations are the narrator's.
gauntlet_prep() {
  local plate shown out rc result portrait=${GAUNTLET_PORTRAIT:-ref-$GAUNTLET.png}
  local total=0 already=0 scanned=0 named=0 failed=0 only=""
  while IFS= read -r plate; do
    total=$((total + 1))
    shown=$($D dev --port $P media locate show "$plate" 2>&1)
    rc=$?
    if [ "$rc" -eq 0 ] && printf '%s\n' "$shown" | grep -q "^ *[0-9]*: $GAUNTLET \["; then
      printf '[prep] %s already\n' "$plate"
      already=$((already + 1))
      continue
    fi
    if [ "$rc" -ne 0 ]; then
      out=$(timeout 300 $D dev --port $P media locate "$plate" 2>&1)
      rc=$?
      if [ "$rc" -ne 0 ]; then
        printf '[prep] %s FAILED(scan: %s)\n' "$plate" "$(printf '%s\n' "$out" | head -1)"
        failed=$((failed + 1)); NO_WARRIOR[$plate]=1; only="$only $plate"
        continue
      fi
      printf '[prep] %s scanned\n' "$plate"
      scanned=$((scanned + 1))
    fi
    out=$(timeout 300 $D dev --port $P media locate identify "$plate" \
      "$GAUNTLET=$GAUNTLET_LOOK" "portrait.$GAUNTLET=$portrait" 2>&1)
    rc=$?
    result=$(printf '%s\n' "$out" | sed -n "s/^  $GAUNTLET: .* -> \(.*\)\$/\1/p" | head -1)
    if [ "$rc" -eq 0 ] && [ "$result" = named ]; then
      printf '[prep] %s named\n' "$plate"
      named=$((named + 1))
    else
      printf '[prep] %s FAILED(%s)\n' "$plate" \
        "${result:-identify: $(printf '%s\n' "$out" | head -1)}"
      failed=$((failed + 1)); NO_WARRIOR[$plate]=1; only="$only $plate"
    fi
  done < <(gauntlet_plates)
  G_PREP_FAILED=$failed
  printf '[prep] %s plates: already=%s scanned=%s named=%s failed=%s | narrator-only plates:%s\n' \
    "$total" "$already" "$scanned" "$named" "$failed" "${only:- none}"
}

# A step of a station ended it: rc 124 is the station running out of its time (stuck), anything else
# an error that skips it. A check that was under way goes on in the host, so its card is waited for.
gauntlet_halt() { # number, rc, step, output, after-sequence
  if [ "$2" = 124 ]; then
    printf '[station %02d] stuck at %s after %s s (limit %s s): skipped\n' "$1" "$3" \
      "$((SECONDS - STATION_BEGAN))" "$GAUNTLET_LIMIT"
    G_STUCK=$((G_STUCK + 1))
    G_STUCK_IDS="${G_STUCK_IDS:+$G_STUCK_IDS }$STATION_ID"
  else
    printf '[station %02d] skipped at %s: %s\n' "$1" "$3" "$(printf '%s\n' "$4" | head -1)"
    G_SKIPPED=$((G_SKIPPED + 1))
  fi
  [ "$3" = check ] && [ "$CHECK_LAUNCHED" = yes ] && gauntlet_settle "$1" "$5"
  return 0
}
# A check the station gave up on is still in the host's flow. The card is cleared (`scene roll-clear`),
# the flow is waited for until a `CHECK cleared`, a `CHECK skipped` or a refusal of the check ends it
# (two minutes at most), and the card is cleared once more, so a stuck station cannot cover the next.
gauntlet_settle() { # number, after-sequence
  local waited=0 chk idle=no
  $D dev --port $P scene roll-clear 2>&1 | head -1 | sed 's/^/       ~ /'
  while [ "$waited" -lt 120 ]; do
    chk=$({ gauntlet_lines "$2" CHECK; gauntlet_lines "$2" PERSONA | grep '^PERSONA refused verb=check'; })
    if printf '%s\n' "$chk" | grep -q '^\(CHECK cleared\|CHECK skipped\|PERSONA refused verb=check\)'; then
      idle=yes
      break
    fi
    sleep 2
    waited=$((waited + 2))
  done
  $D dev --port $P scene roll-clear 2>&1 | head -1 | sed 's/^/       ~ /'
  if [ "$idle" = yes ]; then
    printf '[station %02d] cleared after stuck (the host was idle %s s later)\n' "$1" "$waited"
  else
    printf '[station %02d] cleared after stuck, but the host did not end its check within %s s\n' \
      "$1" "$waited"
  fi
}

# What the check run's answer says, as the `[check]`, `[roll]` and `[dm]` lines (and the tallies).
gauntlet_report() { # number, the answer of persona check run
  local number=$1 header call designed roll called result faces die sides face kept mode kind title
  local match dice
  header=$(printf '%s\n' "$2" | sed -n '1p')
  call=$(printf '%s\n' "$2" | sed -n 's/^  call: //p')
  designed=$(printf '%s\n' "$2" | sed -n 's/^  designed: //p')
  roll=$(printf '%s\n' "$2" | sed -n 's/^  roll: //p')
  called=$(printf '%s\n' "$2" | sed -n 's/^  called ([^)]*): //p')
  result=$(printf '%s\n' "$2" | sed -n 's/^  result ([^)]*): //p')
  if [ "$(field_of "$header" required)" != true ]; then
    printf '[check %02d] required=%s initiator=%s reason=%s: no roll was called\n' "$number" \
      "$(field_of "$header" required)" "$(field_of "$header" initiator)" \
      "$(field_of "$header" reason)"
    G_NO_ROLL=$((G_NO_ROLL + 1))
    return
  fi
  kind=$(field_of "$call" kind)
  mode=$(field_of "$call" mode)
  case "$(field_of "$designed" match)" in
    true) match=match; G_MATCH=$((G_MATCH + 1)) ;;
    false) match="differs($(field_of "$designed" kind))"; G_DIFFER=$((G_DIFFER + 1)) ;;
    *) match=- ;;
  esac
  printf '[check %02d] required=true kind=%s ability=%s skill=%s dc=%s mode=%s grounds=%s uses=%s initiator=%s reason=%s retried=%s judge_ms=%s designed=%s\n' \
    "$number" "$kind" "$(field_of "$call" ability)" "$(field_of "$call" skill)" \
    "$(field_of "$call" dc)" "$mode" "$(field_of "$call" grounds)" "$(field_of "$call" uses)" \
    "$(field_of "$header" initiator)" "$(field_of "$header" reason)" "$(field_of "$header" retried)" \
    "$(field_of "$header" duration_ms)" "$match"
  faces=$(field_of "$roll" dice)
  title=$(printf '%s\n' "$roll" | sed 's/ kind=.*//')
  printf '[roll %02d] faces=%s total=%s outcome=%s natural=%s card=%s target=%s (%s)\n' "$number" \
    "${faces:--}" "$(field_of "$roll" total)" "$(field_of "$roll" outcome)" \
    "$(field_of "$roll" natural)" "$(field_of "$roll" kind)" \
    "$(printf '%s\n' "$roll" | sed -n 's/.* target=\(.*\) outcome=.*/\1/p')" "$title"
  printf '[dm %02d] call: %s\n' "$number" "${called:--}"
  printf '[dm %02d] result: %s\n' "$number" "${result:--}"
  G_ROLLED=$((G_ROLLED + 1))
  G_KINDS[$kind]=$((${G_KINDS[$kind]:-0} + 1))
  G_MODES[$mode]=$((${G_MODES[$mode]:-0} + 1))
  case "$(field_of "$roll" natural)" in
    one) G_CARD_ONE=$((G_CARD_ONE + 1)) ;;
    twenty) G_CARD_TWENTY=$((G_CARD_TWENTY + 1)) ;;
  esac
  IFS=',' read -ra dice <<< "$faces"
  for die in "${dice[@]}"; do
    sides=${die%%=*}
    face=${die#*=}
    kept=true
    case "$face" in *x) kept=false; face=${face%x} ;; esac
    G_DICE[$sides]=$((${G_DICE[$sides]:-0} + 1))
    if [ "$sides" = d20 ] && [ "$kept" = true ]; then
      case "$face" in
        1) G_D20_ONE=$((G_D20_ONE + 1)) ;;
        20) G_D20_TWENTY=$((G_D20_TWENTY + 1)) ;;
      esac
    fi
  done
}

# What the host's own audit lines say of the station's check (events after $2): the times of the card
# and of the dungeon master's lines (`CHECK`, `TIMING roll`), the Heroic Inspiration spent (`RULES
# rolled`) and the judge's refusals and retries (`PERSONA check_*`).
gauntlet_audit() { # number, after-sequence, judge ms, seconds the station took
  local number=$1 chk persona rules page card drawn shown min_hold rstart rend cleared hold
  local ok=ok tries judge line_retries refused replaced old new total mode target voice seen_ms
  chk=$(gauntlet_lines "$2" CHECK)
  drawn=$(gauntlet_last "$chk" 'CHECK rolled ' drawn_ms)
  shown=$(gauntlet_last "$chk" 'CHECK shown ' at_ms)
  card=$(gauntlet_last "$chk" 'CHECK shown ' id)
  min_hold=$(gauntlet_last "$chk" 'CHECK shown ' min_hold_ms)
  rstart=$(gauntlet_last "$chk" 'CHECK result_read ' start_ms)
  rend=$(gauntlet_last "$chk" 'CHECK result_read ' end_ms)
  cleared=$(gauntlet_last "$chk" 'CHECK cleared ' at_ms)
  hold=$(gauntlet_last "$chk" 'CHECK cleared ' hold_ms)
  if [ -n "$shown" ] && [ -n "$cleared" ] && [ -n "$rend" ] && [ -n "$hold" ]; then
    if [ "$hold" -lt 6000 ] || [ "$((cleared - rend))" -lt 2000 ]; then
      ok=SHORT
      G_SHORT=$((G_SHORT + 1))
    fi
  else
    ok=unknown
  fi
  # The page's own record of the card (`TIMING roll`), which reaches the host a moment after the card goes.
  page=""
  for tries in 1 2 3; do
    [ -n "$card" ] || break
    page=$(gauntlet_lines "$2" TIMING | grep -F "TIMING roll " | grep -F " id=$card " | tail -1)
    [ -n "$page" ] && break
    sleep 1
  done
  voice=$(gauntlet_last "$chk" 'CHECK result_read ' voice)
  seen_ms=$(field_of "$page" shown_ms)
  printf '[hold %02d] card_up_ms=%s dm_result_ms=%s hold_ms=%s min_hold_ms=%s result_to_clear_ms=%s page_card_ms=%s voice=%s held=%s\n' \
    "$1" "$(span "$shown" "$cleared")" "$(span "$rstart" "$rend")" "${hold:--}" "${min_hold:--}" \
    "$(span "$rend" "$cleared")" "${seen_ms:--}" "${voice:--}" "$ok"
  printf '[time %02d] station took %s s (judge %s, call %s, roll %s, result %s, hold %s)\n' "$1" \
    "$4" "$(gauntlet_s "$3")" "$(gauntlet_s "$(span "$drawn" "$shown")")" \
    "$(gauntlet_s "$(span "$shown" "$rstart")")" "$(gauntlet_s "$(span "$rstart" "$rend")")" \
    "$(gauntlet_s "$(span "$rend" "$cleared")")"
  persona=$(gauntlet_lines "$2" PERSONA)
  judge=$(gauntlet_count "$persona" 'PERSONA check_judge_retried')
  line_retries=$(gauntlet_count "$persona" 'PERSONA check_line_retried')
  refused=$(gauntlet_count "$persona" 'PERSONA refused verb=check')
  G_JUDGE_RETRIES=$((G_JUDGE_RETRIES + judge))
  G_LINE_RETRIES=$((G_LINE_RETRIES + line_retries))
  G_REFUSALS=$((G_REFUSALS + refused))
  printf '[host %02d] judge_retries=%s line_retries=%s refused=%s\n' "$1" "$judge" \
    "$line_retries" "$refused"
  rules=$(gauntlet_lines "$2" RULES | grep '^RULES rolled ' | tail -1)
  replaced=$(field_of "$rules" replaced)
  if [ -n "$replaced" ] && [ "$replaced" != - ]; then
    G_INSPIRED=$((G_INSPIRED + 1))
    mode=$(field_of "$rules" mode)
    old=$(printf '%s' "$replaced" | cut -d: -f2)
    new=$(field_of "$rules" dice | cut -d, -f1 | cut -d: -f2)
    total=$(field_of "$rules" total)
    target=$(field_of "$rules" target)
    printf '[inspire %02d] Heroic Inspiration replaced %s (was %s, now %s) mode=%s modifiers=%s total_now=%s' \
      "$1" "$replaced" "$old" "$new" "$mode" "$(field_of "$rules" modifiers)" "$total"
    if [ "$mode" = normal ]; then
      printf ' total_before=%s target=%s\n' "$((total - new + old))" "$target"
    else
      printf ' total_before=- target=%s\n' "$target"
    fi
  fi
}

# One station: the plate, the dungeon master's scene, his line, the check, and what the host says of it.
gauntlet_station() { # number, the @station line
  local number=$1 id who scene goal title kind plate setting out rc said="" after took attempt header
  IFS='|' read -r id who scene goal <<< "$2"
  id=$(gauntlet_trim "$id"); who=$(gauntlet_trim "$who")
  scene=$(gauntlet_trim "$scene"); goal=$(gauntlet_trim "$goal")
  STATION_ID=$id
  title=$(gauntlet_field "$id" title); kind=$(gauntlet_field "$id" kind)
  plate=$(gauntlet_plate_of "$id"); setting=$(gauntlet_setting_of "$id")
  STATION_BEGAN=$SECONDS
  GAUNTLET_DEADLINE=$((SECONDS + GAUNTLET_LIMIT))
  printf '[station %02d] %s | %s | %s\n' "$number" "$id" "${title:-?}" "${kind:-?}"
  if [ -z "$title" ]; then
    gauntlet_halt "$number" 1 "stations file" "the stations file has no station $id" 0
    return
  fi
  after=$(latest_event)
  after=${after:-0}
  # He is not in every plate (the prep found him in none of this one): the dungeon master sets the
  # scene and calls the check, and nobody speaks.
  if [ "$who" = player ] && [ -n "${NO_WARRIOR[$plate]:-}" ]; then
    printf '[station %02d] narrator-only: %s has no %s named in it, so there is no persona say\n' \
      "$number" "$plate" "$GAUNTLET"
    who=narrator
    G_PREP_ONLY=$((G_PREP_ONLY + 1))
  fi
  # (a) the plate, with the page's fade
  out=$(gauntlet_timed $D dev --port $P scene image tv-backdrop "$plate" 2>&1)
  rc=$?
  if [ "$rc" -ne 0 ]; then gauntlet_halt "$number" "$rc" plate "$out" "$after"; return; fi
  gauntlet_pause "$GAUNTLET_FADE"
  # (b) the dungeon master sets the scene, and is heard out before anyone answers
  printf '[dm %02d] scene: %s\n' "$number" "$scene"
  out=$(gauntlet_timed $D dev --port $P persona narrate speak=true "$scene" 2>&1)
  rc=$?
  if [ "$rc" -ne 0 ]; then gauntlet_halt "$number" "$rc" scene "$out" "$after"; return; fi
  heard_out "$scene"
  wait_heard
  # (c) he says what he does; at a narrator's station the dungeon master calls the check unasked
  # (d) judge, call, dice, card, result, clear. A player's line the judge calls no roll for, at a
  # station designed as a roll, is logged and asked once more with a firmer goal; both are logged.
  attempt=1
  while :; do
    if [ "$who" = player ]; then
      out=$(gauntlet_timed $D dev --port $P persona say "$GAUNTLET" speak=true \
        "recent=The dungeon master: $scene" "intent=$goal" "Where you are: $setting" 2>&1)
      rc=$?
      said=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
      if [ "$rc" -ne 0 ] || [ -z "$said" ]; then
        gauntlet_halt "$number" "$rc" says "${out:-he said nothing}" "$after"
        return
      fi
      printf '[said %02d] %s%s\n' "$number" "$([ "$attempt" -gt 1 ] && printf 'retry: ')" "$said"
      heard_out "$said"
      wait_heard
    else
      printf '[said %02d] (nobody: the dungeon master calls the check)\n' "$number"
    fi
    gauntlet_check_args "$id" "$who" "$said" "$scene"
    CHECK_LAUNCHED=""
    [ $((GAUNTLET_DEADLINE - SECONDS)) -gt 0 ] && CHECK_LAUNCHED=yes
    out=$(gauntlet_timed $D dev --port $P persona check run "$GAUNTLET" "${CHECK_ARGS[@]}" 2>&1)
    rc=$?
    if [ "$rc" -ne 0 ]; then
      if [ "$rc" = 124 ]; then
        gauntlet_halt "$number" "$rc" check "$out" "$after"
      else
        printf '[check %02d] failed: %s\n' "$number" "$(printf '%s\n' "$out" | head -1)"
        G_FAILED=$((G_FAILED + 1))
      fi
      return
    fi
    header=$(printf '%s\n' "$out" | sed -n '1p')
    [ "$(field_of "$header" required)" = true ] && break
    # No roll was called.
    printf '[station %02d] NO CHECK (judge: %s; he said: %s)\n' "$number" \
      "$(field_of "$header" reason)" "${said:-nobody, the dungeon master called it}"
    if [ "$who" = player ] && [ "$kind" != passive ] && [ "$attempt" -eq 1 ]; then
      G_NO_CHECK_FIRST=$((G_NO_CHECK_FIRST + 1))
      printf '[station %02d] retry: asking him again with "You commit to it: do it, in a sentence."\n' \
        "$number"
      goal="$goal You commit to it: do it, in a sentence."
      attempt=2
      continue
    fi
    [ "$attempt" -gt 1 ] && printf '[station %02d] NO CHECK after the retry: given up\n' "$number"
    break
  done
  G_RUN=$((G_RUN + 1))
  [ "$who" = player ] && G_PLAYER=$((G_PLAYER + 1)) || G_NARRATOR=$((G_NARRATOR + 1))
  gauntlet_report "$number" "$out"
  took=$((SECONDS - STATION_BEGAN))
  gauntlet_audit "$number" "$after" "$(field_of "$(printf '%s\n' "$out" | sed -n '1p')" duration_ms)" \
    "$took"
  G_TOTAL_SECONDS=$((G_TOTAL_SECONDS + took))
  [ "$took" -le "$G_LONGEST" ] || G_LONGEST=$took
  { [ "$G_SHORTEST" -ne 0 ] && [ "$G_SHORTEST" -le "$took" ]; } || G_SHORTEST=$took
}

# The closing table.
gauntlet_summary() { # seconds the run took
  local key
  printf '\n[summary] ------------------------------------------------------------\n'
  printf '[summary] stations planned=%s run=%s no_check=%s (first asks with no check, then retried: %s) rolled=%s skipped=%s stuck=%s failed=%s\n' \
    "${#STATIONS[@]}" "$G_RUN" "$G_NO_ROLL" "$G_NO_CHECK_FIRST" "$G_ROLLED" "$G_SKIPPED" "$G_STUCK" \
    "$G_FAILED"
  printf '[summary] stuck: %s\n' "${G_STUCK_IDS:--}"
  printf '[summary] initiated by: player=%s narrator=%s | prep: plates failed=%s, stations made narrator-only=%s\n' \
    "$G_PLAYER" "$G_NARRATOR" "$G_PREP_FAILED" "$G_PREP_ONLY"
  printf '[summary] rolls by kind:'
  for key in $(printf '%s\n' "${!G_KINDS[@]}" | sort); do printf ' %s=%s' "$key" "${G_KINDS[$key]}"; done
  printf '\n[summary] rolls by die size (set-aside dice included):'
  for key in $(printf '%s\n' "${!G_DICE[@]}" | sort -V); do printf ' %s=%s' "$key" "${G_DICE[$key]}"; done
  printf '\n[summary] modes:'
  for key in $(printf '%s\n' "${!G_MODES[@]}" | sort); do printf ' %s=%s' "$key" "${G_MODES[$key]}"; done
  printf ' | Heroic Inspiration spent=%s\n' "$G_INSPIRED"
  printf '[summary] natural marks on the card: one=%s twenty=%s | kept d20 faces: 1=%s 20=%s\n' \
    "$G_CARD_ONE" "$G_CARD_TWENTY" "$G_D20_ONE" "$G_D20_TWENTY"
  printf '[summary] the judge: designed test matched=%s differed=%s | retries judge=%s line=%s refusals=%s\n' \
    "$G_MATCH" "$G_DIFFER" "$G_JUDGE_RETRIES" "$G_LINE_RETRIES" "$G_REFUSALS"
  printf '[summary] cards held too short=%s\n' "$G_SHORT"
  printf '[summary] time: run %s s, stations %s s in all' "$1" "$G_TOTAL_SECONDS"
  if [ "$G_RUN" -gt 0 ]; then
    printf ', mean %s s, shortest %s s, longest %s s' "$((G_TOTAL_SECONDS / G_RUN))" "$G_SHORTEST" "$G_LONGEST"
  fi
  printf '\n'
}

# The run: store the character, set the fade, meet the stations one after the other.
play_gauntlet() {
  local number=0 spec began=$SECONDS usage
  [ -n "$PLAN" ] && gauntlet_plan
  printf '[gauntlet] %s: %s stations, fade %s ms, breath %s s, limit %s s a station\n' "$GAUNTLET" \
    "${#STATIONS[@]}" "$GAUNTLET_FADE" "$GAUNTLET_BREATH" "$GAUNTLET_LIMIT"
  $D dev --port $P persona write "$GAUNTLET" "$GAUNTLET_PERSONA" | head -1 | sed 's/^/       ~ /'
  $D dev --port $P persona state "$GAUNTLET" reset | head -1 | sed 's/^/       ~ /'
  $D dev --port $P playground set device=tv transition=fade "ms=$GAUNTLET_FADE" | head -1 |
    sed 's/^/       ~ /'
  # The narrator's authored test goes to the judge as `intent=`, and the scene line just spoken as
  # `told=` (so the call does not restate it), once this host's usage text has each.
  usage=$($D dev --port $P persona check 2>&1 </dev/null)
  printf '%s\n' "$usage" | grep -q 'intent=' && GAUNTLET_INTENT=yes
  printf '%s\n' "$usage" | grep -q 'told=' && GAUNTLET_TOLD=yes
  printf '[gauntlet] persona check takes intent=: %s\n' "${GAUNTLET_INTENT:-no}"
  printf '[gauntlet] persona check takes told=: %s\n' "${GAUNTLET_TOLD:-no}"
  gauntlet_prep
  for spec in "${STATIONS[@]}"; do
    number=$((number + 1))
    gauntlet_station "$number" "$spec"
    sleep "$GAUNTLET_BREATH"
  done
  gauntlet_summary "$((SECONDS - began))"
}
[ -n "$GAUNTLET" ] && play_gauntlet

stop_watch
