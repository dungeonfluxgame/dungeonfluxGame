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
# one line toward `@roam_show_say`. A painting is made with the price of `@roam_art_price <amount>
# <cp|sp|ep|gp|pp>` (5 sp when none), so it can be sold from the bag, and a painting the hero holds
# that was made with no price gets it before the first move (`[art] ..`).
# Free roam played from a phone (`@roam_player <seat>`, or `DF_RUN_PLAYER=<seat>` for one run): the
# seat's player says the traveller's three lines in their own words, spoken exactly as written. At
# each line point the runner asks the seat (`players ask`, the prompt of `@roam_player_ask_move`,
# `_look` or `_declare` and suggested cards), waits for its line (`players inbox`) and runs the same
# verb with the words given (`move act text=`, `persona say text=`, `persona declare text=`) instead
# of the model-worded line; everything after the line runs as before. The cards come from data the
# runner has: `@roam_player_card <way> <label> | <line>` for the move, a thing in sight (its name in
# `{thing}` of `@roam_player_look_line <label> | <line>`) for the look, and the thing studied for
# `@roam_player_declare_line <label> | <line>`. A move Jev cannot read as one is asked again with its
# reason as the note. Bind the seat first: `players persona <seat> <@roam id>`.
# The player's own hero as the traveller (`DF_RUN_HERO_ID`, set with `DF_RUN_ROAM` by the host
# that starts the table's scenario): before the first move the runner makes, once and reused when
# present, the hero's persona (`persona generate unplaced=true` with `@roam_hero_hint` and the
# locked design in words, `DF_RUN_HERO_LOOK`; its display name the hero's), the sheets of the kit
# the hero carries (`world inventory sheets`) and the starting plate `roam-00-<@roam>.png` painted
# from the hero's reference sheet (`DF_RUN_HERO_SHEET`, the look studio's, used by name)
# (`@roam_start_plate`), located and the hero named in it, and shows it (`[prepare] ..` lines).
# Nobody else's pictures are used; first the TV is made clean (the `painters_day` fixture, reset,
# no picture). A step that fails stops the runner with status 3 before the first ask: the host
# tells the phone the day could not start. The more line points the player says: the finishing
# line (`@roam_player_ask_painted`), the trade (`@roam_player_ask_trade`) and the show
# (`@roam_player_ask_show`), each with its cards (`@roam_player_painted_line`,
# `@roam_player_trade_line`, `@roam_player_show_line`).
# A place with no things worth a close look offers what the hero saw there as its look cards
# (`[sights NN]`, `@roam_player_sight_line`, `{thing}` the sight's words), each added to the
# host's place as a thing of it (`world thing add`, so its close look is a look like any thing's);
# one the host did not add is looked at by a focus on its words (`[insert NN] .. via=focus`). The
# declare and finish cards of a sight have their own lines (`@roam_player_declare_sight_line`,
# `@roam_player_painted_sight_line`). The declare and finish cards name what the player looked at:
# the insert's thing, else the card tapped. A card whose close look could not be made is told to
# the seat (`players note`, `[ask NN] notice to seat ..`). Talk from the phone is taken at every
# pause, not only between moves: while a move is made and walked, while a plate is taken in and
# while an ask is open (`[talk NN] taken waited_ms=.. at=move|dwell|ask|pause`, from the inbox
# line's `at_ms`). Every move ask also offers a card back to each place the hero has been
# (`@roam_player_back_line`, `{place}` its name with a leading article as `the`, carrying its scene
# memory's id); a tapped one, or typed words Jev reads as going back to a kept place
# (`move act` stops at `back_to=<plate>`, `back at place=..`), goes back there without a new
# design, plate or walk:
# the host makes it the active place again (`world place`), and the look and the paint follow as
# usual over its own things (`[revisit NN] .. via=card|jev`).
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
# Played from a phone (the host starts gauntlet beats for a seat as it starts the painter's day,
# `DF_RUN_PLAYER` and `DF_RUN_HERO_*`): the seat's hero (`hero-<id>`) is the character in place of
# `@gauntlet`, its persona kept or written (`@gauntlet_hero_hint`), its rules sheet the seat's derived
# creation sheet (`creation sheet <seat> out=<file>`) and every station plate its hero version, made
# once per locked look after the hero's reference sheet (`@gauntlet_hero_plate`, `[prep] hero plate
# ..`). At a player station the seat is asked for the line (`players ask`, the station's fifth field
# as the prompt and its sixth as the one suggested card: `@station <id> | player | <scene> | <goal> |
# <ask> | <card>`), spoken as written, and talk is taken between stations. `@gauntlet_subset on`
# says the beats play part of the stations file (`gauntlet-short.beats`), so `--plan` lists the rest.
# A party journey (`@journey <id>`): one or two seats' heroes travel a quest written stop by stop
# (`@stop`, `@dm`, `@ask`, `@talk`, `@check`, `@shop`, `@cast`, `@quest_board`, `@riddle`,
# `@teleport`; see play_journey); `--plan` prints and checks every step and calls no host.
# A copy of the runner (the host starts one for a table's scenario) names the checkout it runs in.
cd "${DF_RUN_ROOT:-$(dirname "$0")/..}"
D=${DF_TABLE_EXE:-./artifacts/build/bubbles/debug/df-table.exe}
P=${DF_TABLE_PORT:-7879}
# How long a model-backed command may take before the run calls it hung: the host bounds each model
# call itself (600 s), and a line's chain is several calls (dungeon run 13: at 60 s a call, a
# reply cut at 280 s came back empty).
HUNG=1800
# The waits of a turn a person at the table sits through, bounded at human pace: a wait past its
# bound is a hang, never a slow turn, and the turn goes on by its fallback, so the phone always
# gets its next ask. Defaults are HUNG; the free roam sets its own (play_explore, EXPLORE_*_WAIT).
TEXT_WAIT=$HUNG
MOVE_WAIT=$HUNG
# A close look's insert: shown 41-72 s after its line on hosts 32-40 (24 inserts, median 51 s),
# so 90 s to be shown; then its end comes after the insert hold (20 s), so 60 s more. Past either
# the look goes on without it (`[insert NN] waited_out`): the dungeon master tells the look.
INSERT_SHOW_WAIT=90
INSERT_END_WAIT=60
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
# What a painting sells for (`@roam_art_price <amount> <cp|sp|ep|gp|pp>`), as the trade card says.
ART_PRICE="5 sp"
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
# The places of a roam the hero has been to, by plate (the host's place id): name, what the plate
# shows, its things, its elements, the kind of move it was reached by and the key its back card
# carries (its scene memory's id, the Map page's place id, else the plate); VISIT_PLATE by key.
declare -A VISIT_NAME VISIT_PICTURE VISIT_THINGS VISIT_ELEMENTS VISIT_WAY VISIT_KEY VISIT_PLATE
# The thing a reached approach stop is the close-up of, by its plate (explore_approach).
declare -A VISIT_TARGET
VISIT_ORDER=()
GAUNTLET=""
GAUNTLET_PERSONA=""
GAUNTLET_SHEET=""
GAUNTLET_STATIONS=""
GAUNTLET_FADE=800
GAUNTLET_BREATH=1.5
GAUNTLET_LIMIT=180
GAUNTLET_LOOK=""
GAUNTLET_PORTRAIT=""
# The gauntlet played from a phone by the seat's hero (see play_gauntlet): the hint its persona is
# written from when it has none, what each station plate is redrawn with so the hero is the one in
# it, and whether the beats play only part of the stations file.
GAUNTLET_HERO_HINT="{name} is a {species} {class}, an adventurer who meets every trial on the road head on."
GAUNTLET_HERO_PLATE="the one adventurer in it is the person of the reference sheet, seen whole, with their own face, build, clothing and gear, doing what the scene asks."
GAUNTLET_SUBSET=""
HERO_LOOKS=""
declare -A HERO_PLATE
STATIONS=()
WALK_HOLD=
WALK_SECONDS=0
ROAM_PLAYER=""
# The fixture a hero's day starts the TV from: no caption, card, party, title or roleplay lines.
CLEAN_FIXTURE=painters_day
PLAYER_ASK_MOVE="Where do you go next?"
PLAYER_ASK_LOOK="What do you step up to?"
PLAYER_ASK_DECLARE="What are your hands doing?"
PLAYER_LOOK_LINE=""
PLAYER_DECLARE_LINE=""
SAID_LINE=""
SAID_TO=""
SAID_THING=""
PLAYER_ASK_PAINTED="What do you say as you finish the painting?"
PLAYER_ASK_TRADE="What do you offer, and what do you ask for?"
PLAYER_ASK_SHOW="What do you say about the day's work?"
PLAYER_PAINTED_LINE=""
PLAYER_TRADE_LINE=""
PLAYER_SHOW_LINE=""
# The cards of a sight (what the hero saw where the place has no things, `{thing}` its words) and
# the move cards back to a place the hero has been (`{place}` its name).
PLAYER_SIGHT_LINE="{thing} | I go closer and take a long look: {thing}."
PLAYER_DECLARE_SIGHT_LINE="Paint it | I set a board on the easel and start laying in what I see: {thing}."
PLAYER_PAINTED_SIGHT_LINE="Finish it | The study is done. I've used up the board, a little ochre and umber, some oil and a rag."
PLAYER_BACK_LINE="Back to {place} | I head back to {place}."
# Where the talk taken was (`[talk NN] .. at=`).
TALK_AT=ask
ROAM_HERO_HINT=""
ROAM_START_PLATE=""
# Explore (`@explore on`, see play_explore): its ask, the cards it builds from the scene and what
# it tells the phone; the scene, the approach under way and the step made ahead.
EXPLORE=""
EXPLORE_ASK="What do you do?"
# The name of the place the opening plate shows (it has no design): `@roam_start_place`.
ROAM_START_PLACE="where the road begins"
EXPLORE_CARD_TALK="Talk to {thing} | Good day to you. Have you a moment to talk?"
EXPLORE_CARD_TRADE="See the wares of {thing} | What have you got for sale, and what do you want for it?"
EXPLORE_CARD_LOOK="Look at the {thing} | I step up and take a close look at the {thing}."
EXPLORE_CARD_WALK="Keep walking toward the {thing} | I keep walking toward the {thing}."
EXPLORE_CARD_REACH="Go up to the {thing} | I go right up to the {thing}."
EXPLORE_CARD_AWAY="Keep backing away from the {thing} | I keep backing away from the {thing}."
# How the approach under way goes (explore_approach): its pace, and whether it backs away.
APPROACH_PACE=walk
APPROACH_AWAY=false
APPROACH_NEXT_REACHES=""
EXPLORE_UNREACHED="The table didn't hear back. Try again."
# A line the judge could not be reached for, offered back as a card to send again.
EXPLORE_RETRY=""
# How much of each earlier line of the talk the judge is passed (the command line's size).
EXPLORE_RECENT_CHARS=600
# The longest line a phone's words are passed at once (an ask's answer, a Talk-page line); a
# longer one is asked for again in fewer words.
SAID_LINE_CHARS=4000
SAID_TOO_LONG="That's a lot to take in at once. Say it in fewer words."
# How many steps an act on what the line walks toward may take to reach it first.
EXPLORE_REACH_STEPS=3
# The free roam's waits at human pace (play_explore).
EXPLORE_TEXT_WAIT=240
EXPLORE_STEP_WAIT=300
EXPLORE_MOVE_WAIT=600
# A pose plate (a held stance, checked once and painted again at most once when the first was
# quick: the host's POSE_REPAINT_WITHIN, 110 s) is bounded like an insert: past it the turn goes on
# from the plate on screen.
EXPLORE_POSE_WAIT=240
# A required roll that did not resolve: the deed waits for it, offered back as a card.
EXPLORE_UNROLLED="The roll didn't go through. Try again."
EXPLORE_SHEET=""
EXPLORE_SEEN=""
# Whether this turn's search could not look at the picture at all (explore_find).
EXPLORE_BLIND=""
EXPLORE_ATTENTION=""
EXPLORE_ADDED=""
EXPLORE_SIGHTS=""
# Whether the turn's line was said already (explore_say), and its follow-on steps.
EXPLORE_SAID=""
# The follow-on step of the turn being carried out (empty for its first), for plate names.
EXPLORE_STEP_AT=""
# The turn's reading (`persona intent`: `step=<n> <field>=<value>` lines, step 0 the first intent,
# then each follow-on); intent_load puts one step's fields in INTENT_*.
INTENT_OUT=""
INTENT_FIELDS="kind target way revisit pace away affects enter depends reads into words sets
  renamed pose spoken direction toward distance distance_from continue named noun named_id
  addressees"
for field in $INTENT_FIELDS; do printf -v "INTENT_${field^^}" '%s' ""; done
# The scene of the place the hero is at: its plate, name, what the plate shows, its thing lines
# (`thing <id> (<name>): <look>`) and each id's distance as the design gave it.
HERE_PLATE=""
HERE_NAME=""
HERE_PICTURE=""
HERE_THINGS=""
declare -A DIST
# The approach under way (its target id or direction key, words, step and distance now) and the
# step made ahead for it (the job's pid, its output file and its plate).
APPROACH_TARGET=""
APPROACH_TOWARD=""
APPROACH_STEP=0
APPROACH_DISTANCE=""
# `direction` for a walk a way (a direction or a route the player named), so it keeps its words.
APPROACH_DIRECTED=""
APPROACH_RUN=0
AHEAD_PID=""
AHEAD_FILE=""
AHEAD_PLATE=""
# The cards of the scene the last ask offered: `<id>\t<name>` per thing and per person, sellers.
SCENE_THINGS=""
SCENE_PEOPLE=""
SCENE_SELLERS=""

declare -A PLAYER_CARDS
# A party journey (play_journey): its steps in file order (`<kind> <text>`), each step's cards,
# the quests and clues it names, and each seat's hero.
JOURNEY=""
JOURNEY_STORY=""
JOURNEY_HERO_HINT="{name} is a {species} {class}, an adventurer who travels with the party."
JOURNEY_START_PLATE=""
BOARD_SECONDS=40
RIDDLE_CLOSE_NOTE="The door stirs at that, then stills. Close, but not it."
RIDDLE_WRONG_NOTE="The door stays shut."
# How long an ask, and a shop for every seat together, may wait for an answer.
JOURNEY_ASK_SECONDS=600
JOURNEY_SHOP_SECONDS=900
JOURNEY_WHO=""
# The one to talk to at each stop (`@npc <stop> <id> | <who, as the place shows them>`): a
# resident of the stop's design with that id, so the same persona is met there every run.
declare -A NPC_ID NPC_WHO
# What each NPC knows as anyone of their place does (`@npc_knows <id> | <fact>`, repeated): the
# scenario's public facts, told to them whenever they answer (never a secret or the riddle's answer).
declare -A NPC_KNOWS
# The name each character of a journey plate is shown by (a hero's name, a resident's from the
# design), by id: `media locate identify` names regions by id alone, so the runner gives each its
# shown name after (journey_shown_names), and no line is ever labelled with a raw id.
declare -A SHOWN_NAME
# How many people a scan of a journey plate has room for (`media locate masks=`, at most 32).
JOURNEY_MASKS=12
# How many times a party move that did not go live is asked again before the run goes on.
JOURNEY_MOVE_TRIES=3
# How many refused payments a barred way takes before the run goes on, logging it as failed.
JOURNEY_GATE_TRIES=3
# What the phones are told when the words of a move ask are not read as going somewhere.
JOURNEY_NOT_A_MOVE="That's not somewhere to go. Pick a way, or say where you head."
# Whether a line is shown as a caption now (said_caption), to take down once the plate is read.
CAPTION_UP=""
STEPS=()
STOP_TOTAL=0
STOP_NUMBER=0
STOP=""
STEP=0
PARSE_STOP=""
CLUE_STOPS=()
QUEST_TAKEN=""
REGION=""
CARD_UP=""
declare -A STEP_CARDS STEP_PAYS QUEST_SPEC CLUE_AT CLUE_SEEN
# The seats the run is played from (journey_seats; a roam's or gauntlet's one seat) and each seat's
# hero (HERO_OF): the seat helpers (seat_ask .. seat_reply) read them.
SEATS=()
declare -A HERO_OF HERO_ID_OF HERO_NAME_OF HERO_SPECIES_OF HERO_CLASS_OF HERO_SHEET_OF
declare -A HERO_LOOK_OF HERO_LOOK_KEY_OF RULES_SHEET_OF
# A field of a `|`-separated directive, without the spaces around it (fields from 1).
journey_field() { # text, number
  printf '%s' "$1" | awk -F'|' -v n="$2" '{ v = $n; sub(/^ +/, "", v); sub(/ +$/, "", v); print v }'
}
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
    '@roam_art_price '*) ART_PRICE=$value ;;
    '@roam_show '*) ROAM_SHOW=$value ;;
    '@roam_show_look '*) ROAM_SHOW_LOOK=$value ;;
    '@roam_show_say '*) ROAM_SHOW_SAY=$value ;;
    '@roam_player '*) ROAM_PLAYER=$value ;;
    '@roam_player_ask_move '*) PLAYER_ASK_MOVE=$value ;;
    '@roam_player_ask_look '*) PLAYER_ASK_LOOK=$value ;;
    '@roam_player_ask_declare '*) PLAYER_ASK_DECLARE=$value ;;
    '@roam_player_card '*) PLAYER_CARDS[${value%% *}]+="${value#* }"$'\n' ;;
    '@roam_player_look_line '*) PLAYER_LOOK_LINE=$value ;;
    '@roam_player_declare_line '*) PLAYER_DECLARE_LINE=$value ;;
    '@roam_player_ask_painted '*) PLAYER_ASK_PAINTED=$value ;;
    '@roam_player_ask_trade '*) PLAYER_ASK_TRADE=$value ;;
    '@roam_player_ask_show '*) PLAYER_ASK_SHOW=$value ;;
    '@roam_player_painted_line '*) PLAYER_PAINTED_LINE=$value ;;
    '@roam_player_trade_line '*) PLAYER_TRADE_LINE=$value ;;
    '@roam_player_show_line '*) PLAYER_SHOW_LINE=$value ;;
    '@roam_player_sight_line '*) PLAYER_SIGHT_LINE=$value ;;
    '@roam_player_declare_sight_line '*) PLAYER_DECLARE_SIGHT_LINE=$value ;;
    '@roam_player_painted_sight_line '*) PLAYER_PAINTED_SIGHT_LINE=$value ;;
    '@roam_player_back_line '*) PLAYER_BACK_LINE=$value ;;
    '@roam_hero_hint '*) ROAM_HERO_HINT=$value ;;
    '@roam_start_plate '*) ROAM_START_PLATE=$value ;;
    '@explore '*) EXPLORE=$value ;;
    '@explore_ask '*) EXPLORE_ASK=$value ;;
    '@roam_start_place '*) ROAM_START_PLACE=$value ;;
    '@explore_card_talk '*) EXPLORE_CARD_TALK=$value ;;
    '@explore_card_trade '*) EXPLORE_CARD_TRADE=$value ;;
    '@explore_card_look '*) EXPLORE_CARD_LOOK=$value ;;
    '@explore_card_walk '*) EXPLORE_CARD_WALK=$value ;;
    '@explore_card_reach '*) EXPLORE_CARD_REACH=$value ;;
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
    '@gauntlet_hero_hint '*) GAUNTLET_HERO_HINT=$value ;;
    '@gauntlet_hero_plate '*) GAUNTLET_HERO_PLATE=$value ;;
    '@gauntlet_subset '*) GAUNTLET_SUBSET=$value ;;
    '@station '*) STATIONS+=("$value") ;;
    '@station_sight '*) STATION_SIGHT[${value%% *}]+="${value#* }"$'\n' ;;
    '@station_inspire '*) STATION_INSPIRE[${value%% *}]=${value#* } ;;
    '@station_plate '*) STATION_PLATE[${value%% *}]=${value#* } ;;
    '@station_place '*) STATION_PLACE[${value%% *}]=${value#* } ;;
    '@journey '*) JOURNEY=$value ;;
    '@journey_story '*) JOURNEY_STORY=$value ;;
    '@journey_hero_hint '*) JOURNEY_HERO_HINT=$value ;;
    '@journey_start_plate '*) JOURNEY_START_PLATE=$value ;;
    '@journey_board_seconds '*) BOARD_SECONDS=$value ;;
    '@journey_ask_seconds '*) JOURNEY_ASK_SECONDS=$value ;;
    '@npc_knows '*) NPC_KNOWS[$(journey_field "$value" 1)]+="$(journey_field "$value" 2) " ;;
    '@npc '*) NPC_ID[${value%% *}]=$(journey_field "${value#* }" 1)
      NPC_WHO[${value%% *}]=$(journey_field "${value#* }" 2) ;;
    '@journey_shop_seconds '*) JOURNEY_SHOP_SECONDS=$value ;;
    '@riddle_close_note '*) RIDDLE_CLOSE_NOTE=$value ;;
    '@riddle_wrong_note '*) RIDDLE_WRONG_NOTE=$value ;;
    '@quest '*) QUEST_SPEC[$(journey_field "$value" 1)]=${value#*|} ;;
    '@clue '*) read -r clue_stop clue_id <<< "$value"
      CLUE_AT[$clue_id]=$clue_stop; CLUE_STOPS+=("$clue_stop") ;;
    '@card '*) STEP_CARDS[$((${#STEPS[@]} - 1))]+="$value"$'\n' ;;
    '@pay '*) STEP_PAYS[$((${#STEPS[@]} - 1))]+="$value"$'\n' ;;
    '@stop '*) STEPS+=("stop $value"); STOP_TOTAL=$((STOP_TOTAL + 1))
      PARSE_STOP=$(journey_field "$value" 1) ;;
    '@dm '*) STEPS+=("dm $value") ;;
    '@ask '*) STEPS+=("ask $value") ;;
    '@talk '*) STEPS+=("talk $value") ;;
    '@check '*) STEPS+=("check $value") ;;
    '@fare '*) STEPS+=("fare $value") ;;
    '@shop '*) STEPS+=("shop $value") ;;
    '@cast '*) STEPS+=("cast $value") ;;
    '@quest_board '*) STEPS+=("board $value") ;;
    # A riddle's door is read by the clue judge too, so a stuck party is counted there.
    '@riddle '*) STEPS+=("riddle $value"); CLUE_STOPS+=("$PARSE_STOP") ;;
    '@teleport '*) STEPS+=("teleport $value") ;;
  esac
done < <(grep '^@' "$BEATS")
ROAM_PLAYER=${DF_RUN_PLAYER:-$ROAM_PLAYER}
# The host names the seat's hero as the roam's character for every scenario; gauntlet beats play
# the gauntlet, with the hero as its character (play_gauntlet), never a roam.
[ -n "$GAUNTLET$JOURNEY" ] || ROAM=${DF_RUN_ROAM:-$ROAM}
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
# Every name a resident was given, kept across runs and hosts, so new people get new names.
export DF_NAMES_GIVEN=${DF_NAMES_GIVEN:-artifacts/names-given.txt}
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

# A line whose speaker chose the thing they look at closely ($2, a tapped card's id): the frame
# judge may find the line does not look at it, but never names another thing (`@look`).
watch_look() { # speaker, thing id, line
  [ -n "$WATCH_PID" ] && [ -n "$3" ] && printf '@look %s | %s | %s\n' "$2" "$1" "$3" >&4
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
if [ -z "$FLOW" ] && [ -z "$ROAM" ] && [ -z "$GAUNTLET" ] && [ -z "$JOURNEY" ] && { [ -z "$KEEPER" ] || [ -z "$KEEPER_SITUATION" ] ||
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

# Whom Jev's addressee judge (`persona addressed`, a model call against the scene's speakers) finds
# the line $2 of speaker $1 directed at, with the options after them (`recent=`, `intent=`,
# `reply=`): ADDRESSEE, an id of the scene, empty for nobody, the speaker themself, more than one
# or a seat's hero (a player answers for their own hero); and ANSWER_OWED, false only when the judge
# found that an answer would add nothing the talk needs (a fact, a decision, an action, the answer
# to a question or a demand). A judge that gave no answer leaves the answer owed.
addressee() { # speaker, line, options...
  local judged seat
  judged=$(timeout $TEXT_WAIT $D dev --port $P persona addressed "$1" "${@:3}" "$2" 2>/dev/null)
  ANSWER_OWED=$(printf '%s\n' "$judged" | sed -n 's/^answer_owed=//p')
  ADDRESSEE=$(printf '%s\n' "$judged" | head -1)
  case "$ADDRESSEE" in ''|nobody|"$1"|*' '*|answer_owed=*) ADDRESSEE="" ;; esac
  for seat in "${SEATS[@]}"; do
    [ "$ADDRESSEE" = "${HERO_OF[$seat]}" ] && ADDRESSEE=""
  done
  return 0
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
  local reply=false
  [ -n "$follow" ] && reply=true
  addressee "$LAST_SPEAKER" "$LAST" "${RECENT[@]}" "intent=$SPEAKER_SITUATION $LAST_INTENT" \
    "reply=$reply"
  # When an answer would only acknowledge or echo the line, nobody answers (dungeon runs: a
  # torchbearer said "Yes, sir" to nearly every player line, doubling the lines at each place).
  if [ "$ANSWER_OWED" = false ]; then
    printf '       - no answer owed\n'
    return
  fi
  answerer=$ADDRESSEE
  # A flowing move names whom it is put to (`MOVE_TO`, an id the storyteller chose from who is
  # there): they answer it (run 78: the judge gave answers to the wrong one of two lookalikes).
  if [ -z "$follow" ] && [ -n "$MOVE_TO" ] && [ "$MOVE_TO" != room ]; then
    answerer=$MOVE_TO
  fi
  if [ -z "$answerer" ]; then
    [ -n "$follow" ] && return
    answerer=$KEEPER
  fi
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
# The changes a `persona say` output took of inventories (`<item> <before> -> <after> (<why>)`,
# `; ` between). A change to another owner names it (`[owner <id>]`): the ground of the place
# (`ground-<place>`) is where a thrown or dropped thing now lies, so the teller never contradicts
# the bag.
said_changes() { # output of the command that said it
  printf '%s\n' "$1" | sed -n 's/^ *inventory: //p' | awk '
    match($0, / \[owner [^]]*\]$/) {
      owner = substr($0, RSTART + 8, RLENGTH - 9); $0 = substr($0, 1, RSTART - 1)
      gained = $2 + 0 < $4 + 0
      if (owner ~ /^ground-/) $0 = $0 (gained ? ", now lying on the ground here" : ", picked up from the ground here")
      else $0 = $0 (gained ? ", now held by " : ", taken from ") owner
    } { print }' | paste -sd ';' - | sed 's/;/; /g'
}
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
# the `[insert NN]` line. A card thing that is no thing of the host's place (a sight, a thing of a
# place gone back to) is shown by a focus on its words (`focus`, `via=focus` in the line): its
# insert is the focus's, and a line's own judgement is not waited for.
roam_insert() { # after-sequence, frames name, move number, thing lines, card|judge, [card thing], [focus]
  local after=$1 frames=$2 number=$3 things=$4 from=${5:-judge} chosen=${6:-} item="" name=""
  local frame="" shown="" made="" reason=inspection via=""
  local began=$SECONDS deadline=$((SECONDS + INSERT_SHOW_WAIT)) sequence text
  if [ "${7:-}" = focus ]; then
    reason=focus
    via=" via=focus"
  fi
  while [ "$SECONDS" -lt "$deadline" ]; do
    sleep 1
    while read -r sequence _ text; do
      after=$sequence
      case "$text" in
        'PERSONA frame_judged '*'reason=focus'*)
          [ "$reason" = focus ] && [ -z "$item" ] || continue
          item=$chosen
          name=$(printf '%s\n' "$things" | sed -n "s/^thing $item (\([^)]*\)).*/\1/p" | head -1) ;;
        'PERSONA frame_judged '*'reason=line'*)
          { [ -n "$item" ] || [ "$reason" = focus ]; } && continue
          # A tapped card's judgement is the one that names its thing; another line's is not it.
          if [ -n "$chosen" ] && [ "$(field_of "$text" chosen)" != "$chosen" ]; then
            continue
          fi
          item=$(field_of "$text" inspected)
          name=$(printf '%s\n' "$things" | sed -n "s/^thing $item (\([^)]*\)).*/\1/p" | head -1)
          if [ "$(field_of "$text" inspection)" != true ]; then
            printf '[insert %02d] none: the judge saw no new close look (inspected=%s from=%s)\n' \
              "$number" "$item" "$from"
            return
          fi ;;
        'PERSONA frame_timing '*"frame=$frames-"*"reason=$reason"*)
          frame=$(field_of "$text" frame)
          # An insert the card asked for is of its thing, when no judgement named it first.
          if [ -z "$item" ] && [ -n "$chosen" ]; then
            item=$chosen
            name=$(printf '%s\n' "$things" | sed -n "s/^thing $item (\([^)]*\)).*/\1/p" | head -1)
          fi
          case "$(field_of "$text" moment)" in
            shown)
              shown=$(field_of "$text" at_ms)
              made=$(field_of "$text" took_ms)
              deadline=$((SECONDS + INSERT_END_WAIT)) ;;
            ended)
              printf '[insert %02d] item=%s (%s) from=%s%s frame=%s shown_at_ms=%s made_ms=%s ended_at_ms=%s up_ms=%s\n' \
                "$number" "$item" "${name:-?}" "$from" "$via" "$frame" "${shown:--}" \
                "${made:--}" "$(field_of "$text" at_ms)" "$(field_of "$text" took_ms)"
              return ;;
            failed)
              printf '[insert %02d] item=%s (%s) from=%s%s frame=%s failed after %s ms\n' \
                "$number" "$item" "${name:-?}" "$from" "$via" "$frame" \
                "$(field_of "$text" took_ms)"
              return ;;
          esac ;;
        'PERSONA refused verb=frame '*)
          printf '[insert %02d] none: the frame judge refused (%s) from=%s%s\n' "$number" \
            "$(field_of "$text" error)" "$from" "$via"
          return ;;
      esac
    done < <($D dev --port $P events "$after" area=PERSONA 2>/dev/null | grep -E '^ *[0-9]+ ')
  done
  printf '[insert %02d] waited_out ms=%s item=%s shown=%s from=%s%s\n' "$number" \
    "$(((SECONDS - began) * 1000))" "${item:-${chosen:--}}" "$([ -n "$shown" ] && echo yes ||
      echo no)" "$from" "$via"
}

# Free roam memories: the plate that went live, kept as a memory of the place (kind `@roam_scene_kind`).
# SCENE_MEMORY is the memory's id, or nothing.
roam_remember_scene() { # move number, plate, place name, what the plate shows
  SCENE_MEMORY=""
  [ -z "$SCENE_KIND" ] && return
  local kept
  kept=$($D dev --port $P world memory capture "$ROAM" "$SCENE_KIND" "$2.png" "${3:-$2}" \
    "place=$2" "description=${4:-${3:-$2}}" by=runner 2>&1 | head -1)
  case "$kept" in
    'ok memory='*)
      SCENE_MEMORY=$(field_of "$kept" memory)
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
  local number=$1 id file prompt title request try seen made stored done_line done_out now given=()
  if [ -n "$ROAM_PAINTED" ]; then
    if [ -n "$ROAM_PLAYER" ]; then
      seat_line "$number" "$ROAM_PLAYER" "$PLAYER_ASK_PAINTED" "" \
        "$(player_cards "$(sight_or "$PLAYER_PAINTED_SIGHT_LINE" "$PLAYER_PAINTED_LINE")" "$3")"
      given=("text=$SAID_LINE")
    fi
    # The finishing line is what uses the supplies up: its own inventory judgement reads it
    # (`expect=inventory`; without it about one finishing line in six used nothing up).
    done_out=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true expect=inventory \
      ${SAID_TO:+"from=$SAID_TO"} \
      "recent=$ROAM: $7" "intent=$ROAM_PAINTED" "${given[@]}" "You are at $6.$MIND" 2>&1)
    done_line=$(printf '%s\n' "$done_out" | sed -n '1s/.*says: //p')
    printf '[painted %02d] %s\n' "$number" "${done_line:-<no line: $done_out>}"
    roam_inventory "$done_line" "$done_out"
    seat_reply "$number" "$SAID_TO" "$ROAM" "$done_line"
  fi
  now=$(item_count "$ART_BOARD")
  if [ -n "$ART_BOARD" ] && [ "${now:-0}" -ge "${5:-0}" ]; then
    printf '[memory %02d] not painted: no %s was used up (had %s, have %s)\n' "$number" \
      "$ART_BOARD" "${5:-0}" "$now"
    return
  fi
  id=$(next_art_id)
  file="item-$ROAM-$id"
  # The finish reads the id here when the study is abandoned, to remove only its own picture.
  printf '%s\n' "$id" > "$8.id"
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
  # The prompt stays in the run's log; the player sees what the study is of (`subject`, the thing
  # studied, and `detail`, its look).
  printf 'stored=%q id=%q title=%q prompt=%q subject=%q detail=%q\n' "$stored" "$id" "$title" \
    "$prompt" "$3" "$4" > "$8.state"
}
# The next artwork id of the hero, unique for its whole life: one past the highest number any of
# its artwork items, artwork memories or item pictures uses, so a later day never takes an id (and
# never paints over a picture) an earlier one made.
next_art_id() {
  local prefix="${ART_ITEM:-artwork}_" highest=0 used
  for used in $( {
      $D dev --port $P world inventory get "$ROAM" 2>/dev/null
      $D dev --port $P world memory list "$ROAM" "kind=$ART_KIND" limit=1000 2>/dev/null
      $D dev --port $P media 2>/dev/null | grep -F "item-$ROAM-$prefix"
    } | grep -o "$prefix[0-9][0-9]*" | sed "s/^$prefix//"); do
    used=$((10#$used))
    [ "$used" -gt "$highest" ] && highest=$used
  done
  printf '%s%02d' "$prefix" $((highest + 1))
}
# The price of a painting in copper pieces, from ART_PRICE (`<amount> <coin>`, a coin worth in
# copper what the rules say: cp 1, sp 10, ep 50, gp 100, pp 1000); nothing when it is not that.
art_price_cp() {
  local amount=${ART_PRICE%% *} worth
  case "${ART_PRICE#* }" in
    cp) worth=1 ;;
    sp) worth=10 ;;
    ep) worth=50 ;;
    gp) worth=100 ;;
    pp) worth=1000 ;;
    *) return 0 ;;
  esac
  [[ "$amount" =~ ^[0-9]+$ ]] && [ "$amount" -gt 0 ] && printf '%s\n' $((10#$amount * worth))
}
# Before the first move: the price paintings are made with (ART_PRICE_CP; ART_PRICE_ON_ADD when this
# host's `world inventory add` takes `price=`), and once for the paintings the hero already holds
# that were made with none (`world inventory price`, which prices an item only where it has no
# price), one `[art] ..` line each. A host without these verbs leaves paintings unpriced, and the
# log says so.
roam_art_prices() {
  [ -n "$ART_KIND" ] || return 0
  local usage id
  ART_PRICE_CP=$(art_price_cp)
  ART_PRICE_ON_ADD=""
  if [ -z "$ART_PRICE_CP" ]; then
    printf '[art] no price: @roam_art_price %s is not <amount> <cp|sp|ep|gp|pp>\n' "$ART_PRICE"
    return 0
  fi
  usage=$($D dev --port $P world inventory help 2>&1)
  case "$usage" in
    *'price='*) ART_PRICE_ON_ADD=1
      printf '[art] paintings are made at %s (price_cp=%s)\n' "$ART_PRICE" "$ART_PRICE_CP" ;;
    *) printf '[art] paintings are made unpriced: this host'"'"'s world inventory add takes no price=\n' ;;
  esac
  case "$usage" in
    *'world inventory price '*) ;;
    *) printf '[art] held paintings not priced: this host has no world inventory price\n'
      return 0 ;;
  esac
  for id in $($D dev --port $P world inventory get "$ROAM" 2>/dev/null |
    sed -n "s/^  \(${ART_ITEM:-artwork}_[0-9][0-9]*\) x[1-9][0-9]*: .*/\1/p"); do
    printf '[art] price %s: %s\n' "$id" "$($D dev --port $P world inventory price "$ROAM" "$id" \
      "$ART_PRICE_CP" "why=a painting made before paintings had a price" 2>&1 | head -1)"
  done
}
# Starts the study of move $1 in the background (see roam_study_job).
roam_study_start() { # move number, plate, thing name, thing look, board count before, place name, line
  [ -z "$ART_KIND" ] && return
  local folder
  folder=$(dirname "$FRAMES_LOG")
  mkdir -p "$folder"
  STUDY_BASE="$folder/study-$2"
  rm -f "$STUDY_BASE.out" "$STUDY_BASE.state" "$STUDY_BASE.done" "$STUDY_BASE.id"
  STUDY_STARTED=$SECONDS
  # The job does not hold the frame watch's input open.
  ( exec 4>&-
    PLAYER_LOG_FD=5
    roam_study_job "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$STUDY_BASE"
    : > "$STUDY_BASE.done" ) 5>&1 > "$STUDY_BASE.out" 2>&1 &
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
  local subject="" detail=""
  local kept
  while [ ! -e "$STUDY_BASE.done" ] && [ "$SECONDS" -lt "$deadline" ]; do
    sleep 1
  done
  if [ ! -e "$STUDY_BASE.done" ]; then
    kill "$STUDY_PID" 2>/dev/null
    cat "$STUDY_BASE.out" 2>/dev/null
    printf '[memory %02d] study abandoned after %s s\n' "$number" "$((SECONDS - STUDY_STARTED))"
    # Only the picture this study was making is removed, never one an earlier study kept.
    if [ -s "$STUDY_BASE.id" ]; then
      $D dev --port $P media remove "item-$ROAM-$(cat "$STUDY_BASE.id").png" >/dev/null 2>&1
    fi
    rm -f "$STUDY_BASE.out" "$STUDY_BASE.state" "$STUDY_BASE.done" "$STUDY_BASE.id"
    STUDY_PID=""
    return
  fi
  cat "$STUDY_BASE.out"
  # shellcheck disable=SC1090
  [ -e "$STUDY_BASE.state" ] && . "$STUDY_BASE.state"
  rm -f "$STUDY_BASE.out" "$STUDY_BASE.state" "$STUDY_BASE.done" "$STUDY_BASE.id"
  STUDY_PID=""
  if [ -z "$stored" ]; then
    TIME_STUDY=$((SECONDS - began))
    return
  fi
  kept=$($D dev --port $P world memory capture "$ROAM" "$ART_KIND" "$stored" "$title" "id=$id" \
    "place=$3" "description=A study of ${subject:-$title}.${detail:+ $detail}" \
    "link=item:$id" "link=place:$3" \
    by=runner 2>&1 | head -1)
  case "$kept" in
    'ok memory='*)
      printf '[memory %02d] painted %s seq=%s picture=%s after=%s plate=%s (%s)\n' "$number" \
        "$(field_of "$kept" memory)" "$(field_of "$kept" seq)" "$stored" "-" "$2.png" "$title"
      roam_tempo "$number" painted ;;
    *) printf '[memory %02d] painted %s but not kept as a memory: %s\n' "$number" "$stored" "$kept"
      kept="" ;;
  esac
  # The painting is made with its price (`@roam_art_price`), so it can be sold from the bag.
  $D dev --port $P world inventory add "$ROAM" "$id" 1 "name=$title" \
    "look=A small oil-on-board study of ${subject:-$title}.${detail:+ $detail}" use=kept \
    ${ART_PRICE_ON_ADD:+"price=$ART_PRICE_CP"} why=made 2>&1 | head -1 |
    sed "s/^/       ~ item $id${ART_PRICE_ON_ADD:+ price_cp=$ART_PRICE_CP}: /"
  TIME_STUDY=$((SECONDS - began))
  if [ -n "$kept" ]; then
    began=$SECONDS
    roam_reveal "$number" "$stored" "$title" "$2"
    TIME_REVEAL=$((SECONDS - began))
  fi
}
# Free roam: the day's work shown. The artworks the hero still holds are composed into one gallery
# sheet; the show plate is the move's plate edited after that sheet and the traveller's own sheet,
# put behind the scene with the traveller located in it, and the traveller says a line. When no
# show plate can be made, that is logged and the line is still asked over the square's plate.
roam_show() { # move number, plate, place name, line said when moving
  [ -z "$ART_KIND" ] && return
  local line began=$SECONDS held
  SHOW_PLATE=""
  roam_show_plate "$2"
  if [ -n "$SHOW_PLATE" ]; then
    # The square has been on screen all this while; it stays up at least for the show's dwell.
    held=$((SECONDS - began))
    [ "$held" -lt "$ROAM_SHOW_DWELL" ] && seat_dwell "$1" dwell $((ROAM_SHOW_DWELL - held))
    printf '[dwell %02d] the town square was up %s s before the show plate (dwell %s s)\n' "$1" \
      "$((SECONDS - began))" "$ROAM_SHOW_DWELL"
    $D dev --port $P scene image tv-backdrop "$SHOW_PLATE" plate=show 2>&1 | head -1 |
      sed 's/^/       ~ /'
    start_watch "${FRAMES_LOG%.log}-$2-show.log" "frames=$2-show" "scene=$SHOW_LOOK" \
      ${WATCH_REFS:+"$WATCH_REFS"} "${FRAMES[@]}"
    seat_dwell "$1" dwell "$ROAM_DWELL"
    roam_show_pieces
    stop_watch quiet
  else
    printf '[show] no show plate: the show line is asked over the square plate\n'
  fi
  local given=()
  if [ -n "$ROAM_PLAYER" ]; then
    seat_line "$1" "$ROAM_PLAYER" "$PLAYER_ASK_SHOW" "" "$(player_cards "$PLAYER_SHOW_LINE" "$3")"
    given=("text=$SAID_LINE")
  fi
  line=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true \
    ${SAID_TO:+"from=$SAID_TO"} "recent=$ROAM: $4" \
    "intent=$ROAM_SHOW_SAY" "${given[@]}" "You are at $3." 2>&1)
  printf '[show] %s\n' "$(printf '%s\n' "$line" | sed -n '1s/.*says: //p')"
  roam_inventory "$(printf '%s\n' "$line" | sed -n '1s/.*says: //p')" "$line"
  seat_reply "$1" "$SAID_TO" "$ROAM" "$(printf '%s\n' "$line" | sed -n '1s/.*says: //p')"
}

# The show plate of move plate $1, set as SHOW_PLATE (and its look as SHOW_LOOK), located and the
# traveller named in it; SHOW_PLATE stays empty, with a `[show]` line saying why, when it cannot
# be made. The gallery is the artworks the hero still holds (one sold on an earlier day is not
# shown), selected by id: each artwork's memory has its item's id.
roam_show_plate() { # move plate
  local composed gallery count refs shown made scan region ids try counted pieces sheet
  ids=$($D dev --port $P world inventory get "$ROAM" 2>/dev/null |
    sed -n "s/^  \(${ART_ITEM:-artwork}_[0-9][0-9]*\) x[1-9][0-9]*: .*/id=\1/p")
  if [ -z "$ids" ]; then
    printf '[show] no gallery: %s holds no artwork\n' "$ROAM"
    return
  fi
  # shellcheck disable=SC2086
  composed=$($D dev --port $P world memory compose "$ROAM" $ids 2>&1 | head -1)
  gallery=$(field_of "$composed" picture)
  if [ -z "$gallery" ]; then
    printf '[show] no gallery: %s\n' "$composed"
    return
  fi
  count=$(field_of "$composed" members)
  printf '[show] composed gallery of %s pieces -> %s\n' "$count" "$gallery"
  # The plate shows exactly the pieces on the gallery sheet, no more and no fewer.
  SHOW_LOOK="$ROAM_SHOW_LOOK There are exactly $count paintings set out, the $count on the \
reference sheet, no more and no fewer."
  # The gallery sheet carries every piece; the traveller's own sheet and the base come with it,
  # within `@refs max=`.
  refs=$gallery
  sheet=${DF_RUN_HERO_SHEET:-ref-$ROAM.png}
  if library_holds "$sheet" && [ "${REFS_MAX:-8}" -ge 3 ]; then
    refs="$refs,$sheet"
  fi
  # The frame judge counts the paintings the plate shows; a plate that shows another number is
  # made once more, and the second one stands whatever it shows.
  for try in 1 2; do
    made=$(timeout $HUNG $D dev --port $P media generate "$SHOW_LOOK" "as=$1-show" \
      purpose=plate "base=$1.png" "refs=$refs" wait=true look=none 2>&1)
    shown=$(stored_file "$made")
    if [ -z "$shown" ]; then
      printf '[show] plate not made: %s\n' "$(printf '%s\n' "$made" | head -1)"
      return
    fi
    counted=$(timeout $HUNG $D dev --port $P persona frame "line=$ROAM: My day's work." \
      "count=paintings set out" "count_picture=$shown" "$SHOW_LOOK" 2>&1)
    pieces=$(printf '%s\n' "$counted" | sed -n 's/^pieces_shown=//p' | head -1)
    printf '[show] plate %s shows %s paintings, the gallery has %s (try %s)\n' "$shown" \
      "${pieces:-?}" "$count" "$try"
    [ "$pieces" = "$count" ] && break
  done
  printf '[show] plate %s after %s\n' "$shown" "$refs"
  scan=$($D dev --port $P media locate "$shown" masks=1 2>&1)
  printf '%s\n' "$scan" | head -3 | sed 's/^/       ~ /'
  # Regions are numbered from 0: the traveller is the first region the scan reports.
  region=$(printf '%s\n' "$scan" | sed -n 's/^ *\([0-9][0-9]*\): .*/\1/p' | head -1)
  if [ -n "$region" ]; then
    $D dev --port $P media locate label "$shown" "$region" "$ROAM" \
      "${DF_RUN_HERO_NAME:-${ROAM^}}" 2>&1 | head -1 | sed 's/^/       ~ /'
  else
    printf '[show] no person found in %s to name\n' "$shown"
  fi
  SHOW_PLATE=$shown
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
    printf '%s' "$PERCEIVED_SIGHTS" > "$VIEW_STEM.sights"
  ) > "$VIEW_STEM.see" 2>&1 &
  VIEW_PID=$!
}
roam_view_finish() { # move number
  # The dwell and the perception are a pause: talk the player sends is taken as it comes.
  while [ "$SECONDS" -lt $((VIEW_BEGAN + ROAM_DWELL)) ] || kill -0 "$VIEW_PID" 2>/dev/null; do
    seat_dwell "$1" dwell 1
  done
  wait "$VIEW_PID"
  cat "$VIEW_STEM.see"
  MIND=$(cat "$VIEW_STEM.mind" 2>/dev/null)
  PERCEIVED_THINGS=$(cat "$VIEW_STEM.things" 2>/dev/null)
  PERCEIVED_SIGHTS=$(cat "$VIEW_STEM.sights" 2>/dev/null)
  rm -f "$VIEW_STEM.mind" "$VIEW_STEM.things" "$VIEW_STEM.sights" "$VIEW_STEM.see"
  printf '[dwell %02d] the view was up %s s before %s spoke of it (dwell %s s)\n' "$1" \
    "$((SECONDS - VIEW_BEGAN))" "${DF_RUN_HERO_NAME:-the hero}" "$ROAM_DWELL"
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
# has its name), PERCEIVED_SIGHTS to the words of each thing seen there (not themselves or what
# they carry), one per line, and MIND to what the look and paint lines carry in their situation:
# what is on their mind and what they have their eye on.
roam_perceive() { # move number, line said when moving, what the plate shows
  MIND=""
  PERCEIVED_THINGS=""
  PERCEIVED_SIGHTS=""
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
  PERCEIVED_SIGHTS=$(printf '%s\n' "$out" | grep -v ' subject=\(self\|carried\)$' |
    sed -n 's/^see: \(.*\) | region=.*$/\1/p')
  sees=$(printf '%s\n' "$PERCEIVED_SIGHTS" | paste -sd ';' - | sed 's/;/; /g')
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
  local after out line relay given=()
  if [ -n "$ROAM_PLAYER" ]; then
    seat_line "$1" "$ROAM_PLAYER" "$PLAYER_ASK_DECLARE" "" \
      "$(player_cards "$(sight_or "$PLAYER_DECLARE_SIGHT_LINE" "$PLAYER_DECLARE_LINE")" \
        "$STUDY_THING_NAME")"
    given=("text=$SAID_LINE")
  fi
  after=$(latest_event)
  out=$(timeout $HUNG $D dev --port $P persona declare "$ROAM" speak=true \
    ${SAID_TO:+"from=$SAID_TO"} "recent=$ROAM: $2" \
    "${given[@]}" "$ROAM_DECLARE You are at $3.$MIND" 2>&1)
  line=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  printf '[declare %02d] %s\n' "$1" "${line:-<no line: $(printf '%s\n' "$out" | head -1)>}"
  roam_inventory "$line" "$out"
  seat_reply "$1" "$SAID_TO" "$ROAM" "$line"
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
  roam_declared_plate "$after" "$LOOK_FRAMES" "$1"
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
  local before after out line gained given=()
  before=$(held_counts)
  roam_tempo "$1" trade
  if [ -n "$ROAM_PLAYER" ]; then
    seat_line "$1" "$ROAM_PLAYER" "$PLAYER_ASK_TRADE" "" \
      "$(player_cards "$PLAYER_TRADE_LINE" "$3")"
    given=("text=$SAID_LINE")
  fi
  # A trade line said to a keeper deals with that keeper (`from=`, fixed by the host).
  out=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true "recent=$ROAM: $2" \
    ${SAID_TO:+"from=$SAID_TO"} "intent=$4" "${given[@]}" "You are at $3.$MIND" 2>&1)
  line=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  printf '[trade %02d] %s\n' "$1" "${line:-<no line: $(printf '%s\n' "$out" | head -1)>}"
  roam_inventory "$line" "$out"
  seat_reply "$1" "$SAID_TO" "$ROAM" "$line"
  after=$(held_counts)
  gained=$(held_gains "$before" "$after")
  printf '[trade %02d] changes: %s; gained: %s\n' "$1" "$(held_diff "$before" "$after")" \
    "${gained:-no gain recorded}"
}

# Every `players` call of a run goes through `players_call`, in this shell (never in `$(..)`, so
# its exit ends the runner): the output is in PLAYERS_OUT and the status is the command's. The host
# gives the runner its run (`DF_RUN_TOKEN`, sent by the dev CLI on `players ask|note|inbox`, never
# unset here) and its process id (`DF_HOST_PID`). A runner that is not the table's current run is
# refused `not_this_run`: it says so (`[stale] this runner is not the table's current run`) and
# exits 4 at once. A runner whose host is gone (its process no longer listed, or five calls in a row
# finding nothing serving on the port) says `[stale] host gone` and exits 5.
PLAYERS_OUT=""
REFUSED_IN_ROW=0
HOST_CHECKED_AT=-10
players_call() { # players words...
  local status
  host_check
  PLAYERS_OUT=$($D dev --port $P players "$@" 2>&1)
  status=$?
  case "$PLAYERS_OUT" in
    *not_this_run*)
      printf "[stale] this runner is not the table's current run\n"
      exit 4 ;;
    *'nothing is serving on 127.0.0.1:'*)
      REFUSED_IN_ROW=$((REFUSED_IN_ROW + 1))
      if [ "$REFUSED_IN_ROW" -ge 5 ]; then
        printf '[stale] host gone (nothing serving on port %s, %s calls in a row)\n' "$P" \
          "$REFUSED_IN_ROW"
        exit 5
      fi ;;
    *) REFUSED_IN_ROW=0 ;;
  esac
  return "$status"
}
# Whether the host process (`DF_HOST_PID`) still runs, checked at most every 10 s: Windows process
# ids are listed by `tasklist` (Git Bash's `kill -0` does not see them), other systems by `kill -0`.
host_check() {
  [ -n "${DF_HOST_PID:-}" ] || return 0
  [ $((SECONDS - HOST_CHECKED_AT)) -ge 10 ] || return 0
  HOST_CHECKED_AT=$SECONDS
  if command -v tasklist >/dev/null 2>&1; then
    tasklist //FI "PID eq $DF_HOST_PID" //NH 2>/dev/null | grep -qw "$DF_HOST_PID" && return 0
  elif kill -0 "$DF_HOST_PID" 2>/dev/null; then
    return 0
  fi
  printf '[stale] host gone (process %s is not running)\n' "$DF_HOST_PID"
  exit 5
}

# The seats a run is played from (SEATS: a party journey's, or the one seat of a roam or a gauntlet
# played from a phone) and each seat's hero (HERO_OF): one client of the seat protocol for every
# flow. Each helper takes the step or move number its lines are logged under (`[ask NN]`,
# `[talk NN]`, `[said NN]`, `[answer NN]`, `[cast NN]`) and the seat it is put to: a seat number,
# `party` (the first answer to a party ask closes it on every phone) or `each`. In a background job
# (the painted study) the ask, the answer and the talk taken go to the run's log at once
# (`PLAYER_LOG_FD`), not when the job's output is printed.

# Asks `$2` the prompt $3 with the note $4 shown above it and the cards $5 (`<label> | <line> [|
# <thing>]` lines) as kind $6 (line, move or look; line when none).
seat_ask() { # number, who, prompt, note, cards, [kind]
  local args=("prompt=$3" "kind=${6:-line}") card label line thing
  [ -n "$4" ] && args+=("note=$4")
  while IFS= read -r card; do
    [ -n "$card" ] || continue
    label=$(journey_field "$card" 1)
    line=$(journey_field "$card" 2)
    thing=$(journey_field "$card" 3)
    args+=("card=$label|$line${thing:+|$thing}")
  done <<< "$5"
  players_call ask "$2" "${args[@]}"
  printf '%s\n' "$PLAYERS_OUT" | sed "s/^/[ask $(printf '%02d' "$1")] /" >&"${PLAYER_LOG_FD:-1}"
}

# Waits for the answer to the ask put to `$2`: sets SAID_SEAT, SAID_LINE (as written), SAID_TO (the
# one of the shown scene the player put it to) and SAID_THING (the thing of the card tapped), each
# checked by the host, and logs `[ask NN] seat <seat> said ..`. Talk and casts that come meanwhile
# are turns of their own (seat_talk, seat_cast) and the ask stays open; with `$3` `cast` a cast
# answers it (CAST_LINE). With no deadline $5 the ask waits until it is answered; at the deadline
# (a `$SECONDS` value) it is closed, the seats are told and the first of the cards $4 is the answer
# (seat_expired); with no card SAID_LINE stays empty and the caller goes on without it.
seat_wait() { # number, who, [cast], [cards], [deadline]
  local said left deadline=${5:-}
  SAID_SEAT="" SAID_LINE="" SAID_TO="" SAID_THING="" CAST_LINE=""
  while [ -z "$SAID_LINE$CAST_LINE" ]; do
    left=60
    if [ -n "$deadline" ]; then
      left=$((deadline - SECONDS))
      if [ "$left" -le 0 ]; then
        seat_expired "$1" "$2" "${4:-}"
        return 0
      fi
      [ "$left" -gt 60 ] && left=60
    fi
    players_call inbox "$2" "wait=$left"
    said=$PLAYERS_OUT
    case "$said" in
      'said '*)
        SAID_LINE=${said#*| }
        SAID_SEAT=$(said_seat "$said" "$2")
        SAID_TO=$(field_of "${said%%|*}" to)
        SAID_THING=$(field_of "${said%%|*}" thing) ;;
      'talk'*) TALK_AT=ask; seat_talk "$1" "$said" "$2" ;;
      'cast'*)
        seat_cast "$1" "$said" "$2"
        if [ "${3:-}" = cast ]; then
          CAST_LINE=$said
          SAID_SEAT=$(said_seat "$said" "$2")
        fi ;;
      none) ;;
      *) printf '[ask %02d] inbox: %s\n' "$1" "$said"; sleep 2 ;;
    esac
  done
  [ -n "$SAID_LINE" ] && printf '[ask %02d] seat %s said%s%s: %s\n' "$1" "$SAID_SEAT" \
    "${SAID_TO:+ to $SAID_TO}" "${SAID_THING:+ card $SAID_THING}" "$SAID_LINE" \
    >&"${PLAYER_LOG_FD:-1}"
  return 0
}

# An ask and its answer (seat_ask, then seat_wait).
seat_line() { # number, who, prompt, note, cards, kind, [deadline]
  seat_ask "$1" "$2" "$3" "$4" "$5" "$6"
  seat_wait "$1" "$2" "" "$5" "${7:-}"
}

# The ask put to `$2` ran out of time: one `[stalled] seat= ask=` line, the ask closed, every seat
# asked told, and the first card of $3 taken as the answer of the first seat asked.
seat_expired() { # number, who, cards
  local first seat seats=("$2")
  case "$2" in party|each) seats=("${SEATS[@]}") ;; esac
  first=$(printf '%s\n' "$3" | grep -m1 '|')
  printf '[stalled] seat=%s ask=%02d default=%s\n' "$2" "$1" \
    "$([ -n "$first" ] && journey_field "$first" 1 || printf none)"
  players_call ask "$2" clear
  for seat in "${seats[@]}"; do
    seat_notice "$1" "$seat" "No answer came in time$([ -n "$first" ] &&
      printf ', so the party goes on: %s' "$(journey_field "$first" 1)")."
  done
  [ -n "$first" ] || return 0
  SAID_SEAT=${seats[0]}
  SAID_LINE=$(journey_field "$first" 2)
  SAID_THING=$(journey_field "$first" 3)
}

# The seat an inbox line came from: its `seat=`, else the one seat it was asked of.
said_seat() { # inbox line, who was asked
  local seat
  seat=$(field_of "${1%%|*}" seat)
  case "$2" in party|each) ;; *) seat=${seat:-$2} ;; esac
  printf '%s' "${seat:-${SEATS[0]}}"
}

# A word for one seat (`players note`), logged as `[ask NN] notice to seat <seat>: <words>`.
seat_notice() { # number, seat, words
  [ -n "$2" ] || return 0
  players_call note "$2" "$3"
  printf '[ask %02d] notice to seat %s: %s (%s)\n' "$1" "$2" "$3" \
    "$(printf '%s\n' "$PLAYERS_OUT" | head -1)"
}

# Every seat's talk and casts that came while no ask of it was open, taken now at pause $2 (move,
# dwell, pause), each as its own turn (seat_talk, seat_cast). An answer here has no ask left to
# answer: it is noted and dropped.
seat_drain_talk() { # number, pause
  local seat said
  for seat in "${SEATS[@]}"; do
    while :; do
      players_call inbox "$seat" wait=0
      said=$PLAYERS_OUT
      case "$said" in
        'talk'*) TALK_AT=$2; seat_talk "$1" "$said" "$seat" ;;
        'cast'*) seat_cast "$1" "$said" "$seat" ;;
        'said '*) printf '[ask %02d] an answer of seat %s came after its ask closed: dropped\n' \
          "$1" "$seat" ;;
        *) break ;;
      esac
    done
  done
}

# R8: talk is taken at every pause of the run, not only between steps: waits $3 seconds at pause $2
# (a move made and walked, a plate taken in), taking every seat's talk as it comes.
seat_dwell() { # number, pause, seconds
  local until=$((SECONDS + $3))
  while [ "$SECONDS" -lt "$until" ]; do
    seat_drain_talk "$1" "$2"
    sleep 1
  done
}

# A talk line from a phone ($2, `talk at_ms= seat= [to=] | <line>`, from the inbox of `$3`): a turn
# of its own, logged as `[talk NN] taken waited_ms=.. at=<pause> seat <seat> (<hero>) ..`, how long
# after it was sent (the host's `at_ms`, Unix ms) it was taken and at which pause. The seat's hero
# says it as written (seat_say) and whom it is put to answers (seat_answer: the phone's `to`, else
# the addressee judge's reading, else the stop's NPC). A line the host refuses is told to the seat,
# never dropped. The ask the runner waits on stays open.
seat_talk() { # number, inbox line, who was asked
  local seat hero line to sent
  seat=$(said_seat "$2" "$3")
  hero=${HERO_OF[$seat]:-}
  line=${2#*| }
  to=$(field_of "${2%%|*}" to)
  sent=$(field_of "${2%%|*}" at_ms)
  printf '[talk %02d] taken waited_ms=%s at=%s seat %s (%s)%s: %s\n' "$1" \
    "$([ -n "$sent" ] && echo $(($(date +%s%3N) - sent)) || echo -)" "$TALK_AT" "$seat" \
    "${hero:-?}" "${to:+ to $to}" "$line" >&"${PLAYER_LOG_FD:-1}"
  if [ "${#line}" -gt "$SAID_LINE_CHARS" ]; then
    printf '[talk %02d] the line is %s characters: asked again in fewer words\n' "$1" "${#line}"
    seat_notice "$1" "$seat" "$SAID_TOO_LONG"
    return 0
  fi
  if [ -z "$hero" ]; then
    printf '[talk %02d] seat %s plays no hero of this run: dropped\n' "$1" "$seat"
    return 0
  fi
  if ! seat_say "$1" "$seat" "$to" "$line" "You talk with who is here."; then
    seat_notice "$1" "$seat" "Your words didn't reach the table. Try again."
    return 0
  fi
  seat_answer "$1" "$seat" "$to" "$HERO_SAID" "${STOP:+${NPC_ID[$STOP]:-}}"
}

# A cast from a phone's Hero page ($2, `cast at_ms= seat= spell= level= ritual= | <Name>: <what it
# does>`, from the inbox of `$3`; the host spent the slot), logged as `[cast NN]`. Where the run's
# story has evidence (`@clue`), the clue judge reads it as the hero's finding at the stop and the
# dungeon master says what it lays bare, in the story's words (no check: the spell is the means);
# without a story the dungeon master says what it does. The ask the runner waits on stays open.
seat_cast() { # number, inbox line, who was asked
  local seat summary out
  seat=$(said_seat "$2" "$3")
  summary=${2#*| }
  printf '[cast %02d] seat %s (%s) spell=%s level=%s ritual=%s | %s\n' "$1" "$seat" \
    "${HERO_OF[$seat]:-?}" "$(field_of "${2%%|*}" spell)" "$(field_of "${2%%|*}" level)" \
    "$(field_of "${2%%|*}" ritual)" "$summary"
  [ "$summary" != "$2" ] && [ -n "$summary" ] || return 0
  if [ "${#CLUE_STOPS[@]}" -eq 0 ]; then
    out=$($D dev --port $P persona narrate speak=true "${summary#*: }" 2>&1) ||
      printf '[cast %02d] <narrate: %s>\n' "$1" "$(printf '%s\n' "$out" | head -1)"
    watch_line narrator "${summary#*: }"
    return 0
  fi
  journey_clues "$STOP" "${HERO_OF[$seat]:-${HERO_OF[${SEATS[0]}]}}" \
    "casts $(printf '%s' "$summary" | sed 's/:.*//')" "spell:${summary#*: }"
  case " ${CLUE_STOPS[*]} " in
    *" $STOP "*) ;;
    *) printf '[cast %02d] no evidence on show at %s: nothing for it to lay bare\n' "$1" "$STOP" ;;
  esac
}

# The seat's hero says the line $4 as written (`persona say text=`, judged for what changes hands),
# put to $3 when the phone named whom (the one a deal in it is with, `from=`, fixed by the host),
# with the situation $5 (and $6 `inventory` when the line is a deed done that uses up what it uses:
# `expect=inventory`, the inventory judge's finished work); sets HERO_SAID. A hero the picture
# names no region for (the protocol's
# `unknown_character` refusal) says it unanchored, as a caption. Returns non-zero when nothing was
# said.
seat_say() { # number, seat, to, line, situation, [inventory], [what came of it]
  local hero=${HERO_OF[$2]} out
  ROAM=$hero
  recent_args
  out=$(timeout $TEXT_WAIT $D dev --port $P persona say "$hero" speak=true ${6:+"expect=$6"} "text=$4" \
    ${3:+"from=$3"} ${3:+"intent=Said to $3."} ${7:+"intent=$7"} "${RECENT[@]}" \
    "${here:+You are at $here. }$5" 2>&1)
  HERO_SAID=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  # What the line moved of what the hero carries, as the host took it, for whoever tells how it
  # turned out.
  SAID_CHANGES=$(said_changes "$out")
  if [ -z "$HERO_SAID" ] && printf '%s\n' "$out" | grep -q '(unknown_character)'; then
    printf '[said %02d] seat %s (%s) unanchored: %s\n' "$1" "$2" "$hero" "$4"
    HERO_SAID=$4
    said_caption "$1" "$hero" "$4"
  else
    printf '[said %02d] seat %s (%s): %s\n' "$1" "$2" "$hero" \
      "${HERO_SAID:-<no line: $(printf '%s\n' "$out" | head -1)>}"
  fi
  [ -n "$HERO_SAID" ] || return 1
  HISTORY+=("$hero: $HERO_SAID")
  roam_inventory "$HERO_SAID" "$out"
  watch_line "$hero" "$HERO_SAID"
  heard_out "$HERO_SAID"
}

# Whom the seat's hero's line $4 was put to answers it (seat_reply): the phone's `to` ($3), else
# whom the addressee judge finds it directed at (addressee), else the NPC $5 (a talk step's line is
# put to the stop's NPC); nobody when no answer is owed and there is no NPC. $6 is what the one
# answering is told of the moment (a roll's result, a payment, the board they are about to show).
seat_answer() { # number, seat, to, line, [NPC], [situation]
  local hero=${HERO_OF[$2]} to=$3
  [ -n "$4" ] || return 0
  if [ -z "$to" ]; then
    addressee "$hero" "$4" "recent=$hero: $4"
    to=${ADDRESSEE:-${5:-}}
    [ -z "${5:-}" ] && [ "$ANSWER_OWED" = false ] && return 0
  fi
  seat_reply "$1" "$to" "$hero" "$4" "${6:-}"
}

# `$2` answers the hero `$3`'s line `$4` in character (`persona ask from=`, judged for what changes
# hands), told what they know of their place (`@npc_knows`) and the moment $5; said like any line:
# on the TV, voiced when TTS is on, on the phones' Story, and read by the clue judge. Nothing when
# nobody (`$2` empty) answers.
seat_reply() { # number, who answers, hero, line, [situation]
  local out line situation=${5:-}
  ANSWERED="" ANSWER_LINE=""
  [ -n "$2" ] && [ -n "$4" ] || return 0
  [ -n "${NPC_KNOWS[$2]:-}" ] &&
    situation="What you know, as anyone here does: ${NPC_KNOWS[$2]}${situation:+ Now: $situation}"
  out=$(timeout $TEXT_WAIT $D dev --port $P persona ask "$2" speak=true "from=$3" "recent=$3: $4" \
    ${situation:+"situation=$situation"} "$4" 2>&1)
  line=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p')
  printf '[answer %02d] %s: %s\n' "$1" "$2" "${line:-<no line: $(printf '%s\n' "$out" | head -1)>}"
  [ -n "$line" ] || return 0
  ANSWERED=$2 ANSWER_LINE=$line
  HISTORY+=("$2: $line")
  ROAM=$3
  roam_inventory "$line" "$out"
  watch_line "$2" "$line"
  heard_out "$line"
  journey_clues "$STOP" "$2" "$line"
}
# The player's own hero as the traveller: its persona (written from the hero, with no scene),
# kit sheets and starting plate, each made once and reused when present (see the header), each step
# one or more `[prepare]` lines. A step that fails prints `[prepare] failed: <step>: <reason>` and
# returns non-zero: the day does not start without the hero.
roam_prepare_hero() {
  # The opening plate is the hero's in their locked look: a new look (`DF_RUN_HERO_LOOK_KEY`)
  # opens on a new plate.
  local plate="roam-00-$ROAM${DF_RUN_HERO_LOOK_KEY:+-$DF_RUN_HERO_LOOK_KEY}" out region scan
  local sheet=${DF_RUN_HERO_SHEET:-}
  [ -n "${DF_RUN_HERO_ID:-}" ] || return 0
  printf '[prepare] hero %s plays %s (%s)\n' "$DF_RUN_HERO_ID" "$ROAM" "$DF_RUN_HERO_NAME"
  prepare_clean_tv || return 1
  prepare_hero_persona "$ROAM" "$ROAM_HERO_HINT" || return 1
  # The hero's own sheet from the look studio (`DF_RUN_HERO_SHEET`), used by name: no copy.
  if [ -z "$sheet" ] || ! library_holds "$sheet"; then
    prepare_failed reference \
      "the hero's reference sheet (${sheet:-none named}) is not in the library"
    return 1
  fi
  printf '[prepare] reference %s\n' "$sheet"
  out=$($D dev --port $P world inventory sheets "$ROAM" 2>&1) ||
    { prepare_failed kit "$out"; return 1; }
  printf '%s\n' "$out" | sed 's/^ */[prepare] kit sheets: /'
  if library_holds "$plate.png"; then
    printf '[prepare] starting plate %s.png kept\n' "$plate"
  else
    out=$($D dev --port $P media generate "$ROAM_START_PLATE" "as=$plate" purpose=plate \
      "refs=$sheet" wait=true 2>&1)
    printf '[prepare] starting plate: %s\n' "$(printf '%s\n' "$out" | head -1)"
    library_holds "$plate.png" ||
      { prepare_failed plate "$out"; return 1; }
    scan=$($D dev --port $P media locate "$plate.png" masks=1 2>&1) ||
      { prepare_failed plate "$scan"; return 1; }
    printf '[prepare] located: %s\n' "$(printf '%s\n' "$scan" | head -1)"
    # Regions are numbered from 0: the hero, alone on the plate, is the first the scan reports.
    region=$(printf '%s\n' "$scan" | sed -n 's/^ *\([0-9][0-9]*\): .*/\1/p' | head -1)
    out=$($D dev --port $P media locate label "$plate.png" "${region:-0}" "$ROAM" \
      "$DF_RUN_HERO_NAME" 2>&1) || { prepare_failed plate "$out"; return 1; }
    printf '[prepare] named: %s\n' "$(printf '%s\n' "$out" | head -1)"
  fi
  out=$($D dev --port $P scene image tv-backdrop "$plate.png" 2>&1) ||
    { prepare_failed shown "$out"; return 1; }
  printf '[prepare] shown: %s\n' "$(printf '%s\n' "$out" | head -1)"
}
# The TV starts clean, as the roam procedure does: the clean fixture (`painters_day`, bare: no
# caption, card, party, title or roleplay lines) with its live edits reset, no picture, place,
# clues or narration until the day's first plate is shown. Returns non-zero, with a `[prepare]
# failed: clean: ..` line, when a step is refused.
prepare_clean_tv() {
  local out
  out=$($D dev --port $P playground go "$CLEAN_FIXTURE" 2>&1) ||
    { prepare_failed clean "$out"; return 1; }
  out=$($D dev --port $P scene reset 2>&1) || { prepare_failed clean "$out"; return 1; }
  out=$($D dev --port $P scene image-clear tv-backdrop 2>&1) ||
    { prepare_failed clean "$out"; return 1; }
  $D dev --port $P world place - >/dev/null 2>&1
  $D dev --port $P scene clues clear >/dev/null 2>&1
  $D dev --port $P scene narration clear >/dev/null 2>&1
  printf '[prepare] clean: %s with nothing shown\n' "$CLEAN_FIXTURE"
}
# The hero's persona $1, kept when stored, else written from the hint $2 (`{name}`, `{species}` and
# `{class}` the hero's) and the locked design in words, with the hero's display name. Returns
# non-zero, with a `[prepare] failed: persona: ..` line, when there is none after.
prepare_hero_persona() { # persona id, hint
  local hint out
  if $D dev --port $P persona show "$1" >/dev/null 2>&1; then
    printf '[prepare] persona %s kept\n' "$1"
  else
    hint=${2//\{name\}/$DF_RUN_HERO_NAME}
    hint=${hint//\{species\}/$DF_RUN_HERO_SPECIES}
    hint=${hint//\{class\}/$DF_RUN_HERO_CLASS}
    hint="$hint${DF_RUN_HERO_LOOK:+ How they look: $DF_RUN_HERO_LOOK.}"
    out=$($D dev --port $P persona generate "$1" unplaced=true hint "$hint" 2>&1) ||
      { prepare_failed persona "$out"; return 1; }
    printf '[prepare] persona: %s\n' "$(printf '%s\n' "$out" | head -1)"
    out=$($D dev --port $P persona set "$1" display_name "$DF_RUN_HERO_NAME" 2>&1) ||
      { prepare_failed persona "$out"; return 1; }
    printf '[prepare] persona: %s\n' "$(printf '%s\n' "$out" | head -1)"
  fi
  $D dev --port $P persona show "$1" >/dev/null 2>&1 ||
    { prepare_failed persona "no persona is stored for $1"; return 1; }
}
# One `[prepare] failed: <step>: <reason>` line, the reason the first line of $2.
prepare_failed() { # step, output
  printf '[prepare] failed: %s: %s\n' "$1" "$(printf '%s\n' "$2" | head -1)"
}

# The cards of a template `<label> | <line>` with `{thing}` filled by each name of $2, one per line;
# a `<name><tab><thing id>` entry also names the thing the card is about (`| <thing id>`).
player_cards() { # template, names (one per line)
  local name thing
  [ -n "$1" ] || return 0
  while IFS=$'\t' read -r name thing; do
    [ -n "$name" ] && printf '%s%s\n' "${1//\{thing\}/$name}" "${thing:+ | $thing}"
  done <<< "$2"
}
# The look cards of thing lines $1: a thing's from `@roam_player_look_line`, a sight's (words, not
# a name: `sight-..`, roam_sights) from `@roam_player_sight_line`.
roam_look_cards() { # thing lines
  local pairs='s/^thing \([^ ]*\) (\([^)]*\)).*/\2\t\1/p'
  player_cards "$PLAYER_LOOK_LINE" "$(printf '%s\n' "$1" | grep -v '^thing sight-' | sed -n "$pairs")"
  player_cards "$PLAYER_SIGHT_LINE" "$(printf '%s\n' "$1" | grep '^thing sight-' | sed -n "$pairs")"
}
# The card template $1 when the thing studied (STUDY_THING_ID) is a sight, words rather than a
# name (`sight-<n>`, see roam_sights), else $2.
sight_or() { # sight template, thing template
  case "${STUDY_THING_ID:-}" in
    sight-*) printf '%s' "$1" ;;
    *) printf '%s' "$2" ;;
  esac
}
# What the hero saw of a place that has no things worth a close look (#51): one `thing
# sight-<move>-<n> (<words>): <words>` line for each sight of $2 (one per line, as `persona
# perceive` said them), in the shape of the design's thing lines so a card, the insert and the
# study can name it.
roam_sights() { # move number, sights
  local sight words number=0
  while IFS= read -r sight; do
    words=$(printf '%s' "$sight" | tr -d '()|' | tr -s ' ' | sed 's/^ //; s/[ .;]*$//')
    [ -n "$words" ] || continue
    number=$((number + 1))
    printf 'thing sight-%02d-%s (%s): %s\n' "$1" "$number" "${words,}" "$words"
  done <<< "$2"
}
# Adds each sight of $2 (roam_sights lines) to the host's active place as a thing of it (`world
# thing add`), so a close look at one is a look like any thing's (`@look`), and prints `[sights NN]
# <id>: <answer>`. A sight the host did not add joins FOCUS_THINGS (its close look is a focus on its
# words); ADDED_SIGHTS gets the lines of those added.
roam_add_sights() { # move number, sight lines
  local thing id name answer
  ADDED_SIGHTS=""
  while IFS= read -r thing; do
    [ -n "$thing" ] || continue
    id=${thing#thing }
    id=${id%% *}
    name=$(printf '%s\n' "$thing" | sed -n 's/^thing [^ ]* (\([^)]*\)).*/\1/p')
    if answer=$($D dev --port $P world thing add "$id" "$name" "|" "${thing#*): }" 2>&1); then
      ADDED_SIGHTS="$ADDED_SIGHTS$thing"$'\n'
    else
      FOCUS_THINGS="$FOCUS_THINGS${FOCUS_THINGS:+$'\n'}$thing"
    fi
    printf '[sights %02d] %s: %s\n' "$1" "$id" "$(printf '%s\n' "$answer" | head -1)"
  done <<< "$2"
}
# A place's name as said in a line: a leading article the design gave (A, An, The) becomes `the`
# (`Back to the mossy back-lane stair`, a place already known); the rest as it is.
place_said() { # place name
  case "$1" in
    'A '* | 'An '* | 'The '* | 'a '* | 'an '*) printf 'the %s' "${1#* }" ;;
    *) printf '%s' "$1" ;;
  esac
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
# R8: the move is designed, painted and walked in the background (`move act` with the arguments
# after the move number) and the talk the player sends meanwhile is taken as it comes (pause
# `move`). MOVE_OUT is the move's output.
roam_move_act() { # move number, move act arguments...
  local number=$1 file pid
  shift
  file="$(dirname "$FRAMES_LOG")/move-$ROAM-$number.out"
  mkdir -p "$(dirname "$file")"
  timeout $MOVE_WAIT $D dev --port $P move act "$@" > "$file" 2>&1 &
  pid=$!
  while kill -0 "$pid" 2>/dev/null; do
    seat_dwell "$number" move 1
  done
  wait "$pid"
  MOVE_OUT=$(cat "$file")
  rm -f "$file"
}
# Keeps the place of plate $1 to go back to (VISIT_*): its back card carries $7, the key of its
# scene memory, the Map page's id of the place, else the plate.
roam_keep_place() { # plate, place name, what the plate shows, things, elements, way, memory id
  local key=${7:-$1}
  [ -n "${VISIT_NAME[$1]:-}" ] || VISIT_ORDER+=("$1")
  VISIT_NAME[$1]=$2
  VISIT_PICTURE[$1]=$3
  VISIT_THINGS[$1]=$4
  VISIT_ELEMENTS[$1]=$5
  VISIT_WAY[$1]=$6
  VISIT_KEY[$1]=$key
  VISIT_PLATE[$key]=$1
}
# The move cards back to each place the hero has been but is not at ($1, the plate here), oldest
# first, from `@roam_player_back_line` (`{place}` its name, place_said), each carrying the place's
# key.
roam_back_cards() { # plate here
  local visit names=""
  for visit in "${VISIT_ORDER[@]}"; do
    [ "$visit" = "$1" ] && continue
    names="$names$(place_said "${VISIT_NAME[$visit]}")"$'\t'"${VISIT_KEY[$visit]}"$'\n'
  done
  player_cards "${PLAYER_BACK_LINE//"{place}"/"{thing}"}" "$names"
}
# A move back to a place the hero has been ($2, its plate: its back card tapped on the phone, by
# its key, or Jev's reading of the move's line naming it, `back_to=<plate>` from `move act`, which
# then stops; never by matching the words, via $4 card|jev): no design, plate or walk is made. The
# host makes the place the active one again (`world place <plate>`, its things and town kept), the
# hero says the move's line as written (`persona say text=`; a move Jev read has said it already)
# and the place's plate is shown again (`scene image`). When the
# host refuses, the refusal is logged and the place's things are looked at by a focus on their
# words. Prints `[revisit NN] ..`; sets REVISIT_SAID to the line said and REVISIT_RESTORED when
# the host's active place is it again.
roam_revisit() { # move number, plate, situation, card|jev|said
  local out="" shown restored
  REVISIT_RESTORED="" REVISIT_SAID=""
  if restored=$($D dev --port $P world place "$2" 2>&1); then
    REVISIT_RESTORED=1
  fi
  case "$4" in
    jev)
      # The move said the line already (`move act`, MOVE_OUT).
      out=$MOVE_OUT
      REVISIT_SAID=$(printf '%s\n' "$out" | sed -n "s/^$ROAM says: //p" | head -1) ;;
    said) ;; # The caller said the line already (explore_say).
    *)
      out=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true "text=$SAID_LINE" \
        ${SAID_TO:+"from=$SAID_TO"} "$3 You go back to $(place_said "${VISIT_NAME[$2]}")." 2>&1)
      REVISIT_SAID=$(printf '%s\n' "$out" | sed -n '1s/.*says: //p') ;;
  esac
  [ -z "$out" ] || roam_inventory "$REVISIT_SAID" "$out"
  shown=$($D dev --port $P scene image tv-backdrop "$2.png" 2>&1 | head -1)
  printf '[revisit %02d] back to %s (%s) key=%s via=%s | shown: %s | host place: %s | things: %s\n' \
    "$1" "$2" "${VISIT_NAME[$2]}" "${VISIT_KEY[$2]}" "$4" "${shown:--}" \
    "$(printf '%s\n' "$restored" | head -1)" \
    "$(printf '%s\n' "${VISIT_THINGS[$2]}" | grep -c '^thing ')"
}

# Free roam: the `@move` lines in order. Each move is said by the roaming character toward the goal
# and the way's situation (where they are, where they have been and what they hold now), run as a
# move with a plate of its own, and the close look at the new place waited out before the next.
play_roam() {
  local stamp number=0 way out said reading place picture plate things trace look line seq
  local here="" visited="" holding situation interpreted designed briefed painted live note
  local given=()
  local events_at held_before held_after paint paint_line insert_line inspected thing_name thing_look
  local board_at t_move t_plate t_look_from t_declare_from t_look t_declare look_from
  local here_plate="" revisit cards elements arrive_way focus card_name via back owned kept_things
  local key
  stamp=$(date +%m%d%H%M)
  # Played from a phone: the one seat the roam is played from, its hero the traveller.
  if [ -n "$ROAM_PLAYER" ]; then
    SEATS=("$ROAM_PLAYER")
    HERO_OF[$ROAM_PLAYER]=$ROAM
  fi
  # The soft cap on the pictures a frame follows goes to the watch only when this host takes it.
  if [ -n "$REFS_MAX" ] &&
    timeout 30 $D dev --port $P persona watch </dev/null 2>&1 | grep -q 'refs_max'; then
    WATCH_REFS="refs_max=$REFS_MAX"
  fi
  # A player's hero carries their bag from day to day: the kit is theirs on the first day only.
  # A dev run's character starts from the kit each run.
  if [ -n "${DF_RUN_HERO_ID:-}" ] && $D dev --port $P world inventory get "$ROAM" >/dev/null 2>&1
  then
    printf '       ~ %s carries the bag of their last day\n' "$ROAM"
  elif [ -n "$INVENTORY" ]; then
    $D dev --port $P world inventory set "$ROAM" character "$INVENTORY" | head -1 | sed 's/^/       ~ /'
  fi
  # The TV shows what the traveller holds, supplies going down as they are used up.
  $D dev --port $P world inventory show "$ROAM" | head -1 | sed 's/^/       ~ /'
  # Without the hero the day does not start: the host reads this status (`PREPARE_FAILED_STATUS`).
  if ! roam_prepare_hero; then
    printf '[prepare] the day does not start: the hero could not be made ready\n'
    exit 3
  fi
  # The soft cap holds for every plate the host builds, the move plates before the first watch too.
  if [ -n "$REFS_MAX" ] && [ -n "$WATCH_REFS" ]; then
    $D dev --port $P media backend "refs=$REFS_MAX" | head -1 | sed 's/^/       ~ /'
  fi
  roam_art_prices
  roam_tempo_start
  for way in "${MOVES[@]}"; do
    number=$((number + 1))
    caption_down
    seat_drain_talk "$number" pause
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
    note=""
    revisit=""
    while :; do
      given=()
      if [ -n "$ROAM_PLAYER" ]; then
        # The way's cards, then one back to each place the hero has been (not on the way to the
        # show, which ends the day where the work is shown).
        cards=${PLAYER_CARDS[$way]:-}
        [ "$way" = _show_ ] || cards="$cards$(roam_back_cards "$here_plate")"
        seat_line "$number" "$ROAM_PLAYER" "$PLAYER_ASK_MOVE" "$note" "$cards" move
        given=("text=$SAID_LINE")
        # A tapped back card is a move back to that place, by its key.
        via=card
        [ -n "$SAID_THING" ] && revisit=${VISIT_PLATE[$SAID_THING]:-}
        [ -n "$revisit" ] && break
      fi
      roam_move_act "$number" "$ROAM" "plate=$plate" $(roam_walk_options "$way") "${given[@]}" \
        "$situation"
      out=$MOVE_OUT
      # Words Jev reads as going back to a place the hero has been: its reading names one of the
      # host's kept places by id (`back_to=<plate>`, never a match of the words) and the move
      # stops there (`back at place=..`: no design, plate or walk).
      if printf '%s\n' "$out" | grep -q '^back at place='; then
        back=$(printf '%s\n' "$out" | sed -n 's/^back_to=//p' | head -1)
        via=jev
        revisit=${back:+${VISIT_NAME[$back]:+$back}}
        [ -n "$revisit" ] && break
        printf '[move %02d] back_to=%s is no place kept by the runner\n' "$number" "${back:--}"
        note="That place is not one you can go back to today. Say where you go next."
        [ -n "$ROAM_PLAYER" ] && continue
      fi
      [ -n "$ROAM_PLAYER" ] || break
      note=$(printf '%s\n' "$out" | sed -n 's/^not moved: //p' | head -1)
      [ -n "$note" ] || break
      printf '[player %02d] move not read: %s\n' "$number" "$note"
      # The judge's reason stays in the log; the phone gets a player's line.
      note=$JOURNEY_NOT_A_MOVE
    done
    if [ -n "$revisit" ]; then
      # Back to a place: its stored plate, things and elements, no design, plate or walk made.
      roam_revisit "$number" "$revisit" "$situation" "$via"
      t_plate=$((SECONDS - t_move))
      plate=$revisit
      said=$REVISIT_SAID
      picture=${VISIT_PICTURE[$plate]}
      place=${VISIT_NAME[$plate]}
      things=${VISIT_THINGS[$plate]}
      elements=${VISIT_ELEMENTS[$plate]}
      arrive_way=${VISIT_WAY[$plate]}
      roam_view_start "$number" "$said" "$picture"
      printf '[move %02d] %s | said: %s | read: back key=%s via=%s | place: %s | plate: %s | revisit\n' \
        "$number" "$way" "${said:-<none>}" "${VISIT_KEY[$plate]}" "$via" "$place" "$plate"
      seat_reply "$number" "$SAID_TO" "$ROAM" "$said"
      WALK_SECONDS=0
      # Its things are the host's again when the host made it the active place again; else a close
      # look at one is a focus on its words.
      FOCUS_THINGS=""
      [ -n "$REVISIT_RESTORED" ] || FOCUS_THINGS=$things
      LOOK_FRAMES=$(printf '%s-back%02d-look' "$plate" "$number")
    else
      t_plate=$((SECONDS - t_move))
      # The plate is on screen, so the perception of it starts at once; the move's bookkeeping
      # below runs while the model looks (the bookkeeping is a score of short commands that took
      # seconds).
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
      elements=$(printf '%s\n' "$out" | sed -n 's/^elements: //p' | head -1)
      interpreted=$(stage_at "$trace" designing)
      designed=$(stage_at "$trace" briefing)
      briefed=$(stage_at "$trace" generating)
      painted=$(stage_at "$trace" locating)
      printf '[move %02d] %s | said: %s | read: %s | place: %s | plate: %s | jev_ms=%s design_ms=%s plate_ms=%s live_at_ms=%s\n' \
        "$number" "$way" "${said:-<none>}" "${reading:-<none>}" "${place:-<none>}" "$plate" \
        "${interpreted:--}" "$(span "$interpreted" "$designed")" "$(span "$briefed" "$painted")" \
        "${live:--}"
      roam_inventory "$said" "$out"
      seat_reply "$number" "$SAID_TO" "$ROAM" "$said"
      roam_refs "${events_at:-0}" "$plate" "$number" "$out"
      printf '%s\n' "$out" | grep -E '^(sheet |place )' | sed 's/^/       /'
      roam_walk_lines "$number" "$out"
      if [ -z "$live" ]; then
        printf '       <move %02d did not go live: %s>\n' "$number" "$(printf '%s\n' "$out" | tail -1)"
        continue
      fi
      visited="${visited:+$visited; }$place"
      roam_remember_scene "$number" "$plate" "$place" "$picture"
      arrive_way=$way
      FOCUS_THINGS=""
      LOOK_FRAMES="$plate-look"
    fi
    here=$place
    here_plate=$plate
    roam_tempo "$number" arrival "$arrive_way"
    if [ "$way" = _show_ ]; then
      roam_tempo "$number" show
      roam_show "$number" "$plate" "$place" "$said"
      continue
    fi
    t_look_from=$SECONDS
    start_watch "${FRAMES_LOG%.log}-${LOOK_FRAMES%-look}.log" "frames=$LOOK_FRAMES" \
      "scene=$picture" ${ROAM_DECLARE_HOLD:+"declare_hold=$ROAM_DECLARE_HOLD"} \
      ${WATCH_REFS:+"$WATCH_REFS"} "${FRAMES[@]}"
    roam_view_finish "$number"
    # Only this place's own things (#53): what he noticed is a thing of it when the host's place is
    # it (a new place, or one the host made active again); noticed while the host's place was
    # another, it was added there, and is left out here.
    owned=$([ -z "$revisit" ] || [ -n "$REVISIT_RESTORED" ] && echo yes)
    if [ -n "$owned" ]; then
      things="$things${PERCEIVED_THINGS:+${things:+$'\n'}$PERCEIVED_THINGS}"
    elif [ -n "$PERCEIVED_THINGS" ]; then
      printf '[thing %02d] left out: %s noticed while the host'"'"'s place was another\n' \
        "$number" "$(printf '%s\n' "$PERCEIVED_THINGS" | grep -c '^thing ')"
    fi
    kept_things=$things
    # A place with no things worth a close look offers what he saw there (#51), each added to the
    # host's place as a thing of it when the host's place is it.
    if [ -z "$things" ] && [ -n "$PERCEIVED_SIGHTS" ]; then
      things=$(roam_sights "$number" "$PERCEIVED_SIGHTS")
      printf '[sights %02d] no things here: %s sight(s) offered as look cards\n' "$number" \
        "$(printf '%s\n' "$things" | grep -c '^thing ')"
      if [ -n "$owned" ]; then
        roam_add_sights "$number" "$things"
        kept_things=$ADDED_SIGHTS
      else
        FOCUS_THINGS=$things
      fi
    fi
    # The place is kept to go back to (again, with what was noticed now), by its key.
    if [ -n "$owned" ]; then
      key=$SCENE_MEMORY
      [ -n "$revisit" ] && key=${VISIT_KEY[$plate]}
      roam_keep_place "$plate" "$place" "$picture" "$kept_things" "$elements" "$arrive_way" "$key"
    fi
    seq=$(latest_event)
    given=()
    if [ -n "$ROAM_PLAYER" ]; then
      seat_line "$number" "$ROAM_PLAYER" "$PLAYER_ASK_LOOK" "" "$(roam_look_cards "$things")" look
      given=("text=$SAID_LINE")
    fi
    look=$(timeout $HUNG $D dev --port $P persona say "$ROAM" speak=true \
      ${SAID_TO:+"from=$SAID_TO"} "recent=$ROAM: $said" \
      "intent=$ROAM_LOOK" "${given[@]}" "You are at $place.$MIND" 2>&1)
    line=$(printf '%s\n' "$look" | sed -n '1s/.*says: //p')
    printf '[look %02d] %s\n' "$number" "${line:-<no line: $look>}"
    roam_inventory "$line" "$look"
    seat_reply "$number" "$SAID_TO" "$ROAM" "$line"
    # A tapped look card names the thing: the close look is of it or of nothing.
    look_from=judge
    focus=""
    card_name=""
    if [ -n "$SAID_THING" ]; then
      look_from=card
      card_name=$(printf '%s\n' "$things" |
        sed -n "s/^thing $SAID_THING (\([^)]*\)).*/\1/p" | head -1)
      if printf '%s\n' "$FOCUS_THINGS" | grep -q "^thing $SAID_THING ("; then
        # No thing of the host's place: the close look is a focus on its words.
        focus=focus
        watch_line "$ROAM" "$line"
        [ -n "$WATCH_PID" ] && [ -n "$line" ] && printf '@focus %s\n' "$(printf '%s\n' \
          "$FOCUS_THINGS" | sed -n "s/^thing $SAID_THING ([^)]*): //p" | head -1 |
          tr '|' '-')" >&4
      else
        watch_look "$ROAM" "$SAID_THING" "$line"
      fi
    else
      watch_line "$ROAM" "$line"
    fi
    inspected=""
    if [ -n "$line" ]; then
      insert_line=$(roam_insert "${seq:-0}" "$LOOK_FRAMES" "$number" "$things" "$look_from" \
        "$SAID_THING" "$focus")
      printf '%s\n' "$insert_line"
      inspected=$(field_of "$insert_line" item)
      [ -n "$inspected" ] && roam_tempo "$number" look
      # R6: the phone is told when the close look its card asked for could not be made.
      if [ "$look_from" = card ]; then
        case "$insert_line" in
          *'] none: '* | *' failed after '*)
            seat_notice "$number" "$ROAM_PLAYER" \
              "No close look at ${card_name:-that} this time." ;;
        esac
      fi
    fi
    # What the study is of: the thing looked at closely, else the card the player tapped, else the
    # place as the design describes it.
    [ -n "$inspected" ] || inspected=$SAID_THING
    thing_name=$(printf '%s\n' "$things" | sed -n "s/^thing $inspected (\([^)]*\)).*/\1/p" | head -1)
    thing_look=$(printf '%s\n' "$things" | sed -n "s/^thing $inspected ([^)]*): //p" | head -1)
    STUDY_THING_ID=$inspected
    if [ -z "$thing_name" ]; then
      thing_name=$place
      thing_look=$elements
      STUDY_THING_ID=""
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
    if [ -n "${ARRIVAL[$arrive_way]:-}" ]; then
      # This kind of move has its own arrival step (a trade at a market), not the declaration; a
      # place gone back to keeps the kind of move it was first reached by.
      roam_trade "$number" "${line:-$said}" "$place" "${ARRIVAL[$arrive_way]}"
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

# Explore (`@explore on`, with `@roam` and a phone seat): dynamic single-player free roam
# (`planning/explore.md`). One open ask per turn ("What do you do?", `@explore_ask`) with cards
# built from the scene the host knows each turn (`persona intent scene`: a talk card per person, a
# trade card per seller, a look card per thing, the scenario's way cards `@roam_player_card <way>`
# and, during an approach, a card to keep walking or to go up to it;
# `@explore_card_talk|trade|look|walk|reach <label> | <line>`, `{thing}` the name). Every line is
# Jev's to read (`persona intent`: one intent of move, look, talk, trade, act, ask, done or
# clarify, with up to two follow-on steps, its ids checked by the host against the scene, the
# `@way` ids and the distances the place design gave), and each intent goes to its one handler:
# a move by a way to `move act` (design, plate and walk, as the roam's moves; a way's words are its
# meaning, `@way`, and `@walk` its sights), a move toward something on screen to `move approach`
# (one step, a plate continuing the one before from closer in, told by the DM; the step that goes
# up to it is its close-up; the next step is made ahead in the background, `ahead=true`, and taken
# with `@take` on its input when the player goes on, else dropped and kept in the library
# unshown), a look to the frame
# judge's insert (`@look` to the watch), a talk to the person's answer (seat_say, seat_reply), a
# trade to the deal path (the line said to the seller, `from=`, and their answer), an act to the
# check judge (`persona check run` with the hero's rules sheet, `creation sheet`), a clarify to one
# question back as the next ask's note, and done ends the roam. No move list and no turn cap.
# The log: `[intent NN]`, `[move NN]`, `[approach NN]`, `[look NN]`, `[act NN]`, `[trade NN]`,
# `[explore] ..`, besides the seat helpers' `[ask NN]`, `[said NN]` and `[answer NN]`.
# The scene the hero is in now (`persona intent scene`): SCENE_THINGS, SCENE_PEOPLE and
# SCENE_SELLERS as `<name>\t<id>` lines (player_cards' input).
explore_scene() {
  local out
  # A scene not read (the host did not answer) keeps the last one: the cards stay while the hero
  # stays at the place.
  if ! out=$(timeout $TEXT_WAIT $D dev --port $P persona intent scene "$ROAM" 2>&1); then
    printf '[scene %02d] not read, the last scene kept: %s\n' "$NUMBER" \
      "$(printf '%s\n' "$out" | head -1)"
    return 0
  fi
  SCENE_THINGS=$(printf '%s\n' "$out" | sed -n 's/^thing \([^ ]*\) (\(.*\))$/\2\t\1/p')
  SCENE_LOOKS=$(printf '%s\n' "$out" | sed -n 's/^look //p')
  # The host's people of the scene never include the hero (`people_of`).
  SCENE_PEOPLE=$(printf '%s\n' "$out" | sed -n 's/^person \([^ ]*\) (\(.*\)) sells=.*/\2\t\1/p')
  SCENE_SELLERS=$(printf '%s\n' "$out" |
    sed -n 's/^person \([^ ]*\) (\(.*\)) sells=true$/\2\t\1/p')
}
# The look of thing $1 as the host keeps it (its notes told after it), from the last scene read.
thing_look() { # thing
  printf '%s\n' "$SCENE_LOOKS" | sed -n "s/^$1 //p" | head -1
}
# The things of the host's place, as the scene gave them (SCENE_THINGS), in the shape of the
# design's thing lines that roam_insert names an inserted thing by.
explore_thing_lines() {
  printf '%s\n' "$SCENE_THINGS" | awk -F'\t' 'NF == 2 { printf "thing %s (%s): \n", $2, $1 }'
}
# The cards of this turn, from the scene: talk to each person, the wares of each seller, a close
# look at each thing, each way on, and the walk on toward the target of an approach under way.
explore_cards() {
  local way
  if [ -n "$EXPLORE_RETRY" ]; then
    printf 'Send it again | %s\n' "$(printf '%s' "$EXPLORE_RETRY" | tr '|\n' '/ ')"
  fi
  # During an approach the walk on comes first: the phone shows the first four cards. Its thing
  # is the continuation's marker (explore_continue_card), never a thing of the scene. A way (a
  # direction) is never gone up to: its walk only goes on.
  if [ -n "$APPROACH_TARGET" ]; then
    local card
    if [ "$APPROACH_AWAY" = true ]; then
      card=$EXPLORE_CARD_AWAY
    elif [ "$APPROACH_NEXT_REACHES" = true ]; then
      card=$EXPLORE_CARD_REACH
    else
      card=$EXPLORE_CARD_WALK
    fi
    # A person is walked toward by their name, which takes no article ("Go up to Mara").
    printf '%s\n' "$SCENE_PEOPLE" | awk -F'\t' -v id="$APPROACH_TARGET" '$2 == id' |
      grep -q . && card=${card//"the {thing}"/"{thing}"}
    player_cards "$card" \
      "$(explore_words "" "$APPROACH_TOWARD")"$'\t'"continue-$APPROACH_TARGET"
  fi
  player_cards "$EXPLORE_CARD_TALK" "$(printf '%s\n' "$SCENE_PEOPLE" | sed 's/^The /the /')"
  player_cards "$EXPLORE_CARD_TRADE" "$SCENE_SELLERS"
  player_cards "$EXPLORE_CARD_LOOK" "$SCENE_THINGS"
  for way in "${!WAY[@]}"; do
    printf '%s' "${PLAYER_CARDS[$way]:-}"
  done
}
# A tap of the walk-on card ($1 the card's thing, `continue-<approach target>`): the approach under
# way goes on at once, the step made ahead taken, with no judgement asked (the card is the
# player's explicit choice; its marker is the protocol's).
explore_continue_card() { # card thing
  [ -n "$APPROACH_TARGET" ] && [ "$1" = "continue-$APPROACH_TARGET" ] || return 1
  INTENT_OUT="" INTENT_CANDIDATES="" INTENT_QUESTION="" INTENT_FALLBACK="" INTENT_UNUSED_FIELD=""
  intent_load 0
  INTENT_KIND=move INTENT_CONTINUE=true INTENT_TARGET=$APPROACH_TARGET
  INTENT_DISTANCE=$APPROACH_DISTANCE
  printf '[intent %02d] card: continue the approach toward %s (%s), step %s\n' "$NUMBER" \
    "$APPROACH_TOWARD" "$APPROACH_TARGET" "$((APPROACH_STEP + 1))"
}
# The value of field $2 of step $1 of the reading $3 (INTENT_OUT by default), empty for none.
intent_field() { # step, field, [reading]
  printf '%s\n' "${3-$INTENT_OUT}" | sed -n "s/^step=$1 $2=//p" | head -1 | sed 's/^-$//'
}
# How many steps the reading $1 (INTENT_OUT by default) has: its first intent and each follow-on.
intent_steps() { # [reading]
  printf '%s\n' "${1-$INTENT_OUT}" | grep -c '^step=[0-9]* kind='
}
# Step $1 of the reading $2 (INTENT_OUT by default) as the turn's intent: every field of
# INTENT_FIELDS set, an absent one or `-` empty.
intent_load() { # step, [reading]
  local field value line
  for field in $INTENT_FIELDS; do printf -v "INTENT_${field^^}" '%s' ""; done
  while IFS= read -r line; do
    field=${line%%=*} value=${line#*=}
    [[ " $(printf '%s' "$INTENT_FIELDS" | tr '\n' ' ') " == *" $field "* ]] || continue
    [ "$value" = - ] && value=""
    printf -v "INTENT_${field^^}" '%s' "$value"
  done < <(printf '%s\n' "${2-$INTENT_OUT}" | sed -n "s/^step=$1 //p")
}
# Jev's reading of the line $2 (`persona intent`): sets INTENT_KIND (move, look, talk, trade, act,
# done, clarify, or unjudged), INTENT_TARGET, INTENT_WAY, INTENT_DIRECTION, INTENT_TOWARD,
# INTENT_DISTANCE and INTENT_QUESTION (empty for none) and prints `[intent NN]`. `$3` is what the
# player picked on the phone with it (a card's thing, or whom they addressed).
explore_intent() { # number, line, picked
  local out args=() way id field
  for way in "${!WAY[@]}"; do args+=("way=$way|${WAY[$way]}"); done
  for id in "${!DIST[@]}"; do args+=("distance=$id:${DIST[$id]}"); done
  [ -n "$APPROACH_TARGET" ] &&
    args+=("approaching=$APPROACH_TARGET|${APPROACH_TOWARD:-$APPROACH_TARGET}|$APPROACH_STEP")
  [ -n "$3" ] && args+=("picked=$3")
  # The places the hero has been and may go back to (a move by a way, the start, or the stop
  # beside what an approach reached; never another step of an approach), but the one they are at.
  for id in "${VISIT_ORDER[@]}"; do
    [ "$id" != "$HERE_PLATE" ] && [ "${VISIT_WAY[$id]}" != approach ] &&
      args+=("visited=$id|${VISIT_NAME[$id]}")
  done
  recent_args
  # Each line of the talk is passed whole up to a bound, so a long line kept in it cannot push
  # every later call over the command line's size.
  local recent=() line
  for line in "${RECENT[@]}"; do recent+=("${line:0:$EXPLORE_RECENT_CHARS}"); done
  out=$(timeout $TEXT_WAIT $D dev --port $P persona intent "${args[@]}" "${recent[@]}" "$ROAM" "$2" \
    2>&1)
  INTENT_OUT=$out
  intent_load 0
  for field in candidates question fallback unused_field; do
    printf -v "INTENT_${field^^}" '%s' \
      "$(printf '%s\n' "$out" | sed -n "s/^$field=//p" | head -1 | sed 's/^-$//')"
  done
  [ "$INTENT_FALLBACK" = true ] &&
    printf '[intent %02d] fallback: no reading stood, the line is played as an act\n' "$1"
  [ -n "$INTENT_UNUSED_FIELD" ] &&
    printf '[intent %02d] unused_field=%s (left out)\n' "$1" "$INTENT_UNUSED_FIELD"
  printf '[intent %02d] kind=%s target=%s way=%s direction=%s distance=%s from=%s%s\n' "$1" \
    "${INTENT_KIND:-<none: $(printf '%s\n' "$out" | head -1)>}" "${INTENT_TARGET:--}" \
    "${INTENT_WAY:--}" "${INTENT_DIRECTION:--}" "${INTENT_DISTANCE:--}" \
    "${INTENT_DISTANCE_FROM:--}" \
    "$([ -n "$INTENT_TOWARD" ] && printf ' toward="%s"' "$INTENT_TOWARD")$([ -n \
      "$INTENT_CANDIDATES" ] && printf ' candidates=%s' "$INTENT_CANDIDATES")$([ \
      "$INTENT_CONTINUE" = true ] && printf ' continue=true')$([ -n "$INTENT_REVISIT" ] &&
      printf ' revisit=%s' "$INTENT_REVISIT")$([ -n "$INTENT_PACE" ] &&
      printf ' pace=%s' "$INTENT_PACE")"
}
# The frame watch of the place on screen (its plate's frames, its picture's words), for the close
# looks of its things. Each start takes frames of its own (EXPLORE_FRAMES, `<plate>-wNN-look`): a
# frame name already in the library is kept and never made again (frame_watch), so a watch
# started again at a place (one gone back to, or after a move that did not land) under the same
# name would make no insert, and a look there would wait for one that never comes.
EXPLORE_FRAMES=""
EXPLORE_WATCHES=0
explore_watch() {
  stop_watch quiet
  EXPLORE_WATCHES=$((EXPLORE_WATCHES + 1))
  EXPLORE_FRAMES=$(printf '%s-w%02d-look' "$HERE_PLATE" "$EXPLORE_WATCHES")
  start_watch "${FRAMES_LOG%.log}-${EXPLORE_FRAMES%-look}.log" "frames=$EXPLORE_FRAMES" \
    ${HERE_PICTURE:+"scene=$HERE_PICTURE"} ${WATCH_REFS:+"$WATCH_REFS"} "${FRAMES[@]}"
  printf '[watch %02d] frames=%s at %s\n' "$NUMBER" "$EXPLORE_FRAMES" "$HERE_PLATE"
}
# The place of a move's or a step's output $2 (plate $1) is the one the hero is at now: its name,
# picture, things and distances, kept to go back to under its key ($3 the kind of move). The place
# left is kept as the one behind (PREV_NAME, PREV_THINGS), for a turn to look back.
PREV_NAME="" PREV_THINGS=""
explore_here() { # plate, output (none for a revisit), way: approach|way|pose|revisit|..
  local line id band key before=$HERE_PLATE
  [ -n "$HERE_PLATE" ] && [ "$HERE_PLATE" != "$1" ] && PREV_NAME=$HERE_NAME PREV_THINGS=$HERE_THINGS
  HERE_PLATE=$1
  if [ "$3" = revisit ]; then
    # A place been to, shown again as it was left: its kept name, picture and things.
    HERE_NAME=${VISIT_NAME[$1]} HERE_PICTURE=${VISIT_PICTURE[$1]} HERE_THINGS=${VISIT_THINGS[$1]}
    DIST=()
    here=$HERE_NAME
    return 0
  fi
  HERE_PICTURE=$(printf '%s\n' "$2" | sed -n 's/^picture: //p' | head -1)
  printf '%s\n' "$2" | grep '^PERSONA ' | sed "s/^/[persona $(printf '%02d' "$NUMBER")] /"
  if [ "$3" = pose ]; then
    # A pose plate is the same spot: the place keeps its name, things (ids, looks, states),
    # distances and the boxes the searches found on the plate before.
    for key in "${!THING_BOX[@]}"; do
      [ "${key%%:*}" = "$before" ] && THING_BOX[$1:${key#*:}]=${THING_BOX[$key]}
    done
  else
    HERE_NAME=$(printf '%s\n' "$2" | sed -n 's/^design: place=\(.*\) mood=.*/\1/p' | head -1)
    HERE_THINGS=$(printf '%s\n' "$2" | grep '^thing ')
    DIST=()
    while read -r _ id band; do
      [ -n "$id" ] && DIST[$id]=$band
    done < <(printf '%s\n' "$2" | grep '^distance ')
  fi
  here=$HERE_NAME
  [ "$3" = pose ] || visited="${visited:+$visited; }$HERE_NAME"
  roam_remember_scene "$NUMBER" "$1" "$HERE_NAME" "$HERE_PICTURE"
  roam_keep_place "$1" "$HERE_NAME" "$HERE_PICTURE" "$HERE_THINGS" \
    "$(printf '%s\n' "$2" | sed -n 's/^elements: //p' | head -1)" "$3" "$SCENE_MEMORY"
}
# Drops the step made ahead: its input is closed (no `@take`), so it ends kept in the library and
# never shown. The player is not kept waiting for it: it finishes in the background and is reaped
# when the roam ends (explore_reap).
DROPPED=()
explore_drop_ahead() {
  [ -n "$AHEAD_PID" ] || return 0
  exec 5>&-
  printf '[approach %02d] ahead dropped plate=%s\n' "$NUMBER" "$AHEAD_PLATE"
  DROPPED+=("$AHEAD_PID $AHEAD_FILE")
  AHEAD_PID="" AHEAD_FILE="" AHEAD_PLATE=""
}
# Waits for every step dropped ahead and prints how each ended (`approach kept ..`).
explore_reap() {
  local job pid file
  for job in "${DROPPED[@]}"; do
    pid=${job%% *}
    file=${job#* }
    wait "$pid"
    printf '[approach] dropped: %s\n' "$(grep -m1 '^approach kept' "$file" || tail -1 "$file")"
    rm -f "$file"
  done
  DROPPED=()
}
# The approach under way ends: its step made ahead is dropped.
explore_end_approach() {
  explore_drop_ahead
  APPROACH_TARGET="" APPROACH_TOWARD="" APPROACH_STEP=0 APPROACH_DISTANCE="" APPROACH_DIRECTED=""
  APPROACH_PACE=walk APPROACH_AWAY=false APPROACH_NEXT_REACHES=""
}
# The `move approach` arguments of step $1 of the approach under way (APPROACH_ARGS).
approach_args() { # step
  APPROACH_ARGS=("target=$APPROACH_TARGET" "toward=$APPROACH_TOWARD"
    "distance=$APPROACH_DISTANCE" "step=$1" "pace=$APPROACH_PACE" "away=$APPROACH_AWAY")
}
# The plate name of step $1 of the approach under way, with suffix $2 (`-again`, `-run`).
approach_plate() { # step, [suffix]
  printf 'explore-%s-a%02d-s%02d%s' "$STAMP" "$APPROACH_RUN" "$1" "${2:-}"
}
# A move that did not happen: the rest of the line is not done (EXPLORE_MOVE_FAILED), the player
# is told $2, an approach under way ends ($3 `approach`), and the watch starts again on the place
# on screen.
explore_move_failed() { # number, notice, [approach]
  EXPLORE_MOVE_FAILED=yes
  seat_notice "$1" "$ROAM_PLAYER" "$2"
  [ "${3:-}" = approach ] && explore_end_approach
  explore_watch
}
# Starts the next step of the approach under way in the background, made ahead and shown only on
# `@take` (fd 5 is its input).
explore_start_ahead() {
  local step=$((APPROACH_STEP + 1))
  AHEAD_PLATE=$(approach_plate "$step")
  AHEAD_FILE="$(dirname "$FRAMES_LOG")/ahead-$ROAM-$AHEAD_PLATE.out"
  mkdir -p "$(dirname "$AHEAD_FILE")"
  approach_args "$step"
  exec 5> >(timeout $HUNG $D dev --port $P move approach "$ROAM" "${APPROACH_ARGS[@]}" \
    "from=$HERE_PLATE" "plate=$AHEAD_PLATE" ahead=true 4>&- > "$AHEAD_FILE" 2>&1)
  AHEAD_PACE=$APPROACH_PACE
  AHEAD_PID=$!
  printf '[approach %02d] ahead step=%s plate=%s started\n' "$NUMBER" "$step" "$AHEAD_PLATE"
}
# The approach's target by the name the step's design gives it (`thing <id> (<name>)`), so the
# cards and the log say what is walked toward ("blue hill ridge"), never a direction's key.
explore_name_approach() {
  local name
  # A walk a way (a direction, a route) keeps the player's own words: the thing the step's design
  # puts there is only the way's stand-in.
  [ -z "$APPROACH_DIRECTED" ] || return 0
  name=$(printf '%s\n' "$HERE_THINGS" | sed -n "s/^thing $APPROACH_TARGET (\([^)]*\)).*/\1/p" |
    head -1)
  [ -n "$name" ] && APPROACH_TOWARD=$name
  return 0
}
# One step of the approach under way, number $2 (0: a turn to face it), made now as plate $3
# (`move approach`), the player's talk taken while it is made; STEP_OUT is its output.
explore_step() { # number, step, plate
  local file pid
  file="$(dirname "$FRAMES_LOG")/approach-$ROAM-$3.out"
  mkdir -p "$(dirname "$file")"
  approach_args "$2"
  timeout $EXPLORE_STEP_WAIT $D dev --port $P move approach "$ROAM" "${APPROACH_ARGS[@]}" \
    "from=$HERE_PLATE" "plate=$3" > "$file" 2>&1 &
  pid=$!
  while kill -0 "$pid" 2>/dev/null; do seat_dwell "$1" move 1; done
  wait "$pid"
  STEP_OUT=$(cat "$file")
  rm -f "$file"
}
# A look a way no id names ($2 a direction's key, $3 what lies there): the hero turns to face it
# where they stand, a step 0 of the approach path (its plate the view that way, its place the
# active one), and that way becomes the approach a later "keep walking" goes on.
explore_turn() { # number, direction, toward
  local out plate
  explore_end_approach
  APPROACH_RUN=$((APPROACH_RUN + 1))
  APPROACH_TARGET=$2
  APPROACH_TOWARD=$3
  APPROACH_DISTANCE=on_the_horizon
  APPROACH_STEP=0
  APPROACH_DIRECTED=direction
  wait_heard
  stop_watch quiet
  plate=$(printf 'explore-%s-a%02d-s00' "$STAMP" "$APPROACH_RUN")
  # Behind the hero is the way they came: the view that way is told the place just left and what
  # was there, so what they passed is still there when they look back.
  if [ "$2" = behind ] && [ -n "$PREV_NAME" ]; then
    APPROACH_TOWARD="$3, back the way you came toward $PREV_NAME$(printf '%s\n' "$PREV_THINGS" |
      sed -n 's/^thing [^ ]* (\([^)]*\)).*/\1/p' | paste -sd ',' - | sed 's/,/, /g; s/^./, with &/')"
  fi
  explore_step "$1" 0 "$plate"
  APPROACH_TOWARD=$3
  out=$STEP_OUT
  printf '%s\n' "$out" | grep '^approach ' | sed "s/^/[turn $(printf '%02d' "$1")] /"
  if ! printf '%s\n' "$out" | grep -q '^approach shown '; then
    printf '[turn %02d] no view that way: %s\n' "$1" "$(printf '%s\n' "$out" | tail -1)"
    explore_end_approach
    explore_watch
    return 0
  fi
  explore_here "$plate" "$out" approach
  explore_name_approach
  APPROACH_DISTANCE=${DIST[$APPROACH_TARGET]:-$APPROACH_DISTANCE}
  printf '[turn %02d] facing %s (%s): %s away, plate %s\n' "$1" "$2" "$3" \
    "$APPROACH_DISTANCE" "$plate"
  explore_watch
}
# The dungeon master tells what a look shows or how an act turns out ($2 look|act), from the
# world as it stands: the place, its things and people, what the plate shows and the hero's deed
# $3, with how a roll came out $4 for an act (`persona story narrate .. look|act`, the narrator,
# with no story needed), shown and voiced (`persona narrate`). Always a line, or the failure
# logged and the player told.
LAST_DM_LINE="" LAST_DM_NUMBER=""
# What came of an act ($4, data: how a roll came out, what changed hands) and the narrator's
# typed fields after it (`reads=true`, `looks_into=`, `impossible=true`, `stays=true`,
# `stance=`, `unanswered=true`), passed as they are.
explore_narrate() { # number, look|act|answer, deed, [outcome], [field=value]...
  local out line status sees=() sight ways=() way
  NARRATED=""
  recent_args
  # What the hero's eyes found on the plate (explore_perceive): the narrator never denies it.
  while IFS= read -r sight; do
    [ -n "$sight" ] && sees+=("sees=$sight")
  done <<< "$EXPLORE_SEEN"
  # An answer of what can be done here names the ways on as the cards offer them.
  if [ "$2" = answer ]; then
    for way in "${!WAY[@]}"; do
      ways+=("way=$(journey_field "${PLAYER_CARDS[$way]:-$way}" 1)")
    done
  fi
  local argv=(persona story narrate "hero=$ROAM" "deed=$3" ${4:+"outcome=$4"}
    ${HERE_PICTURE:+"scene=$HERE_PICTURE"} "${sees[@]}" "${ways[@]}" "${@:5}"
    ${DF_RUN_HERO_LOOK:+"hero_look=$DF_RUN_HERO_LOOK"}
    "${RECENT[@]}" "$(place_id "$HERE_PLATE")" "$2")
  out=$(timeout $TEXT_WAIT $D dev --port $P "${argv[@]}" 2>&1)
  status=$?
  line=$(printf '%s\n' "$out" | head -1)
  if [ "$status" -ne 0 ] || [ -z "$line" ]; then
    printf '[dm %02d] <no narration: %s>\n' "$1" "${line:-empty}"
    # The words the CLI was given, each quoted and cut short, so a refusal can be read back.
    printf '[dm %02d] argv:' "$1"
    printf ' %q' "${argv[@]:0:3}"
    for word in "${argv[@]:3}"; do printf ' %q' "${word:0:80}"; done
    printf '\n'

    # What the deed did stands: the player is not asked to do it again.
    seat_notice "$1" "$ROAM_PLAYER" "The game master is lost for words, but what you did stands."
    return 0
  fi
  if [ "$line" = "$LAST_DM_LINE" ] && [ "$1" = "$LAST_DM_NUMBER" ]; then
    printf '[dm %02d] the same line as just said: not said again\n' "$1"
    NARRATED=$line
    return 0
  fi
  LAST_DM_LINE=$line LAST_DM_NUMBER=$1
  wait_heard
  printf '[dm %02d] %s: %s\n' "$1" "$2" "$line"
  $D dev --port $P persona narrate speak=true "$line" >/dev/null 2>&1 ||
    printf '[dm %02d] <not shown>\n' "$1"
  HISTORY+=("narrator: $line")
  NARRATED=$line
  watch_line narrator "$line"
  heard_out "$line"
}
# A move toward something on screen ($2 an id of the scene or a direction's key, $3 its words, $4
# how far it is now), the hero having said $5: within reach, its close-up; else one step, the one
# made ahead when the player keeps walking toward the same target, else a step made now. A shown
# step is the place the hero is at, and the next step is started ahead.
explore_approach() { # number, target, toward, distance, line, [direction]
  local out plate taken="" after until
  explore_say "$1" "" "$5" "You walk on toward $3." || true
  if [ "$APPROACH_TARGET" != "$2" ]; then
    explore_end_approach
    APPROACH_RUN=$((APPROACH_RUN + 1))
    APPROACH_TARGET=$2
    APPROACH_TOWARD=$3
    APPROACH_DISTANCE=$4
    APPROACH_DIRECTED=${6:-}
    # Backing away from it (the judge's away): it grows farther, step by step.
    APPROACH_AWAY=${INTENT_AWAY:-false}
  elif [ -n "$3" ] && [ "$3" != "$APPROACH_TOWARD" ]; then
    # The same way, a new destination named (toward the hills, after a look at the sky).
    APPROACH_TOWARD=$3
  fi
  # How the hero goes on this step (the intent judge's pace): a run covers more ground. A step
  # made ahead at another pace is dropped and this one made at the new pace.
  APPROACH_PACE=${INTENT_PACE:-walk}
  local renamed=""
  if [ -n "$AHEAD_PID" ] && [ "$AHEAD_PACE" != "$APPROACH_PACE" ]; then
    explore_drop_ahead
    renamed="-$APPROACH_PACE"
  fi
  wait_heard
  stop_watch quiet
  if [ -n "$AHEAD_PID" ]; then
    taken=yes
    plate=$AHEAD_PLATE
    # The word goes only to a step still waiting for it, and from a subshell that ignores
    # SIGPIPE: a step made ahead that already ended (it failed) can never kill the runner.
    if kill -0 "$AHEAD_PID" 2>/dev/null; then
      ( trap '' PIPE; printf '@take\n' >&5 ) 2>/dev/null
    fi
    exec 5>&-
    # Taken, it is shown once made: within a step's bound, else it is given up and nothing more
    # is made for this turn (the no-step fallback below), so the next ask comes.
    until=$((SECONDS + EXPLORE_STEP_WAIT))
    while kill -0 "$AHEAD_PID" 2>/dev/null && [ "$SECONDS" -lt "$until" ]; do
      seat_dwell "$1" move 1
    done
    if kill -0 "$AHEAD_PID" 2>/dev/null; then
      kill "$AHEAD_PID" 2>/dev/null
      printf '[approach %02d] waited_out ms=%s plate=%s: the step made ahead was not shown in time\n' \
        "$1" "$((EXPLORE_STEP_WAIT * 1000))" "$plate"
      wait "$AHEAD_PID" 2>/dev/null
      rm -f "$AHEAD_FILE"
      AHEAD_PID="" AHEAD_FILE="" AHEAD_PLATE=""
      explore_move_failed "$1" "You couldn't get any closer this time." approach
      return 0
    fi
    wait "$AHEAD_PID"
    out=$(cat "$AHEAD_FILE")
    rm -f "$AHEAD_FILE"
    AHEAD_PID="" AHEAD_FILE="" AHEAD_PLATE=""
    # It ended before it was shown: the step is made now instead.
    if ! printf '%s\n' "$out" | grep -q '^approach shown '; then
      printf '[approach %02d] the step made ahead failed: %s; making it now\n' "$1" \
        "$(printf '%s\n' "$out" | tail -1)"
      taken=""
      plate=$(approach_plate "$((APPROACH_STEP + 1))" -again)
      explore_step "$1" "$((APPROACH_STEP + 1))" "$plate"
      out=$STEP_OUT
    fi
  else
    plate=$(approach_plate "$((APPROACH_STEP + 1))" "$renamed")
    explore_step "$1" "$((APPROACH_STEP + 1))" "$plate"
    out=$STEP_OUT
  fi
  printf '%s\n' "$out" | grep '^approach ' | sed "s/^/[approach $(printf '%02d' "$1")] /"
  if ! printf '%s\n' "$out" | grep -q '^approach shown '; then
    printf '[approach %02d] no step: %s\n' "$1" "$(printf '%s\n' "$out" | tail -1)"
    explore_move_failed "$1" "You couldn't get any closer this time." approach
    return 0
  fi
  APPROACH_STEP=$((APPROACH_STEP + 1))
  # Whether the next step goes right up to it, as the host's step says (`approach next_reaches=`).
  APPROACH_NEXT_REACHES=$(printf '%s\n' "$out" | sed -n 's/^approach next_reaches=//p' | head -1)
  explore_carry_ground "$1" "$HERE_PLATE" "$plate" "$APPROACH_TARGET"
  explore_here "$plate" "$out" approach
  explore_name_approach
  after=${DIST[$APPROACH_TARGET]:-}
  printf '[approach %02d] step %s toward %s [%s]: %s -> %s, plate %s%s\n' "$1" "$APPROACH_STEP" \
    "$APPROACH_TOWARD" "$APPROACH_TARGET" "$APPROACH_DISTANCE" "${after:-?}" "$plate" \
    "${taken:+ (made ahead, taken)}"
  APPROACH_DISTANCE=${after:-$APPROACH_DISTANCE}
  roam_tempo "$1" walk
  explore_watch
  # The dungeon master tells what is newly in view, from the step's place and plate: once. A
  # step that reaches what the next step of the line looks at is told by that look.
  EXPLORE_SEEN=""
  if printf '%s\n' "$out" | grep -q '^approach reached ' &&
    [ "$(intent_field "$((${EXPLORE_STEP_AT:-0} + 1))" kind)" = look ]; then
    printf '[approach %02d] arrival told by the look that follows\n' "$1"
  else
    explore_narrate "$1" look "$HERO_SAID"
  fi
  if printf '%s\n' "$out" | grep -q '^approach reached '; then
    # Arrived: the plate is the target up close, the hero beside it; the approach ends.
    printf '[approach %02d] reached %s [%s], plate %s\n' "$1" "$APPROACH_TOWARD" \
    "$APPROACH_TARGET" "$plate"
    REACHED_TARGET=$APPROACH_TARGET REACHED_PLATE=$plate
    # The stop beside it is a place to go back to ("back to the signpost"), offered to the judge.
    VISIT_WAY[$plate]=reached
    VISIT_TARGET[$plate]=$APPROACH_TARGET
    VISIT_NAME[$plate]="beside the $(explore_words "" "$APPROACH_TOWARD")"
    roam_tempo "$1" look
    explore_end_approach
    return 0
  fi
  # A later step of the same line that walks elsewhere leaves the approach: nothing is made ahead.
  if printf '%s\n' "$INTENT_OUT" | sed -n 's/^step=\([0-9]*\) kind=move$/\1/p' |
    awk -v at="${EXPLORE_STEP_AT:-0}" '$1 > at { found = 1 } END { exit !found }'; then
    printf '[approach %02d] no step made ahead: the line walks on elsewhere\n' "$1"
    return 0
  fi
  # The next step is made while the player looks at this one, the one that goes right up to it
  # too: shown only if they go on.
  explore_start_ahead
}
# A move by one of the scenario's ways ($2), said as $3: the hero says the line (once, by the first
# step that says it), then the move is designed, painted and walked (`move act to=`) where the
# intent judge read the step to go: the one reading of where it goes, never read again.
explore_way() { # number, way, line
  local out plate situation
  explore_end_approach
  stop_watch quiet
  plate=$(printf 'explore-%s-%02d%s' "$STAMP" "$1" "${EXPLORE_STEP_AT:+-t$EXPLORE_STEP_AT}")
  # A move into something of the scene (explore_enter) has no way: its own words say where.
  situation="$ROAM_GOAL ${2:+${WAY[$2]}}${here:+ You are at $here now.}"
  situation="$situation${visited:+ Places you have been: $visited.}"
  explore_say "$1" "" "$3" "$situation" || return 0
  roam_move_act "$1" "$ROAM" "plate=$plate" $([ -n "$2" ] && roam_walk_options "$2") \
    "to=${ENTER_TO:-${INTENT_TOWARD:-${INTENT_NAMED:-the $2}}}" "text=$HERO_SAID" "party=$ROAM" \
    "$situation"
  out=$MOVE_OUT
  roam_walk_lines "$1" "$out"
  if ! printf '%s\n' "$out" | grep -q '^TRANSITION .*stage=live'; then
    printf '[move %02d] %s did not go live: %s\n' "$1" "$2" "$(printf '%s\n' "$out" | tail -1)"
    explore_restore_place "$1"
    explore_move_failed "$1" "The way there didn't open this time. Try again."
    return 0
  fi
  explore_here "$plate" "$out" "$2"
  printf '[move %02d] way=%s place=%s plate=%s things=%s\n' "$1" "$2" "$HERE_NAME" "$plate" \
    "$(printf '%s\n' "$HERE_THINGS" | grep -c '^thing ')"
  # Whom the line was put to answers it on arrival, once: a later step of the line answers nothing.
  [ -z "$EXPLORE_STEP_AT" ] && seat_reply "$1" "$SAID_TO" "$ROAM" "$HERO_SAID"
  roam_tempo "$1" arrival "$2"
  explore_watch
  # What the hero's eyes find on the plate becomes things of the place (a footbridge, the
  # stream), each with a look card.
  explore_tell_here "$1" "${INTENT_WORDS:-$HERO_SAID}"
}
# The dungeon master tells where the hero is now, from the plate and what the hero's eyes find on
# it (the place's own things and people, as a look is told).
explore_tell_here() { # number, deed
  EXPLORE_SEEN=""
  explore_perceive "$1" "$HERO_SAID"
  explore_narrate "$1" look "$2"
}
# A move back to a place the hero has been ($2, its plate, as the intent judge read the line from
# the places offered it): the line is said once, by the first step that says it, and the place is
# shown again as it was left (roam_revisit: the host's active place again, its plate, its things),
# with no design or plate made. A place the host cannot make active again leaves the hero where
# they are.
explore_revisit() { # number, plate, line
  local situation
  explore_end_approach
  stop_watch quiet
  situation="$ROAM_GOAL You go back to $(place_said "${VISIT_NAME[$2]}")."
  explore_say "$1" "" "$3" "$situation" || return 0
  roam_revisit "$1" "$2" "$situation" said
  if [ -z "$REVISIT_RESTORED" ]; then
    explore_restore_place "$1"
    seat_notice "$1" "$ROAM_PLAYER" "The way back didn't open this time. Try again."
    explore_watch
    return 0
  fi
  explore_here "$2" "" revisit
  # Back beside a thing an approach reached: the plate is its close-up again.
  [ "${VISIT_WAY[$2]}" = reached ] && REACHED_TARGET=${VISIT_TARGET[$2]} REACHED_PLATE=$2
  explore_watch
  explore_tell_here "$1" "$HERO_SAID"
}
# A move into a thing of the scene ($2: a building, a doorway, a gate): its inside is a place of
# its own, designed from what the thing is (a move with no way, to its inside), with its plate and
# arrival; the place outside is kept as one to go back out to (a revisit).
ENTER_TO=""
explore_enter() { # number, thing, line
  local name look
  name=$(explore_words "$2" "")
  case "${THING_STATE[$2]:-}" in
    closed | locked)
      printf '[enter %02d] the %s is %s: not gone into\n' "$1" "$name" "${THING_STATE[$2]}"
      EXPLORE_MOVE_FAILED=yes
      seat_notice "$1" "$ROAM_PLAYER" "The $name is ${THING_STATE[$2]}."
      return 0 ;;
  esac
  look=$(thing_look "$2")
  VISIT_WAY[$HERE_PLATE]=outside
  VISIT_NAME[$HERE_PLATE]="outside the $name"
  printf '[enter %02d] into the %s [%s]; outside is %s\n' "$1" "$name" "$2" "$HERE_PLATE"
  ENTER_TO="the inside of the $name${look:+, $look}"
  explore_way "$1" "" "$3"
  ENTER_TO=""
}
# The state act $1 left thing $2 in (the judge's `sets`): what the deed did when it worked; a
# failed opening leaves it shut. Kept on the thing as its state note (`world thing note`, never
# over its look or its other notes) and by the runner, so the place, the dungeon master and a way
# in through it all agree.
declare -A THING_STATE
explore_set_state() { # number, thing
  local state=$INTENT_SETS renamed="" out
  [ -n "$EXPLORE_STEP_FAILED" ] && { [ "$state" = open ] && state=closed || return 0; }
  THING_STATE[$2]=$state
  # A name that told the state it had is the one Jev gives for the new state (the open timber
  # gate, shut: timber gate), so "the timber gate" is still found as this thing.
  [ "$state" = "$INTENT_SETS" ] && renamed=$INTENT_RENAMED
  out=$($D dev --port $P world thing note "$2" state $renamed '|' "It stands $state now." 2>&1)
  printf '[state %02d] thing=%s state=%s name=%s: %s\n' "$1" "$2" "$state" \
    "${renamed:-$(explore_words "$2" "")}" "$(printf '%s\n' "$out" | head -1)"
  explore_scene
}
# A plate of the place as it is, from the same spot, the hero now standing as pose $2 says
# (`move approach` step 0 with pose=, an edit of the plate on screen), checked by the host and
# painted again once when the figure does not take it. Logs `POSE plate= pose= ms= checked=`.
explore_pose() { # number, pose
  local plate began out status checked shown view things
  plate=$(printf 'explore-%s-%02d-pose%s' "$STAMP" "$1" "${EXPLORE_STEP_AT:+-$EXPLORE_STEP_AT}")
  began=$(date +%s%3N)
  stop_watch quiet
  # The same view from the same spot: the design is told what the plate on screen shows and the
  # things in sight there, so nothing of it (the gate and walls behind) is dropped.
  things=$(printf '%s\n' "$SCENE_THINGS" | cut -f1 | paste -sd ',' - | sed 's/,/, /g')
  view="the very view on screen now, unchanged: ${HERE_PICTURE:-$HERE_NAME}${things:+; in sight: $things}"
  out=$(timeout $EXPLORE_POSE_WAIT $D dev --port $P move approach "$ROAM" target=ahead \
    "toward=$view" distance=within_reach step=0 "from=$HERE_PLATE" "plate=$plate" "pose=$2" 2>&1)
  status=$?
  # The host checks the painted figure takes the stance (a closed vision judge) and paints it
  # once more when not (`approach pose_check .. checked= shows= repainted=`).
  checked=$(printf '%s\n' "$out" | sed -n 's/^approach pose_check plate=[^ ]* //p' | head -1)
  printf '[pose %02d] POSE plate=%s pose=%s ms=%s %s\n' "$1" "$plate" "$2" \
    "$(($(date +%s%3N) - began))" "${checked:-checked=no repainted=0}"
  printf '%s\n' "$out" | grep -E '^approach (pose_seen|pose_unchecked|pose_repaint_failed) ' |
    sed "s/^/[pose $(printf '%02d' "$1")] /"
  if ! printf '%s\n' "$out" | grep -q '^approach shown '; then
    if [ "$status" -eq 124 ]; then
      printf '[pose %02d] no pose plate: not made within %s s\n' "$1" "$EXPLORE_POSE_WAIT"
    else
      printf '[pose %02d] no pose plate (exit %s): %s\n' "$1" "$status" \
        "$(printf '%s\n' "$out" | grep -v '^ *$' | tail -1)"
    fi
    explore_watch
    return 1
  fi
  # Painted again, the plate shown is the second one.
  shown=$(printf '%s\n' "$out" | sed -n 's/^approach shown .* plate=\([^ ]*\) .*/\1/p' | head -1)
  plate=${shown:-$plate}
  explore_carry_ground "$1" "$HERE_PLATE" "$plate"
  explore_here "$plate" "$out" pose
  printf '%s\n' "$out" | grep '^approach same_spot ' | sed "s/^/[pose $(printf '%02d' "$1")] /"
  explore_watch
}
# Each plate is a place of its own, and what a line left lying (a thrown dagger) is on the ground
# of the place it was left at (`ground-<place>`). A plate of the same spot (a pose) takes all of it
# along; a step toward a thing lying there ($4, "I walk over to my dagger") takes that thing, so it
# lies where the hero now stands and can be picked up. Prints `[ground NN] carried ..`.
explore_carry_ground() { # number, from plate, to plate, [item]
  local from to item count out empty
  from="ground-$(place_id "$2")"
  to="ground-$(place_id "$3")"
  out=$($D dev --port $P world inventory get "$from" 2>/dev/null) || return 0
  empty="$(dirname "$FRAMES_LOG")/ground-empty.json"
  while IFS== read -r item count; do
    [ -n "$item" ] && [ "${count:-0}" -gt 0 ] || continue
    [ -z "$4" ] || [ "$item" = "$4" ] || continue
    if ! $D dev --port $P world inventory get "$to" >/dev/null 2>&1; then
      [ -s "$empty" ] || printf '[]\n' > "$empty"
      $D dev --port $P world inventory set "$to" container "$empty" >/dev/null 2>&1
    fi
    printf '[ground %02d] carried %s x%s %s -> %s: %s\n' "$1" "$item" "$count" "$from" "$to" \
      "$($D dev --port $P world inventory transfer "$from" "$to" "$item" "$count" \
        "why=lies where the hero now stands" 2>&1 | head -1)"
  done < <(printf '%s\n' "$out" | sed -n 's/^  \([^ ]*\) x\([0-9]*\): .*$/\1=\2/p')
}
# A step of an approach where the hero did something to thing $1 of it ("I climb the well's rim")
# is offered to the judge as a place to go back to, by that thing ("by the stone well").
explore_mark_stop() { # thing
  [ -n "$1" ] && [ "${VISIT_WAY[$HERE_PLATE]:-}" = approach ] || return 0
  printf '%s\n' "$SCENE_THINGS" | awk -F'\t' -v id="$1" '$2 == id' | grep -q . || return 0
  VISIT_WAY[$HERE_PLATE]=stop
  VISIT_NAME[$HERE_PLATE]="by the $(explore_words "$1" "")"
}
# A move that did not land may have left the host at another place: the place on screen is made
# the active one again, so the scene the cards, the intent judge and the narrator read is the one
# the TV shows.
explore_restore_place() { # number
  local out
  out=$($D dev --port $P world place "$(place_id "$HERE_PLATE")" 2>&1)
  printf '[move %02d] the place on screen is the active one again: %s\n' "$1" \
    "$(printf '%s\n' "$out" | head -1)"
}
# A look without moving ($3 said): at a thing of the place ($2), its close-up insert; a way no id
# names ($4 its direction, $5 what lies there), a turn to face it (explore_turn). Every look is then
# told by the dungeon master from the scene and from what the hero's eyes find on the plate
# (explore_perceive, explore_narrate): a look always answers, and never denies what the picture
# shows.
explore_look() { # number, target, line, direction, toward
  local seq line near=""
  # The approach's target within reach: its close-up was made ahead, and looking at it takes it.
  if [ -n "$2" ] && [ "$2" = "$APPROACH_TARGET" ] && [ "$APPROACH_NEXT_REACHES" = true ] &&
    [ -n "$AHEAD_PID" ]; then
    explore_approach "$1" "$APPROACH_TARGET" "$APPROACH_TOWARD" "$APPROACH_DISTANCE" "$3"
    return 0
  fi
  seq=$(latest_event)
  explore_say "$1" "" "$3" "You look around you." || return 0
  if [ -n "$2" ] && [ "$2" = "$REACHED_TARGET" ] && [ "$HERE_PLATE" = "$REACHED_PLATE" ]; then
    # The plate on screen is already this thing up close (the step that reached it): no insert
    # is waited for, the dungeon master tells the look from it.
    printf '[look %02d] %s is up close on screen already (plate %s): no insert\n' "$1" "$2" \
      "$HERE_PLATE"
    near="the $(explore_words "$2" "") right before you, up close"
  elif [ -n "$2" ] && explore_thing_lines | grep -q "^thing $2 ("; then
    watch_look "$ROAM" "$2" "$HERO_SAID"
    line=$(roam_insert "${seq:-0}" "$EXPLORE_FRAMES" "$1" "$(explore_thing_lines)" card "$2")
    printf '[look %02d] %s\n' "$1" "$line"
    case "$line" in *' ended_at_ms='*) roam_tempo "$1" look ;; esac
  elif [ -n "$4" ]; then
    explore_turn "$1" "$4" "$5"
    EXPLORE_SEEN=""
  fi
  # The words are grounded in the picture itself, never the scene's list alone.
  [ -n "$EXPLORE_SEEN" ] || explore_perceive "$1" "$HERO_SAID"
  # At its close-up the hero stands right before it: what is on it can be made out.
  [ -n "$near" ] && EXPLORE_SEEN="$near"$'\n'"$EXPLORE_SEEN"
  local reads=()
  [ "$INTENT_READS" = true ] && reads=(reads=true)
  explore_narrate "$1" look "${INTENT_WORDS:-$HERO_SAID}" "" "${reads[@]}"
  # What a thing reads, once told, is kept on it: it reads the same the next time.
  [ "$INTENT_READS" = true ] && [ -n "$2" ] && [ -n "$NARRATED" ] &&
    explore_keep_reading "$1" "$2" Read
}
# Keeps what the dungeon master told of thing $2 on it (`world thing add` under its id replaces it,
# its look now with what it reads, or what is inside it, $3 the label), once per place and label:
# it is the same the next time, and walking in later matches what was seen.
declare -A READ_KEPT
explore_keep_reading() { # number, thing, [label: Read|Inside]
  local out label=${3:-Read}
  [ -z "${READ_KEPT[$HERE_PLATE:$2:$label]:-}" ] || return 0
  out=$($D dev --port $P world thing note "$2" "$(printf '%s' "$label" | tr 'A-Z' 'a-z')" '|' \
    "$label: $NARRATED" 2>&1)
  READ_KEPT[$HERE_PLATE:$2:$label]=yes
  printf '[look %02d] %s kept on %s: %s\n' "$1" "$label" "$2" "$(printf '%s\n' "$out" | head -1)"
}
# A look into thing $2 of the scene (a window, an open door): the dungeon master tells what is
# inside, true to what it is (never a search of the picture outside), and it is kept on the thing,
# so going in later is designed to match it (explore_enter takes the thing's look).
explore_look_into() { # number, thing (or none), line, [words: what is looked into, with no thing]
  local name look=""
  explore_say "$1" "" "$3" "You look inside." || return 0
  name=$(explore_words "$2" "${4:-}")
  [ -n "$2" ] && look=$(thing_look "$2")
  EXPLORE_SEEN=""
  explore_narrate "$1" look "${INTENT_WORDS:-$HERO_SAID}" "" "looks_into=the $name${look:+ ($look)}"
  [ -n "$NARRATED" ] && [ -n "$2" ] && explore_keep_reading "$1" "$2" Inside
}
# The line $3 said to $2, a person on the plate (or to whoever is there), who answers.
explore_talk() { # number, person, line
  local who answering=() earlier="" answered=""
  explore_say "$1" "$2" "$3" "You talk with who is here." || return 0
  # Said to whoever is there ($2 empty): the addressee judge reads whom it is put to. The one
  # answering is told what actually changed hands as it was said (a gold piece for a silver
  # one named), so the answer follows it. Said to several (Jev's addressees: you both,
  # everyone), each answers in turn, in Jev's order.
  ANSWERED=""
  IFS=, read -ra answering <<< "$INTENT_ADDRESSEES"
  [ "${#answering[@]}" -gt 0 ] || answering=("$2")
  [ "${#answering[@]}" -gt 1 ] &&
    printf '[talk %02d] said to %s: each answers in turn\n' "$1" "$INTENT_ADDRESSEES"
  # Each one answering after another is told what was just answered to the same words (host 57:
  # the patron sent the hero to ask the barkeep his name right after he gave it).
  for who in "${answering[@]}"; do
    seat_answer "$1" "$ROAM_PLAYER" "$who" "${SPOKEN:-$HERO_SAID}" "" \
      "$(explore_answering "${SPOKEN:-$HERO_SAID}")${earlier:+ Just now, to the same words: $earlier}"
    if [ -n "$ANSWERED" ]; then
      answered=$ANSWERED
      earlier="${earlier:+$earlier }$(explore_words "$ANSWERED" "") said: \"$ANSWER_LINE\"."
    fi
  done
  ANSWERED=$answered
  # Words nobody there answered (a call into an empty lane): the dungeon master tells what
  # happens, from the place and its people (someone may yet answer, or no one).
  if [ -z "$ANSWERED" ]; then
    explore_narrate "$1" act "${INTENT_WORDS:-$HERO_SAID}" "" unanswered=true
  fi
}
# What one answering a line said to them is told: to answer that very line (never one before it),
# what changed hands as it was said (a gold piece for a silver one named), and that they live here
# and know their town (asked the way to a place in it, they name one, true to the town, never
# themselves).
explore_answering() { # line
  printf 'Answer what the hero has just said to you: "%s". %sYou live here and know this town: \
asked the way to a place in it, name one that fits the town and say how to get there; never send \
them to yourself.' "$1" "${SAID_CHANGES:+What changed hands as it was said: $SAID_CHANGES. }"
}
# The line $3 said to $2, someone who sells: the deal path judges what changes hands in both lines
# (the hero's put to the seller, `from=`, and the answer).
explore_trade() { # number, seller, line
  local before after
  before=$(held_counts)
  roam_tempo "$1" trade
  explore_talk "$1" "$2" "$3"
  after=$(held_counts)
  printf '[trade %02d] with %s: %s; gained: %s\n' "$1" "$2" "$(held_diff "$before" "$after")" \
    "$(held_gains "$before" "$after")"
}
# An action in place ($3): the hero says the deed first (`expect=attempt`: it moves nothing yet),
# then the check judge calls a check when the rules need one (the dice, the call and the result are
# the host's, `persona check run` with the hero's sheet), then what the deed got or used is judged
# as finished work told how the roll came out (`expect=said_inventory`, explore_changes), so a
# failed deed gains nothing, and the dungeon master tells what changed hands. A required check that
# did not resolve (it failed, or it was cut off) resolves nothing: the player is told and the line
# is offered back as a card.
explore_act() { # number, target, line
  local out header outcome result came required reason="" deed=${INTENT_WORDS:-$3}
  explore_say "$1" "" "$3" "You act." attempt || return 0
  wait_heard
  if [ -n "$EXPLORE_SHEET" ]; then
    out=$(timeout $HUNG $D dev --port $P persona check run "$ROAM" "sheet=$EXPLORE_SHEET" \
      "action=$deed" "told=$ROAM: $deed" "place=${here:-the open road}" 2>&1)
    header=$(printf '%s\n' "$out" | head -1)
    printf '[act %02d] %s\n' "$1" "$header"
    printf '%s\n' "$out" | sed -n 's/^ *\(roll\|called ([^)]*)\|result ([^)]*)\): /\1: /p' |
      sed "s/^/[act $(printf '%02d' "$1")] /"
    outcome=$(printf '%s\n' "$out" | sed -n 's/^ *roll: .* outcome=\([a-z_]*\).*/\1/p' | head -1)
    result=$(printf '%s\n' "$out" | sed -n 's/^ *result ([^)]*): //p' | head -1)
    required=$(printf '%s\n' "$header" | sed -n 's/.* required=\([a-z]*\).*/\1/p')
    reason=$(printf '%s\n' "$header" | sed -n 's/.* reason=\([a-z_]*\).*/\1/p')
    if [ "$required" != false ] && [ -z "$outcome" ]; then
      printf '[act %02d] the check did not resolve: nothing comes of the deed: %s\n' "$1" \
        "$(printf '%s\n' "$out" | tail -1)"
      EXPLORE_NOTE=$EXPLORE_UNROLLED
      EXPLORE_RETRY=$3
      EXPLORE_STEP_FAILED=yes
      return 0
    fi
  else
    printf '[act %02d] no check judged: the hero has no rules sheet\n' "$1"
  fi
  # A deed the rules say cannot be done (a spell the hero's sheet has not in play yet) does not
  # happen: the player is told so plainly, and nothing changes hands.
  if [ "$reason" = impossible_no_check ]; then
    EXPLORE_STEP_FAILED=yes
    printf '[act %02d] cannot be done by the rules: nothing comes of it\n' "$1"
    explore_narrate "$1" act "$HERO_SAID" "" impossible=true
    return 0
  fi
  case "$outcome" in failure | miss) EXPLORE_STEP_FAILED=yes ;; esac
  came=""
  if [ -n "$outcome" ]; then
    came="The deed was rolled for: $outcome. ${result:+$result }"
    came="${came}Only what the deed truly got or used changes hands."
  fi
  explore_changes "$1" "$3" "$came"
  printf '[act %02d] changed: %s\n' "$1" "${SAID_CHANGES:-nothing}"
  # The roll's call and result are the dungeon master's own lines and name no gain; what the deed
  # got or used is told after them, from what changed.
  local told="" fields=()
  [ -n "$outcome" ] && told="The roll came out: $outcome. "
  [ -n "$SAID_CHANGES" ] && told="${told}What changed hands: $SAID_CHANGES."
  # A line no reading stood for is played as an act in place: the hero goes nowhere in it.
  [ "$INTENT_FALLBACK" = true ] && fields+=(stays=true)
  # A door opened (or locked, broken, shut) keeps that state on the thing; a failed opening leaves
  # it shut. The hero's new stance gets its own plate of the same view before the line tells it.
  [ -n "$INTENT_SETS" ] && [ -n "$2" ] && explore_set_state "$1" "$2"
  # A held stance is told first and painted after, so the dungeon master's line is read while the
  # picture paints.
  local posing=""
  [ -n "$INTENT_POSE" ] && [ -z "$EXPLORE_STEP_FAILED" ] && posing=$INTENT_POSE
  [ -n "$posing" ] && fields+=("stance=$posing")
  explore_narrate "$1" act "${INTENT_WORDS:-$HERO_SAID}" "$told" "${fields[@]}"
  # Done to someone there, to their things or before them (a punch, a theft off their stall):
  # they answer it in character, told how it went and that it was done before their eyes.
  local who=""
  if [ -n "$2" ] &&
    printf '%s\n' "$SCENE_PEOPLE" | awk -F'\t' -v id="$2" '$2 == id' | grep -q .; then
    who=$2
  fi
  who=${who:-$INTENT_AFFECTS}
  if [ -n "$who" ]; then
    local done=${INTENT_WORDS:-$HERO_SAID}
    local seen="The hero just did this, before your eyes: $done.${outcome:+ How it went: \
$outcome.} ${result:+$result }Answer this, what was just done, as you would; never anything said or \
done before it."
    # What the one answering is given, word for word, so a stale answer can be read back.
    printf '[react %02d] %s told the deed: "%s" | situation: "%s"\n' "$1" "$who" "$done" "$seen"
    seat_reply "$1" "$who" "$ROAM" "$done" "$seen"
  fi
  [ -z "$posing" ] || explore_pose "$1" "$posing" || true
}
# What the deed $2, said already, got or used of what the hero carries, judged now as finished
# work told what came of it ($3; `persona say expect=said_inventory`: nothing is shown or voiced
# again). Sets SAID_CHANGES.
explore_changes() { # number, line, what came of it
  local out
  recent_args
  out=$(timeout $TEXT_WAIT $D dev --port $P persona say "$ROAM" expect=said_inventory "text=$2" \
    ${3:+"intent=$3"} "${RECENT[@]}" "${here:+You are at $here. }You act." 2>&1)
  SAID_CHANGES=$(said_changes "$out")
  printf '%s\n' "$out" | grep -q 'says: ' ||
    printf '[act %02d] <changes not judged: %s>\n' "$1" "$(printf '%s\n' "$out" | head -1)"
  roam_inventory "$2" "$out"
}
# The hero's rules sheet for the check judge (`creation sheet`), once.
explore_sheet() {
  local file out
  file="$(dirname "$FRAMES_LOG")/sheet-$ROAM.json"
  mkdir -p "$(dirname "$file")"
  if out=$($D dev --port $P creation sheet "$ROAM_PLAYER" "out=$file" 2>&1) && [ -s "$file" ]; then
    EXPLORE_SHEET=$file
    printf '[prepare] sheet of seat %s: %s\n' "$ROAM_PLAYER" "$(printf '%s\n' "$out" | head -1)"
  else
    printf '[prepare] no rules sheet, actions go unchecked: %s\n' \
      "$(printf '%s\n' "$out" | head -1)"
  fi
}
# The opening plate has no place design: it is made the active place (`world arrive`, named
# `@roam_start_place`, its words the plate's) and what the hero sees on it, the picture itself
# (`persona perceive .. mode=study`, vision), is added to it as its things, the one scene the
# intent judge, the cards and the narrator read. Prints `[prepare] start scene ..`.
explore_start_scene() {
  local out seen
  HERE_NAME=$ROAM_START_PLACE
  here=$HERE_NAME
  out=$($D dev --port $P world arrive "$(place_id "$HERE_PLATE")" \
    $ROAM_START_PLACE '|' "$ROAM_START_PLATE" 2>&1)
  printf '[prepare] start place: %s\n' "$(printf '%s\n' "$out" | head -1)"
  explore_perceive 0 ""
  seen=$EXPLORE_THING_LINES
  printf '[prepare] start scene: %s thing(s) seen on the plate\n' \
    "$(printf '%s\n' "$seen" | grep -c '^thing ')"
  printf '%s\n' "$seen" | sed 's/^/[prepare]   /'
  # Kept to go back to like any place the hero has been, under the host's id of it.
  visited=$HERE_NAME
  roam_keep_place "$(place_id "$HERE_PLATE")" "$HERE_NAME" "$HERE_PICTURE" \
    "$seen" "" start
}
# The host's place id of plate $1: the plate's name in lower case (`world arrive` keeps it so).
place_id() { # plate
  printf '%s' "$1" | tr 'A-Z' 'a-z'
}
# The turn's intent to its handler (returns non-zero when the player is done). A look or a move
# toward something the line names that the scene has no id for yet (`toward` alone) first
# searches the plate itself for it (explore_find: the hero's perception of the picture adds what
# it shows to the place) and the line is read again against the scene as it is then ($3 `second`);
# what the picture still does not show is told by the dungeon master, from what the hero sees.
explore_dispatch() { # number, line, first|second, [follow-on step from 1]
  local named=""
  # Something named that the scene has no id for, searched for on the plate: the reading gives its
  # noun only then (the host's searched_for).
  [ -n "$INTENT_NOUN" ] && named=yes
  if [ -n "$named" ] && [ "$3" = first ]; then
    case "$INTENT_KIND" in
      look | move | act | ask)
        # The first step's search reads the line again; a follow-on step's binds that step alone
        # to what the search added (its own kind and noun), and is never read as the line.
        explore_find "$1" "$2" "$([ -n "${4:-}" ] && printf step)"
        explore_dispatch "$1" "$2" second "${4:-}"
        return ;;
    esac
  fi
  case "$INTENT_KIND" in
    move)
      if [ "$INTENT_CONTINUE" = true ] && [ -n "$APPROACH_TARGET" ]; then
        # Keep walking: the same approach, its next step (the one made ahead when it is ready).
        explore_approach "$1" "$APPROACH_TARGET" "$APPROACH_TOWARD" "$APPROACH_DISTANCE" "$2"
      elif [ -n "$INTENT_REVISIT" ]; then
        explore_revisit "$1" "$INTENT_REVISIT" "$2"
      elif [ -n "$INTENT_ENTER" ]; then
        explore_enter "$1" "$INTENT_ENTER" "$2"
      elif [ -n "$INTENT_WAY" ]; then
        explore_way "$1" "$INTENT_WAY" "$2"
      elif [ -n "$named" ] && [ -n "$EXPLORE_BLIND" ]; then
        seat_notice "$1" "$ROAM_PLAYER" "You can't make that out from here just now. Try again."
      elif [ -n "$named" ]; then
        # Not found on the plate (a house the search could not pick out): the hero sets off that
        # way toward it, a step ahead; never a move that goes nowhere.
        printf '[find %02d] %s: not picked out on the plate, walking on toward it\n' "$1" \
          "$INTENT_TOWARD"
        explore_approach "$1" ahead "$(explore_words "" "$INTENT_TOWARD")" \
          "${INTENT_DISTANCE:-down_the_road}" "$2" direction
      else
        explore_approach "$1" "${INTENT_TARGET:-$INTENT_DIRECTION}" \
          "$(explore_words "${INTENT_TARGET:-}" "$INTENT_TOWARD")" "$INTENT_DISTANCE" "$2" \
          "$([ -n "$INTENT_TARGET" ] || printf direction)"
      fi ;;
    look)
      if [ -n "$INTENT_ENTER" ]; then
        explore_look_into "$1" "$INTENT_ENTER" "$2"
      elif [ "$INTENT_INTO" = true ]; then
        # A look into a building the scene has no thing for (or the thing the search added):
        # still what is inside is told, never the outside seen again (host 57).
        explore_look_into "$1" "$INTENT_TARGET" "$2" "${INTENT_NAMED:-$INTENT_TOWARD}"
      elif [ -n "$named" ] && [ -n "$EXPLORE_BLIND" ]; then
        # Neither the search nor the eyes could look: nothing is told of what was not seen.
        seat_notice "$1" "$ROAM_PLAYER" "You can't make that out from here just now. Try again."
      else
        explore_mark_stop "$INTENT_TARGET"
        explore_look "$1" "$INTENT_TARGET" "$2" "$INTENT_DIRECTION" \
          "$(explore_words "" "$INTENT_TOWARD")"
      fi ;;
    talk) explore_talk "$1" "$INTENT_TARGET" "$2" ;;
    trade) explore_trade "$1" "$INTENT_TARGET" "$2" ;;
    act)
      # An act on what the line is walking toward is done once it is reached: the approach goes
      # on step by step up to EXPLORE_REACH_STEPS, else the player is told how far it still is.
      if [ -n "$INTENT_TARGET" ] && [ "$INTENT_TARGET" = "$APPROACH_TARGET" ] &&
        [ -z "$APPROACH_DIRECTED" ] && [ "$APPROACH_AWAY" != true ]; then
        local walked=0 target=$INTENT_TARGET
        while [ "$APPROACH_TARGET" = "$target" ] && [ "$walked" -lt "$EXPLORE_REACH_STEPS" ]; do
          explore_approach "$1" "$APPROACH_TARGET" "$APPROACH_TOWARD" "$APPROACH_DISTANCE" "$2"
          walked=$((walked + 1))
        done
        if [ "$APPROACH_TARGET" = "$target" ]; then
          printf '[act %02d] %s not reached after %s steps: not done\n' "$1" "$target" "$walked"
          seat_notice "$1" "$ROAM_PLAYER" \
            "You're not there yet: the $(explore_words "$target" "") is still ${APPROACH_DISTANCE//_/ }."
          EXPLORE_STEP_FAILED=yes
          return 0
        fi
        INTENT_TARGET=$target
      fi
      explore_mark_stop "$INTENT_TARGET"
      explore_act "$1" "$INTENT_TARGET" "$2" ;;
    # A question to the game master: answered from the scene and the plate, never acted on.
    ask)
      explore_perceive "$1" "$2"
      explore_narrate "$1" answer "$2" ;;
    done)
      if ! explore_confirm_done "$1"; then
        # Words typed at the confirm card instead are a turn of their own, carried out as read.
        [ -n "$EXPLORE_INSTEAD" ] || return 0
        local instead=$EXPLORE_INSTEAD
        EXPLORE_INSTEAD="" EXPLORE_SAID="" EXPLORE_STEP_AT=""
        explore_dispatch "$1" "$instead" first || return 1
        explore_then "$1" "$instead" || return 1
        INTENT_OUT=""
        return 0
      fi
      explore_say "$1" "" "$2" "You end the day here." || true
      seat_notice "$1" "$ROAM_PLAYER" "The roam ends here."
      return 1 ;;
    clarify)
      # The question goes into the talk, so the answer to it is read with it.
      HISTORY+=("$ROAM: $2" "game master: $INTENT_QUESTION")
      EXPLORE_NOTE=$INTENT_QUESTION ;;
    *)
      # The judge was not reached (the text service failed or timed out): said plainly, and the
      # line kept as a card to send again.
      EXPLORE_NOTE=$EXPLORE_UNREACHED
      EXPLORE_RETRY=$2 ;;
  esac
  return 0
}
# Step $2 (from 1) of the reading $3 as the turn's intent (intent_load), printed as `[then NN]`.
explore_step_of() { # number, step, reading
  intent_load "$2" "$3"
  INTENT_CANDIDATES="" INTENT_QUESTION="" INTENT_CONTINUE="" INTENT_FALLBACK=""
  printf '[then %02d] step %s: kind=%s target=%s way=%s direction=%s distance=%s%s%s\n' "$1" \
    "$(($2 + 1))" "${INTENT_KIND:-?}" "${INTENT_TARGET:--}" "${INTENT_WAY:--}" \
    "${INTENT_DIRECTION:--}" "${INTENT_DISTANCE:--}" \
    "$([ -n "$INTENT_TOWARD" ] && printf ' toward="%s"' "$INTENT_TOWARD")" \
    "$([ -n "$INTENT_REVISIT" ] && printf ' revisit=%s' "$INTENT_REVISIT")"
}
# The follow-on steps of the turn's line, in order, after its first (a bounded plan of at most two
# more): each goes to its own handler, and the line is said once, by the first.
explore_then() { # number, line
  local reading=$INTENT_OUT count at
  count=$(($(intent_steps "$reading") - 1))
  for ((at = 1; at <= count; at++)); do
    EXPLORE_STEP_AT=$at
    explore_step_of "$1" "$at" "$reading"
    # A step the line makes depend on the one before (to see over the fence from up there) is
    # not done when that one failed, and the player is told.
    if [ "$INTENT_DEPENDS" = true ] && [ -n "$EXPLORE_STEP_FAILED" ]; then
      printf '[then %02d] step %s depends on the step before, which failed: not done\n' "$1" \
        "$((at + 1))"
      seat_notice "$1" "$ROAM_PLAYER" "That didn't work, so the rest of it can't be done."
      continue
    fi
    if [ -n "$EXPLORE_MOVE_FAILED" ]; then
      printf '[then %02d] step %s follows a move that did not happen: not done\n' "$1" \
        "$((at + 1))"
      seat_notice "$1" "$ROAM_PLAYER" "You didn't get there, so the rest of it can't be done."
      return 0
    fi
    EXPLORE_STEP_FAILED=""
    if [ -z "$INTENT_KIND" ]; then
      printf '[then %02d] step %s could not be read: told\n' "$1" "$((at + 1))"
      seat_notice "$1" "$ROAM_PLAYER" "I couldn't follow the rest of that. Say it again on its own."
      continue
    fi
    explore_dispatch "$1" "$2" first "$at" || return 1
  done
}
# The hero says the turn's line once, by the first step that says it (seat_say: to $2 when
# named, with the situation $3 and $4 `inventory` for a deed done); a later step of the same line
# goes on from what was said.
explore_say() { # number, to, line, situation, [inventory], [what came of it]
  if [ -n "$EXPLORE_SAID" ]; then
    HERO_SAID=$3
    return 0
  fi
  SAID_CHANGES=""
  SPOKEN=""
  # Only the words the hero speaks are said aloud (the judge's `spoken`): a deed alone is never
  # a speech bubble or a voice (owner rule).
  if [ -z "$INTENT_SPOKEN" ]; then
    printf '[said %02d] seat %s (%s): spoken=none | does: %s\n' "$1" "$ROAM_PLAYER" "$ROAM" "$3"
    HISTORY+=("$ROAM does: $3")
    HERO_SAID=$3
    return 0
  fi
  seat_say "$1" "$ROAM_PLAYER" "$2" "$INTENT_SPOKEN" "$4" "${5:-}" "${6:-}" || return 1
  # What was said is the spoken words; the deed is told and judged as the whole line.
  SPOKEN=$HERO_SAID
  HERO_SAID=$3
  EXPLORE_SAID=yes
}
# A bare done ends the roam only once the player confirms it ("End the roam?", `end-roam-yes` or
# `end-roam-no` by card; typed words are read again, and only a done again ends it: other words are
# left in EXPLORE_INSTEAD, read, to be carried out). Returns non-zero when the roam goes on.
EXPLORE_INSTEAD=""
explore_confirm_done() { # number
  EXPLORE_INSTEAD=""
  seat_line "$1" "$ROAM_PLAYER" "End the roam?" "" \
    "$(printf '%s\n' "Yes, end it | Yes, I'm done for today. | end-roam-yes" \
      "No, keep going | No, I'll keep going. | end-roam-no")" line
  case "$SAID_THING" in
    end-roam-yes) return 0 ;;
    end-roam-no)
      printf '[explore %02d] the roam goes on\n' "$1"
      return 1 ;;
  esac
  [ -n "$SAID_LINE" ] || return 1
  if [ "${#SAID_LINE}" -gt "$SAID_LINE_CHARS" ]; then
    EXPLORE_NOTE=$SAID_TOO_LONG
    return 1
  fi
  explore_intent "$1" "$SAID_LINE" ""
  [ "$INTENT_KIND" = done ] && return 0
  printf '[explore %02d] not ended: carried out as %s\n' "$1" "${INTENT_KIND:-unjudged}"
  EXPLORE_INSTEAD=$SAID_LINE
  return 1
}
# What the hero's own eyes find on the plate now, the line $2 just said (`persona perceive ..
# mode=study`: the picture itself, with vision): EXPLORE_SEEN gets one line per sight (the
# narrator's `sees=`), and what the picture shows that the place did not have is added to it as
# its things (the scene the intent judge and the cards read). `[see NN] ..`.
explore_perceive() { # number, line, [where to look, in words]
  local out added scene=${HERE_PICTURE}${3:+ $3}
  out=$(timeout $TEXT_WAIT $D dev --port $P persona perceive "$ROAM" mode=study \
    ${2:+"recent=$ROAM: $2"} ${scene:+"scene=$scene"} 2>&1)
  EXPLORE_SEEN=$(printf '%s\n' "$out" | sed -n 's/^see: \(.*\) | region=.*$/\1/p')
  EXPLORE_THING_LINES=$(printf '%s\n' "$out" | grep '^thing ')
  added=$(printf '%s\n' "$EXPLORE_THING_LINES" | sed -n 's/^thing \([^ ]*\) .*/\1/p' |
    paste -sd ',' -)
  # What the eyes settled on, and the things the picture added to the place (EXPLORE_ATTENTION,
  # EXPLORE_ADDED) and the words of each sight by the item it is (EXPLORE_SIGHTS: `<item> <what>`).
  EXPLORE_ATTENTION=$(printf '%s\n' "$out" | sed -n 's/^attention: //p' | head -1)
  EXPLORE_ADDED=$added
  EXPLORE_SIGHTS=$(printf '%s\n' "$out" |
    sed -n 's/^see: \(.*\) | region=[^ ]* item=\([^ ]*\) .*$/\2 \1/p')
  printf '[see %02d] %s sight(s)%s | attention: %s\n' "$1" \
    "$(printf '%s\n' "$EXPLORE_SEEN" | grep -c .)" "${added:+, added to the place: $added}" \
    "$EXPLORE_ATTENTION"
  [ -n "$EXPLORE_SEEN" ] ||
    printf '[see %02d] <no sight: %s>\n' "$1" "$(printf '%s\n' "$out" | head -1)"
}
# Searches the plate for what the line $2 names that the scene has no id for. First a grounded
# query: the segmentation looks for Jev's short noun of it on the picture (`media locate find`,
# the locate path's SAM, nothing stored); a region found means it is there, and it is added to the
# place under Jev's new id (`world thing add`). Then the hero's own eyes on the picture
# (explore_perceive: the narrator's grounding, and a second check that adds what it sees). The
# scene is read again and Jev reads the line again against it; only what neither finds may the
# dungeon master say is not there.
explore_find() { # number, line, [step: a follow-on step's search]
  local out regions began box where look seen same found="" noun=${INTENT_NOUN:-$INTENT_TOWARD}
  local id=$INTENT_NAMED_ID before=$SCENE_THINGS
  FIND_SAME=""
  printf '[find %02d] searching the plate for: %s\n' "$1" "${INTENT_TOWARD:-$noun}"
  began=$(date +%s%3N)
  out=$(timeout $TEXT_WAIT $D dev --port $P media locate find "$HERE_PLATE.png" "$noun" 2>&1)
  regions=$(printf '%s\n' "$out" | sed -n 's/^found regions=\([0-9]*\) .*/\1/p' | head -1)
  # The region the search is surest of, never merely the first (host 56: a corner crop).
  box=$(printf '%s\n' "$out" |
    sed -n 's/^region [0-9]*: box=\([0-9,]*\) score=\([0-9]*\).*/\2 \1/p' | sort -rn |
    head -1 | cut -d' ' -f2)
  printf '[find %02d] sam prompt=%s regions=%s ms=%s%s\n' "$1" "$noun" "${regions:-none}" \
    "$(($(date +%s%3N) - began))" \
    "$([ -n "$regions" ] || printf ' (%s)' "$(printf '%s\n' "$out" | head -1)")"
  if [ "${regions:-0}" -gt 0 ] && [ -n "$id" ] && [ -n "$box" ]; then
    # Found: the eyes look at that very region, told where it sits, so its look is the
    # picture's.
    # Where it sits in the picture is for the log only: the eyes get the box, the look the
    # eyes' words, and the dungeon master never a place in the picture ("at eye height").
    where=$(explore_where "$box")
    explore_perceive "$1" "$2" "The $noun is in the box $box (left,top,right,bottom, thousandths)."
    found=$box
    # The region found is the thing named, a thing of its own under Jev's new id (the sign, never
    # the gate it hangs on: host 49), unless the eyes added it under that very id; its look is the
    # eyes' only when they settled on it, else the line's words and where it is.
    if [[ ",$EXPLORE_ADDED," != *",$id,"* ]]; then
      look=$(printf '%s\n' "$EXPLORE_SIGHTS" | awk -v item="$id" '$1 == item {
        $1 = ""; sub(/^ /, ""); print; exit }')
      # The region may be a thing the place already has (the gate, shut since). That is only a
      # candidate: Jev's reading of the line again confirms it (target the very thing named),
      # else it is added as the new thing (host 56: a house's region was taken for a crate). A
      # follow-on step is never read again, so its region is always added.
      [ "${3:-}" = step ] || same=$(explore_same_thing "$box" "$id")
      if [ -n "$same" ]; then
        FIND_SAME=$same
        printf '[find %02d] region %s: perhaps the same thing as %s, asked\n' "$1" "$box" "$same"
      else
        explore_add_found "$1" "$box" "$id" "$noun" "$look" "$where"
      fi
    fi
  else
    explore_perceive "$1" "$2"
  fi
  # Neither the search nor the eyes could look (a provider down): nothing is denied, and the
  # player is told it cannot be made out now.
  if [ -z "$regions" ] && [ -z "$EXPLORE_SEEN" ]; then
    printf '[find %02d] neither the search nor the eyes could look\n' "$1"
    EXPLORE_BLIND=yes
  fi
  explore_scene
  if [ "${3:-}" = step ]; then
    # The step searched for what the judge named: the id it gave, when the scene has it now.
    if [ -n "$id" ] && printf '%s\n' "$SCENE_THINGS" | awk -F'\t' -v id="$id" '$2 == id' |
      grep -q .; then
      INTENT_TARGET=$id INTENT_TOWARD="" INTENT_NOUN=""
    fi
  else
    explore_intent "$1" "$2" "${SAID_THING:-$SAID_TO}"
    if [ -n "$FIND_SAME" ] && [ "$INTENT_TARGET" = "$FIND_SAME" ]; then
      THING_BOX[$HERE_PLATE:$FIND_SAME]=$found
      printf '[find %02d] region %s: the same thing as %s, confirmed by the reading\n' "$1" \
        "$found" "$FIND_SAME"
    elif [ -n "$FIND_SAME" ]; then
      # Not that thing: the region is the new one named, and the line is read against it.
      printf '[find %02d] region %s: not %s, added as new\n' "$1" "$found" "$FIND_SAME"
      FIND_SAME=""
      explore_add_found "$1" "$found" "$id" "$noun" "$look" "$where"
      explore_scene
      explore_intent "$1" "$2" "${SAID_THING:-$SAID_TO}"
    fi
  fi
  # The region found is the thing Jev's new reading names (its first step or a follow-on), when
  # it is one this search added.
  [ -n "$found" ] || return 0
  for seen in "$INTENT_TARGET" $(printf '%s\n' "$INTENT_OUT" | sed -n 's/^step=[1-9][0-9]* target=//p'); do
    [ -n "$seen" ] && [[ ",$EXPLORE_ADDED,$id,$FIND_SAME," == *",$seen,"* ]] || continue
    printf '[find %02d] region %s: %s, %s\n' "$1" "$found" "$seen" "$where"
    break
  done
}
# Adds the region $2 found as the new thing $3 ($4 its noun, $5 the eyes' words for it, $6 where it
# sits, for the log).
explore_add_found() { # number, box, id, noun, look, where
  local out
  out=$($D dev --port $P world thing add "$3" $4 '|' "${5:-${INTENT_TOWARD:-$4}}" 2>&1)
  THING_BOX[$HERE_PLATE:$3]=$2
  printf '[find %02d] region %s: added %s (%s), %s: %s\n' "$1" "$2" "$3" "$4" "$6" \
    "$(printf '%s\n' "$out" | head -1)"
}
# The thing of the place that the region $1 found for new id $2 may already be, if any: the one the
# eyes settled on when they looked there (an id of the scene before the search, not $2), else one
# found here before whose box covers most of it (the overlap is at least half the smaller box).
declare -A THING_BOX
FIND_SAME=""
explore_same_thing() { # box, new id
  local item key l t r b L T R B w h small
  item=${EXPLORE_ATTENTION#item:}
  case "$item" in ahead | left | right | behind) item=$EXPLORE_ATTENTION ;; esac
  if [ "$item" != "$EXPLORE_ATTENTION" ] && [ "$item" != "$2" ] &&
    printf '%s\n' "$before" | awk -F'\t' -v id="$item" '$2 == id' | grep -q .; then
    printf '%s' "$item"
    return 0
  fi
  IFS=, read -r l t r b <<< "$1"
  for key in "${!THING_BOX[@]}"; do
    [ "${key%%:*}" = "$HERE_PLATE" ] && [ "${key#*:}" != "$2" ] || continue
    case "${key#*:}" in ahead | left | right | behind) continue ;; esac
    IFS=, read -r L T R B <<< "${THING_BOX[$key]}"
    w=$(( (r < R ? r : R) - (l > L ? l : L) ))
    h=$(( (b < B ? b : B) - (t > T ? t : T) ))
    [ "$w" -gt 0 ] && [ "$h" -gt 0 ] || continue
    small=$(( (r - l) * (b - t) ))
    [ $(( (R - L) * (B - T) )) -lt "$small" ] && small=$(( (R - L) * (B - T) ))
    if [ $(( 2 * w * h )) -ge "$small" ]; then
      printf '%s' "${key#*:}"
      return 0
    fi
  done
}
# Where the box $1 (`<left>,<top>,<right>,<bottom>` in thousandths of the picture) sits, in words
# for a look and a description: its middle across (left, middle, right) and down (high, low).
explore_where() { # box
  local left top right bottom across down
  IFS=, read -r left top right bottom <<< "$1"
  across=$(( (left + right) / 2 ))
  down=$(( (top + bottom) / 2 ))
  if [ "$across" -lt 333 ]; then across="on the left"
  elif [ "$across" -lt 667 ]; then across="in the middle"
  else across="on the right"; fi
  if [ "$down" -lt 333 ]; then down="high up"
  elif [ "$down" -lt 667 ]; then down="at eye height"
  else down="low down"; fi
  printf '%s %s of the view' "$down" "$across"
}
# The roam: one open ask a turn until the player is done.
play_explore() {
  local began=$SECONDS
  NUMBER=0
  EXPLORE_NOTE=""
  here=""
  visited=""
  STAMP=$(date +%m%d%H%M)
  if [ -z "$ROAM_PLAYER" ]; then
    printf '[explore] needs a phone seat (@roam_player or DF_RUN_PLAYER)\n'
    exit 2
  fi
  SEATS=("$ROAM_PLAYER")
  HERO_OF[$ROAM_PLAYER]=$ROAM
  # The roam's waits at human pace (see TEXT_WAIT): a text or vision call is seconds, a judge
  # asked up to 3 times at the 60 s a slow call took (dungeon run 13) is under 4 minutes; a plate
  # took 70-154 s (hosts 32-40), so a step gets twice the slowest; a way's move makes up to three
  # pictures in turn. A check keeps HUNG: it waits on the player's own roll on the phone, which has
  # no deadline (owner decision), and a check that does not resolve resolves nothing (explore_act).
  TEXT_WAIT=$EXPLORE_TEXT_WAIT
  MOVE_WAIT=$EXPLORE_MOVE_WAIT
  # A write to a child that has gone (the frame watch, a step made ahead) fails as a write, never
  # kills the roam.
  trap '' PIPE
  if [ -n "$REFS_MAX" ] &&
    timeout 30 $D dev --port $P persona watch </dev/null 2>&1 | grep -q 'refs_max'; then
    WATCH_REFS="refs_max=$REFS_MAX"
  fi
  $D dev --port $P world inventory show "$ROAM" | head -1 | sed 's/^/       ~ /'
  if ! roam_prepare_hero; then
    printf '[prepare] the roam does not start: the hero could not be made ready\n'
    exit 3
  fi
  if [ -n "$REFS_MAX" ] && [ -n "$WATCH_REFS" ]; then
    $D dev --port $P media backend "refs=$REFS_MAX" | head -1 | sed 's/^/       ~ /'
  fi
  explore_sheet
  roam_tempo_start
  HERE_PLATE="roam-00-$ROAM${DF_RUN_HERO_LOOK_KEY:+-$DF_RUN_HERO_LOOK_KEY}"
  HERE_PICTURE=$ROAM_START_PLATE
  explore_start_scene
  printf '[explore] %s from seat %s, %s ways, starting at %s\n' "$ROAM" "$ROAM_PLAYER" \
    "${#WAY[@]}" "$HERE_PLATE"
  explore_watch
  while :; do
    NUMBER=$((NUMBER + 1))
    caption_down
    seat_drain_talk "$NUMBER" pause
    explore_scene
    seat_line "$NUMBER" "$ROAM_PLAYER" "$EXPLORE_ASK" "$EXPLORE_NOTE" "$(explore_cards)" line
    EXPLORE_NOTE="" EXPLORE_RETRY=""
    [ -n "$SAID_LINE" ] || continue
    if [ "${#SAID_LINE}" -gt "$SAID_LINE_CHARS" ]; then
      printf '[ask %02d] the line is %s characters: asked again in fewer words\n' "$NUMBER" \
        "${#SAID_LINE}"
      EXPLORE_NOTE=$SAID_TOO_LONG
      continue
    fi
    EXPLORE_SEEN=""
    EXPLORE_SAID=""
    EXPLORE_BLIND=""
    EXPLORE_STEP_AT=""
    EXPLORE_STEP_FAILED=""
    EXPLORE_MOVE_FAILED=""
    explore_continue_card "$SAID_THING" ||
      explore_intent "$NUMBER" "$SAID_LINE" "${SAID_THING:-$SAID_TO}"
    explore_dispatch "$NUMBER" "$SAID_LINE" first || break
    explore_then "$NUMBER" "$SAID_LINE" || break
  done
  explore_end_approach
  stop_watch quiet
  explore_reap
  printf '[explore] ended after %s turns, %s s, at %s\n' "$NUMBER" "$((SECONDS - began))" \
    "${here:-the start}"
  [ -n "$EXPLORE_SHEET" ] && rm -f "$EXPLORE_SHEET"
}
# The words of what a move walks toward: the name of the thing or person of the scene with id $1,
# else Jev's words $2 (a direction's).
explore_words() { # id, words
  local name=""
  [ -n "$1" ] && name=$(printf '%s\n' "$SCENE_THINGS"$'\n'"$SCENE_PEOPLE" |
    awk -F'\t' -v id="$1" '$2 == id { print $1; exit }')
  name=${name:-${2:-$1}}
  # The cards say `the {thing}`: words that bring their own article drop it.
  case "$name" in
    'the '* | 'The '*) name=${name#* } ;;
  esac
  printf '%s' "$name"
}

# A party journey (`@journey <id>`): one or two seats' heroes travel stop by stop, each stop and
# what happens there written in the beats as ordered steps; nobody's words are guessed by the
# runner. The heroes come from the host (`DF_RUN_PLAYERS="1,2"` and per seat
# `DF_RUN_HERO_<n>_{ID,NAME,SPECIES,CLASS,SHEET,LOOK,LOOK_KEY,PERSONA}`; one seat runs from the
# single-seat `DF_RUN_PLAYER` and `DF_RUN_HERO_*`). Before the first step each hero gets its persona
# (`@journey_hero_hint`), kit sheets, rules sheet (`creation sheet`) and party card, the scenario's
# story is authored with the heroes as its players (`@journey_story`, `persona story author`: its
# evidence is the quest's clues, on show at their stops), and the opening plate is painted from
# every hero's reference sheet at once (`@journey_start_plate`, `refs=<sheet>,<sheet>`), scanned
# and every hero named in it. Every later plate is a move's (`move act`), whose party is the shown
# scene's heroes, so it is drawn after all of their sheets.
#
# The steps, in file order (`<who>` is `party`, a seat number, or `each` for every seat at once):
# - `@stop <id> | <way> | <who> | <prompt> | <where the move goes, for the design>`: the seat is
#   asked where to go (`@card <label> | <line>` lines after it are its cards), the hero says it and
#   it is played as a move (`move act text=`; the way's `@way` situation and `@walk` sights), the
#   plate kept as each hero's scene memory (tagged with the region) and the quest's mark moved on.
# - `@dm <line>`: the dungeon master says it (`persona narrate`).
# - `@ask <who> | <prompt>` and `@talk <who> | <prompt>` (with `@card`s): a seat's line, said by its
#   hero and answered by whom it is put to (the addressee judge, `persona addressed`); at a stop
#   that has a clue (`@clue <stop> <clue id>`) the clue judge reads it (`persona story clues`).
# - `@check <who> <kind:ability:skill|-> | <prompt>` (with `@card`s): a seat's line, then the check
#   judge sets the test from it and the host rolls from the hero's rules sheet (`persona check
#   run moment=player`); the second field is the test the stop was written for, printed beside the
#   judge's call, never sent.
# - `@shop <who> | <prompt>`: each seat is asked until it says it is done; meanwhile Buy on the
#   phone's Shop page is the host's, and talk (haggling) is taken as it comes.
# - `@cast <seat> | <prompt>`: the seat is asked and a cast from its Hero page is waited for (`cast`
#   in the inbox); what the spell shows is read by the clue judge.
# - `@quest <id> | <title> | <giver> | <reward_gp> | <advance_gp> | <site> | <clue,ids>` (a
#   definition) and `@quest_board <plate> | <quest ids> | <the board as a camera sees it>`: the
#   quests go on the board (`world quest offer`), the board is shown as a close-up handout, the
#   party is asked with one card per quest (its id the card's thing); typed words go to Jev
#   (`persona quest pick`), and the quest named is taken (`world quest accept`: the advance is
#   paid, the Notes and the TV's story mark follow).
# - `@riddle <id> | <the door's words> | <answer id> | <the answer> | <clue,ids> | <stalled|none>`:
#   the door asks (`persona riddle ask`), each line of the party is read by the clue judge (which
#   counts a stuck party) and judged by Jev against the answer (`persona riddle judge`); a stuck
#   party gets the narrator's hint (`persona story narrate <stop> stalled`) when the policy is
#   `stalled`; the step ends when it is solved.
# - `@teleport <region id> | <arrival place> | <the card's title> | <how the land looks>`: the region
#   changes (`world region enter`: the map, town and kept places start again, the title card goes
#   up); the next stop is designed there, and the card comes down once its plate has been read.
# The log: `[journey]`, `[prepare]`, `[stop NN]`, `[move NN]`, `[said NN]`, `[answer NN]`, `[clue
# NN]`, `[check NN]`, `[shop NN]`, `[cast NN]`, `[quest NN]`, `[riddle NN]`, `[region NN]`, `[dm
# NN]`. `--plan` prints every step and checks the beats against the story file with no host call.

# The seats of the journey and each one's hero, from the host's environment.
journey_seats() {
  local seat id name
  SEATS=()
  if [ -n "${DF_RUN_PLAYERS:-}" ]; then
    IFS=',' read -r -a SEATS <<< "${DF_RUN_PLAYERS// /}"
    for seat in "${SEATS[@]}"; do
      id=DF_RUN_HERO_${seat}_ID
      [ -n "${!id:-}" ] || return 1
      HERO_ID_OF[$seat]=${!id}
      name=DF_RUN_HERO_${seat}_NAME; HERO_NAME_OF[$seat]=${!name:-}
      name=DF_RUN_HERO_${seat}_SPECIES; HERO_SPECIES_OF[$seat]=${!name:-}
      name=DF_RUN_HERO_${seat}_CLASS; HERO_CLASS_OF[$seat]=${!name:-}
      name=DF_RUN_HERO_${seat}_SHEET; HERO_SHEET_OF[$seat]=${!name:-}
      name=DF_RUN_HERO_${seat}_LOOK; HERO_LOOK_OF[$seat]=${!name:-}
      name=DF_RUN_HERO_${seat}_LOOK_KEY; HERO_LOOK_KEY_OF[$seat]=${!name:-}
      name=DF_RUN_HERO_${seat}_PERSONA; HERO_OF[$seat]=${!name:-hero-${!id}}
    done
  elif [ -n "${DF_RUN_PLAYER:-}" ] && [ -n "${DF_RUN_HERO_ID:-}" ]; then
    seat=$DF_RUN_PLAYER
    SEATS=("$seat")
    HERO_ID_OF[$seat]=$DF_RUN_HERO_ID
    HERO_NAME_OF[$seat]=${DF_RUN_HERO_NAME:-}
    HERO_SPECIES_OF[$seat]=${DF_RUN_HERO_SPECIES:-}
    HERO_CLASS_OF[$seat]=${DF_RUN_HERO_CLASS:-}
    HERO_SHEET_OF[$seat]=${DF_RUN_HERO_SHEET:-}
    HERO_LOOK_OF[$seat]=${DF_RUN_HERO_LOOK:-}
    HERO_LOOK_KEY_OF[$seat]=${DF_RUN_HERO_LOOK_KEY:-}
    HERO_OF[$seat]=${DF_RUN_ROAM:-hero-$DF_RUN_HERO_ID}
  fi
  [ "${#SEATS[@]}" -gt 0 ]
}

# The heroes of the journey, in seat order, comma-separated.
journey_heroes() {
  local seat heroes=""
  for seat in "${SEATS[@]}"; do heroes="${heroes:+$heroes,}${HERO_OF[$seat]}"; done
  printf '%s' "$heroes"
}

# Each hero ready: persona, reference sheet, kit sheets, rules sheet and party card. Returns
# non-zero with a `[prepare] failed: ..` line when a step is refused.
journey_prepare_heroes() {
  local seat out sheet hero
  for seat in "${SEATS[@]}"; do
    hero=${HERO_OF[$seat]}
    printf '[prepare] seat %s: hero %s plays %s (%s %s %s)\n' "$seat" "${HERO_ID_OF[$seat]}" \
      "$hero" "${HERO_NAME_OF[$seat]}" "${HERO_SPECIES_OF[$seat]}" "${HERO_CLASS_OF[$seat]}"
    # prepare_hero_persona words the hint from the single-seat names.
    DF_RUN_HERO_NAME=${HERO_NAME_OF[$seat]} DF_RUN_HERO_SPECIES=${HERO_SPECIES_OF[$seat]} \
      DF_RUN_HERO_CLASS=${HERO_CLASS_OF[$seat]} DF_RUN_HERO_LOOK=${HERO_LOOK_OF[$seat]} \
      prepare_hero_persona "$hero" "$JOURNEY_HERO_HINT" || return 1
    sheet=${HERO_SHEET_OF[$seat]}
    if [ -z "$sheet" ] || ! library_holds "$sheet"; then
      prepare_failed reference "seat $seat's reference sheet (${sheet:-none named}) is not in the library"
      return 1
    fi
    printf '[prepare] reference %s\n' "$sheet"
    out=$($D dev --port $P world inventory sheets "$hero" 2>&1) ||
      { prepare_failed kit "$out"; return 1; }
    printf '%s\n' "$out" | sed 's/^ */[prepare] kit sheets: /'
    RULES_SHEET_OF[$seat]="$(dirname "$FRAMES_LOG")/sheet-$hero.json"
    mkdir -p "$(dirname "${RULES_SHEET_OF[$seat]}")"
    out=$($D dev --port $P creation sheet "$seat" "out=${RULES_SHEET_OF[$seat]}" 2>&1) ||
      { prepare_failed sheet "$out"; return 1; }
    [ -s "${RULES_SHEET_OF[$seat]}" ] || { prepare_failed sheet "no sheet was written: $out"; return 1; }
    printf '[prepare] rules sheet of seat %s: %s\n' "$seat" "$(printf '%s\n' "$out" | head -1)"
    out=$($D dev --port $P scene party add "${HERO_NAME_OF[$seat]:-$hero}" "character=$hero" 2>&1) ||
      { prepare_failed party "$out"; return 1; }
    printf '[prepare] party: %s\n' "$(printf '%s\n' "$out" | head -1)"
  done
}

# Who takes money in the beats, one id a line: each `@pay` payee and each `@shop` keeper (the NPC of
# the stop the shop is in). Read from the directives, never named here.
journey_till_owners() {
  local index step stop="" pays
  for index in "${!STEPS[@]}"; do
    step=${STEPS[$index]}
    case "${step%% *}" in
      stop) stop=$(journey_field "${step#* }" 1) ;;
      shop) printf '%s\n' "${NPC_ID[$stop]:-}" ;;
    esac
    pays=${STEP_PAYS[$index]:-}
    [ -n "$pays" ] && printf '%s\n' "$pays" | awk -F'|' 'NF >= 2 { v = $2; gsub(/ /, "", v); print v }'
  done | awk 'NF && !seen[$0]++'
}

# Each taker of money keeps a till that can make change (`world inventory till`), once a run, before
# the first stop: a fare or toll is otherwise refused (no purse, or no change).
journey_prepare_tills() {
  local owner out
  while IFS= read -r owner; do
    out=$($D dev --port $P world inventory till "$owner" 2>&1) ||
      { prepare_failed till "$owner: $out"; return 1; }
    printf '[prepare] till %s: %s\n' "$owner" "$(printf '%s\n' "$out" | head -1)"
  done < <(journey_till_owners)
}

# The scenario's story, with the heroes as its players: what the clue judge and the narrator read.
journey_prepare_story() {
  local out
  [ -n "$JOURNEY_STORY" ] || return 0
  out=$($D dev --port $P persona story author "players=$(journey_heroes)" "$JOURNEY_STORY" 2>&1) ||
    { prepare_failed story "$out"; return 1; }
  printf '[prepare] story: %s\n' "$(printf '%s\n' "$out" | head -1)"
  out=$($D dev --port $P persona story reset 2>&1) || { prepare_failed story "$out"; return 1; }
  printf '[prepare] story reset: %s\n' "$(printf '%s\n' "$out" | head -1)"
}

# The opening plate, painted from every hero's reference sheet at once (made once per set of
# locked looks), scanned and each hero named in it by look and sheet, then shown.
journey_prepare_plate() {
  local seat plate="journey-00-$JOURNEY" refs="" looks="" identify=() out shown
  for seat in "${SEATS[@]}"; do
    plate="$plate-${HERO_ID_OF[$seat]}${HERO_LOOK_KEY_OF[$seat]:+${HERO_LOOK_KEY_OF[$seat]}}"
    refs="${refs:+$refs,}${HERO_SHEET_OF[$seat]}"
    looks="$looks One of them: ${HERO_LOOK_OF[$seat]:-a ${HERO_SPECIES_OF[$seat]} ${HERO_CLASS_OF[$seat]}}."
    identify+=("${HERO_OF[$seat]}=${HERO_LOOK_OF[$seat]:-a ${HERO_SPECIES_OF[$seat]} ${HERO_CLASS_OF[$seat]}}"
      "portrait.${HERO_OF[$seat]}=${HERO_SHEET_OF[$seat]}")
  done
  if library_holds "$plate.png"; then
    printf '[prepare] opening plate %s.png kept\n' "$plate"
  else
    # The reference sheets are attached in seat order, and the words name no one.
    out=$(timeout $HUNG $D dev --port $P media generate "$JOURNEY_START_PLATE$looks" "as=$plate" \
      purpose=plate "refs=$refs" wait=true 2>&1)
    printf '[prepare] opening plate (refs=%s): %s\n' "$refs" "$(printf '%s\n' "$out" | head -1)"
    library_holds "$plate.png" || { prepare_failed plate "$out"; return 1; }
    out=$(timeout 300 $D dev --port $P media locate "$plate.png" "masks=$JOURNEY_MASKS" 2>&1) ||
      { prepare_failed plate "$out"; return 1; }
    printf '[prepare] located: %s\n' "$(printf '%s\n' "$out" | head -1)"
    out=$(timeout 300 $D dev --port $P media locate identify "$plate.png" "${identify[@]}" 2>&1) ||
      { prepare_failed plate "$out"; return 1; }
    printf '%s\n' "$out" | sed -n 's/^  \(.*\)$/[prepare] identify: \1/p'
  fi
  # A kept plate's layout is checked too: a hero its scan missed is scanned for again.
  journey_locate "$plate.png" "${identify[@]}"
  journey_shown_names "$plate.png"
  shown=$($D dev --port $P scene image tv-backdrop "$plate.png" 2>&1) ||
    { prepare_failed shown "$shown"; return 1; }
  printf '[prepare] shown: %s\n' "$(printf '%s\n' "$shown" | head -1)"
  journey_watch "$plate" "$JOURNEY_START_PLATE"
}

# The party heroes and the people of $2 (ids) that `$1`'s layout names no region for.
journey_unnamed() { # picture, more ids
  local shown seat id
  shown=$($D dev --port $P media locate show "$1" 2>&1)
  for seat in "${SEATS[@]}"; do
    printf '%s\n' "$shown" | grep -qE "^ *[0-9]+: ${HERO_OF[$seat]} (\(|\[)" ||
      printf '%s\n' "${HERO_OF[$seat]}"
  done
  for id in $2; do
    printf '%s\n' "$shown" | grep -qE "^ *[0-9]+: $id (\(|\[)" || printf '%s\n' "$id"
  done | awk '!seen[$0]++'
}

# `hero` for a party hero, `npc` for anyone else, as the `[locate]` lines name them.
journey_who_kind() { # id
  case ",$(journey_heroes)," in *",$1,"*) printf hero ;; *) printf npc ;; esac
}

# Every hero (and anyone of `@npc` and the design's residents in the identify words) named in the
# picture `$1`: one missed is a `[locate] missed hero=` line, and the picture is scanned once more
# with room for everyone (`masks=`) and identified again by the words `$@` after it (`<id>=<look>`,
# `portrait.<id>=<sheet>`). One still missed is `[locate] still missed hero=`: their lines go on
# unanchored. The locate API takes no count of people, only the most masks a scan may give.
journey_locate() { # picture, identify words...
  local picture=$1 missing hero others="" word out
  shift
  for word in "$@"; do
    case "$word" in portrait.*) ;; *) others="$others ${word%%=*}" ;; esac
  done
  # The heroes are checked by journey_unnamed itself.
  for hero in "${SEATS[@]}"; do others=${others// ${HERO_OF[$hero]}/}; done
  missing=$(journey_unnamed "$picture" "$others")
  [ -n "$missing" ] || return 0
  for hero in $missing; do
    printf '[locate] missed %s=%s plate=%s\n' "$(journey_who_kind "$hero")" "$hero" "$picture"
  done
  out=$(timeout 300 $D dev --port $P media locate "$picture" "masks=$JOURNEY_MASKS" 2>&1)
  printf '[locate] scanned again: %s\n' "$(printf '%s\n' "$out" | head -1)"
  out=$(timeout 300 $D dev --port $P media locate identify "$picture" "$@" 2>&1)
  printf '%s\n' "$out" | sed -n 's/^  \(.*\)$/[locate] identify: \1/p'
  journey_shown_names "$picture"
  for hero in $(journey_unnamed "$picture" "$others"); do
    printf '[locate] still missed %s=%s plate=%s\n' "$(journey_who_kind "$hero")" "$hero" "$picture"
  done
}

# Every character `$1`'s layout names gets the name it is shown by (`media locate label <picture>
# <id> <id> <shown name>`): a hero's own name, a resident's from the design (SHOWN_NAME).
journey_shown_names() { # picture
  local shown seat id name
  shown=$($D dev --port $P media locate show "$1" 2>&1)
  for seat in "${SEATS[@]}"; do
    SHOWN_NAME[${HERO_OF[$seat]}]=${HERO_NAME_OF[$seat]:-${SHOWN_NAME[${HERO_OF[$seat]}]:-}}
  done
  for id in "${!SHOWN_NAME[@]}"; do
    name=${SHOWN_NAME[$id]}
    [ -n "$name" ] || continue
    printf '%s\n' "$shown" | grep -qE "^ *[0-9]+: $id (\(|\[)" || continue
    printf '[locate] shown name %s=%s: %s\n' "$id" "$name" \
      "$($D dev --port $P media locate label "$1" "$id" "$id" "$name" 2>&1 | head -1)"
  done
}

# The identify words of every hero: `<id>=<look>` and `portrait.<id>=<sheet>`.
journey_hero_words() {
  local seat
  JOURNEY_WORDS=()
  for seat in "${SEATS[@]}"; do
    JOURNEY_WORDS+=("${HERO_OF[$seat]}=${HERO_LOOK_OF[$seat]:-a ${HERO_SPECIES_OF[$seat]} ${HERO_CLASS_OF[$seat]}}"
      "portrait.${HERO_OF[$seat]}=${HERO_SHEET_OF[$seat]}")
  done
}

# A line the picture cannot anchor to its speaker (their region not named): shown as a caption
# with the speaker's name, so it is still seen on the table, until caption_down.
said_caption() { # number, hero, line
  local name=$2 seat
  for seat in "${SEATS[@]}"; do
    [ "${HERO_OF[$seat]}" = "$2" ] && name=${HERO_NAME_OF[$seat]:-$2}
  done
  printf '[caption %02d] %s: %s (%s)\n' "$1" "$name" "$3" \
    "$($D dev --port $P scene caption "$3" "speaker=$name" 2>&1 | head -1)"
  CAPTION_UP=1
}
# Takes down the caption a line was shown as (said_caption), once the plate it stood in for is read
# or play moves on.
caption_down() {
  [ -n "$CAPTION_UP" ] || return 0
  $D dev --port $P scene caption-clear >/dev/null 2>&1
  CAPTION_UP=""
}

# Ends the frame watch of the place the party leaves at once: its input is closed and the watch
# (the runner's own child) is stopped, so a frame it is still making is never shown; `[watch NN]
# cancelled frame of <stop>`.
journey_cancel_watch() { # the stop left
  [ -n "$WATCH_PID" ] || return 0
  exec 4>&-
  kill "$WATCH_PID" 2>/dev/null
  wait "$WATCH_PID" 2>/dev/null
  printf '[watch %02d] cancelled frame of %s (%s)\n' "$STEP" "${1:-the opening}" "$WATCH_LOG"
  WATCH_PID=""
}

# The frame watch of the plate up now, as a roam's place has: the lines said are handed to it.
journey_watch() { # plate, what it shows
  stop_watch quiet
  start_watch "${FRAMES_LOG%.log}-$1.log" "frames=$1-watch" "scene=$2" \
    ${WATCH_REFS:+"$WATCH_REFS"} "${FRAMES[@]}"
}

# The seat a step is put to: `party`, `each`, a seat number, or `class=<class>`, the seat whose
# hero is of that class (`DF_RUN_HERO_<n>_CLASS`, compared as ids); prints nothing and returns
# non-zero when no seat plays that class.
journey_who() { # who
  local seat class
  case "$1" in
    class=*)
      class=${1#class=}
      for seat in "${SEATS[@]}"; do
        [ "${HERO_CLASS_OF[$seat],,}" = "${class,,}" ] && { printf '%s' "$seat"; return 0; }
      done
      return 1 ;;
    *) printf '%s' "$1" ;;
  esac
}

# Sets JOURNEY_WHO to the seat of `$1` (journey_who); a class nobody plays is a `[skip NN]` line
# and a non-zero return, never another seat.
journey_seat_of() { # who
  if ! JOURNEY_WHO=$(journey_who "$1"); then
    printf '[skip %02d] no seat plays a %s (seats: %s)\n' "$STEP" "${1#class=}" \
      "$(for seat in "${SEATS[@]}"; do printf '%s=%s ' "$seat" "${HERO_CLASS_OF[$seat]:-?}"; done)"
    return 1
  fi
}

# The dungeon master says $1 (shown in its box, voiced when TTS is on), heard out before play goes on.
journey_dm() { # line
  local out
  if out=$($D dev --port $P persona narrate speak=true "$1" 2>&1); then
    printf '[dm %02d] %s\n' "$STEP" "$1"
  else
    printf '[dm %02d] %s <%s>\n' "$STEP" "$1" "$(printf '%s\n' "$out" | head -1)"
  fi
  watch_line narrator "$1"
  heard_out "$1"
  wait_heard
}

# The clue judge reads a line at a stop that has clues (`@clue`), against the story's evidence on
# show there: what was said ($4 `said`, the default), what a player's hero does there in the
# player's own words ($4 `look`: a look, a read, a touch reaches a piece up close), or a spell cast
# there ($4 `spell:<what it does>`: what it lays bare, no check needed). A piece seen is kept on the
# story's findings (the clue board on the TV and the phones' Clues), shown up close (its reference
# sheet and a close-up, which the board then shows), and the narrator says what it adds in the
# story's words (`found`, `worked_out`); a stuck party is `stalled`. Every judgement is one `[clue
# NN]` line. Sets CLUE_STALLED.
journey_clues() { # stop, hero, line, [said|look|spell:<what it does>]
  local out found moment="" options=() kind=${4:-said} id text
  CLUE_STALLED=""
  [ -n "$1" ] || return 0
  case " ${CLUE_STOPS[*]:-} " in *" $1 "*) ;; *) return 0 ;; esac
  case "$kind" in
    look) options=(look=true) ;;
    spell:*) options=("spell=${kind#spell:}") ;;
  esac
  if ! out=$(timeout $HUNG $D dev --port $P persona story clues "${options[@]}" "$1" "$2: $3" 2>&1); then
    printf '[clue %02d] <clues: %s>\n' "$STEP" "$(printf '%s\n' "$out" | head -1)"
    return 0
  fi
  printf '[clue %02d] judged %s at %s: %s\n' "$STEP" "${kind%%:*}" "$1" \
    "$(printf '%s\n' "$out" | sed -n 's/^\(seen\|worked_out\)=\([^ ]*\).*/\1:\2/p' | paste -sd ' ' -)"
  while IFS= read -r found; do
    case "$found" in
      seen=*)
        printf '[clue %02d] seen %s\n' "$STEP" "${found#seen=}"
        id=${found#seen=}; text=${id#* }; id=${id%% *}
        CLUE_SEEN[$id]=1
        journey_focus "$1" "$text"
        [ -n "$moment" ] || moment=found ;;
      worked_out=*) printf '[clue %02d] worked out %s\n' "$STEP" "${found#worked_out=}"; moment=worked_out ;;
      stalled=true) CLUE_STALLED=1; printf '[clue %02d] stalled\n' "$STEP" ;;
    esac
  done <<< "$out"
  [ -n "$moment" ] && journey_story_dm "$1" "$moment"
}

# A found piece shown up close: its reference sheet in the story (`persona story item`, kept or
# designed now) and a close-up of it from the frame watch (`@focus`), which keeps the picture on
# the sheet and so on the clue board and the phones' Clues.
journey_focus() { # stop, what a close look shows
  local item
  item=$($D dev --port $P persona story item "$1" "$2" 2>/dev/null | sed -n 's/^item=//p' | head -1)
  printf '[clue %02d] close-up %s\n' "$STEP" "${item:-<no sheet>}"
  [ -n "$WATCH_PID" ] && printf '@focus %s%s\n' "${item:+$item | }" "$2" >&4
}

# The clues of stop $1 the party leaves without having found: one `[clue NN] missed id=` each.
journey_missed() { # stop
  local clue
  [ -n "$1" ] || return 0
  for clue in "${!CLUE_AT[@]}"; do
    [ "${CLUE_AT[$clue]}" = "$1" ] && [ -z "${CLUE_SEEN[$clue]:-}" ] &&
      printf '[clue %02d] missed id=%s at=%s\n' "$STEP" "$clue" "$1"
  done
  return 0
}

# The narrator's word from the story at stop $1 for moment $2 (`found`, `worked_out`, `stalled`:
# a hint that never names the answer or pairs clues), said when it has one.
journey_story_dm() { # stop, moment
  local line
  line=$(timeout $HUNG $D dev --port $P persona story narrate "$1" "$2" 2>&1) ||
    { printf '[dm %02d] <narrate %s: %s>\n' "$STEP" "$2" "$(printf '%s\n' "$line" | head -1)"; return 0; }
  [ -n "$line" ] || return 0
  [ "$2" = stalled ] && printf '[dm %02d] (hint)\n' "$STEP"
  journey_dm "$line"
}

# One stop: the seat's move, played as a move with a plate of its own, kept and shown.
journey_stop() { # step text, cards
  local id way who prompt where plate out note="" said picture place live hero seat situation
  id=$(journey_field "$1" 1); way=$(journey_field "$1" 2); who=$(journey_field "$1" 3)
  prompt=$(journey_field "$1" 4); where=$(journey_field "$1" 5)
  STOP_NUMBER=$((STOP_NUMBER + 1))
  journey_missed "$STOP"
  # The last place's frame watch ends before the move, its frame in the making dropped: a frame of
  # the old place must not go up over the new place's plate (run 7: each stop showed the stop
  # before it), and the next ask never waits on a picture of a place the party has left.
  journey_cancel_watch "$STOP"
  STOP=$id
  plate=$(printf 'journey-%s-%02d-%s' "$JOURNEY_STAMP" "$STOP_NUMBER" "$id")
  printf '\n[stop %02d] %s (%s of %s) way=%s asked=%s\n' "$STEP" "$id" "$STOP_NUMBER" "$STOP_TOTAL" "$way" "$who"
  situation="The way leads next to $where. ${WAY[$way]:-}${here:+ You are at $here now.}"
  local tries=0 moved=() npc=${NPC_ID[$id]:-} notified resident
  moved=("party=$(journey_heroes)")
  [ -n "$npc" ] && moved+=("resident=$npc|${NPC_WHO[$id]}")
  while :; do
    seat_line "$STEP" "$who" "$prompt" "$note" "$2" move "$((SECONDS + JOURNEY_ASK_SECONDS))"
    if [ -z "$SAID_LINE" ]; then
      printf '[skip %02d] stop %s: no answer and no card to go on with\n' "$STEP" "$id"
      return
    fi
    seat=$SAID_SEAT
    hero=${HERO_OF[$seat]}
    ROAM=$hero
    roam_move_act "$STEP" "$hero" "plate=$plate" $(roam_walk_options "$way") "${moved[@]}" \
      "text=$SAID_LINE" "$situation"
    out=$MOVE_OUT
    # The hero is in the party but the picture up names no region for them: the move goes on
    # unanchored, the line shown as a caption.
    if printf '%s\n' "$out" | grep -q '^unanchored: '; then
      printf '[move %02d] %s\n' "$STEP" "$(printf '%s\n' "$out" | grep -m1 '^unanchored: ')"
      said_caption "$STEP" "$hero" "$SAID_LINE"
    fi
    # Why the move was not read stays in the log; the phones get a player's line, never the
    # judge's reason.
    note=$(printf '%s\n' "$out" | sed -n 's/^not moved: //p' | head -1)
    [ -n "$note" ] && note=$JOURNEY_NOT_A_MOVE
    if printf '%s\n' "$out" | grep -q '^back at place='; then
      note="The way goes on from here, not back. Say how you go on toward $where."
    fi
    if [ -z "$note" ] && [ -z "$(stage_at "$(printf '%s\n' "$out" | grep '^TRANSITION ')" live)" ]; then
      # A move that did not go live is never passed over in silence: both phones are told, and
      # the seat is asked again.
      tries=$((tries + 1))
      printf '[move %02d] <did not go live (try %s of %s): %s>\n' "$STEP" "$tries" \
        "$JOURNEY_MOVE_TRIES" "$(printf '%s\n' "$out" | tail -1)"
      for notified in "${SEATS[@]}"; do
        seat_notice "$STEP" "$notified" "That didn't happen. Try again."
      done
      if [ "$tries" -ge "$JOURNEY_MOVE_TRIES" ]; then
        printf '[failed %02d] stop %s: the move did not go live after %s tries\n' "$STEP" "$id" "$tries"
        return
      fi
      note="That didn't happen. Try again: say where you go."
      continue
    fi
    [ -n "$note" ] || break
    printf '[move %02d] not moved: %s\n' "$STEP" \
      "$(printf '%s\n' "$out" | sed -n 's/^not moved: //p' | head -1)"
  done
  said=$(printf '%s\n' "$out" | sed -n "s/^$hero says: //p" | head -1)
  picture=$(printf '%s\n' "$out" | sed -n 's/^picture: //p' | head -1)
  place=$(printf '%s\n' "$out" | sed -n 's/^design: place=\(.*\) mood=.*/\1/p' | head -1)
  live=$(stage_at "$(printf '%s\n' "$out" | grep '^TRANSITION ')" live)
  printf '[move %02d] seat %s (%s) | said: %s | place: %s | plate: %s | live_at_ms=%s\n' "$STEP" \
    "$seat" "$hero" "${said:-<none>}" "${place:-<none>}" "$plate" "${live:--}"
  roam_walk_lines "$STEP" "$out"
  roam_inventory "$said" "$out"
  [ -n "$said" ] && HISTORY+=("$hero: $said")
  here=$place
  printf '[arrival %02d] stop %s plate=%s.png place=%s\n' "$STEP" "$id" "$plate" "$place"
  if [ -n "$CARD_UP" ]; then
    # The card was the waiting screen while the arrival plate was made. The plate is live now: the
    # card stays one read hold (ROAM_DWELL, the time any new plate is looked at before the scan and
    # the answer) so the title can be read against it, then comes down. The plate is not covered
    # for the scan, the dwell or the answer; the host's REGION card_cleared line carries the at_ms.
    seat_dwell "$STEP" dwell "$ROAM_DWELL"
    printf '[region %02d] %s\n' "$STEP" "$($D dev --port $P world region card-clear 2>&1 | head -1)"
    CARD_UP=""
  fi
  # Every hero, and the stop's NPC and residents, named in the new plate; scanned again once if not.
  journey_hero_words
  while IFS= read -r resident; do
    [ -n "$resident" ] && JOURNEY_WORDS+=("$resident")
  done < <(printf '%s\n' "$out" | sed -n 's/^resident \([^ ]*\) ([^)]*): [^,]*, \(.*\)$/\1=\2/p')
  while IFS=$'\t' read -r resident name; do
    [ -n "$resident" ] && SHOWN_NAME[$resident]=$name
  done < <(printf '%s\n' "$out" | sed -n 's/^resident \([^ ]*\) (\([^)]*\)): .*$/\1\t\2/p')
  journey_locate "$plate.png" "${JOURNEY_WORDS[@]}"
  journey_watch "$plate" "$picture"
  journey_remember "$plate" "$place" "$picture"
  if [ -n "$QUEST_TAKEN" ]; then
    printf '[quest %02d] %s\n' "$STEP" \
      "$($D dev --port $P world quest progress "$STOP_NUMBER" "$STOP_TOTAL" "$place" 2>&1 | head -1)"
  fi
  seat_dwell "$STEP" dwell "$ROAM_DWELL"
  caption_down
  seat_answer "$STEP" "$seat" "$SAID_TO" "$said"
}

# The plate that went live, kept as a scene memory of every hero (the phone's Map), tagged with
# the region the party is in.
journey_remember() { # plate, place name, what the plate shows
  local seat kept
  [ -n "$SCENE_KIND" ] || return 0
  for seat in "${SEATS[@]}"; do
    kept=$($D dev --port $P world memory capture "${HERO_OF[$seat]}" "$SCENE_KIND" "$1.png" \
      "${2:-$1}" "place=$1" ${REGION:+"tag=$REGION"} "description=${3:-${2:-$1}}" by=runner 2>&1 | head -1)
    printf '[memory %02d] seat %s: %s\n' "$STEP" "$seat" "$kept"
  done
}

# A seat's line at the stop: said, answered, and read by the clue judge.
journey_line() { # step text, cards, kind
  local who prompt moment npc=""
  who=$(journey_field "$1" 1); prompt=$(journey_field "$1" 2); moment=$(journey_field "$1" 3)
  # A talk step is put to the stop's NPC unless the words are put to someone else.
  [ "${3:-}" = talk ] && npc=${NPC_ID[$STOP]:-}
  journey_seat_of "$who" || return 0
  who=$JOURNEY_WHO
  # Every line step is asked as a line (the seat's ask kinds are line, move and look): a talk step
  # differs only in who answers it.
  seat_line "$STEP" "$who" "$prompt" "" "$2" line "$((SECONDS + JOURNEY_ASK_SECONDS))"
  [ -n "$SAID_LINE" ] || return 0
  seat_say "$STEP" "$SAID_SEAT" "$SAID_TO" "$SAID_LINE" "" || return 0
  # A look step's line is the hero's own action, judged for what it reaches before anyone answers;
  # a talk step's line is talk, and the answer is judged too (seat_reply).
  if [ "${3:-}" = talk ]; then
    seat_answer "$STEP" "$SAID_SEAT" "$SAID_TO" "$HERO_SAID" "$npc" "$moment"
    journey_clues "$STOP" "${HERO_OF[$SAID_SEAT]}" "$HERO_SAID"
  else
    journey_clues "$STOP" "${HERO_OF[$SAID_SEAT]}" "$HERO_SAID" look
    seat_answer "$STEP" "$SAID_SEAT" "$SAID_TO" "$HERO_SAID" "$npc" "$moment"
  fi
}

# A check: the scenario's test, called by the dungeon master. The seat says what the hero does
# (a card's thing `@pay`s instead when it names a payment), then the check judge words the test the
# beats wrote (`intent=`, at a narrator's moment, told the hero's line and the stakes the beats set
# as what is in sight), and the host rolls it on the hero's own rules sheet. A judge that still
# calls no roll is overruled at the beats' difficulty (`dc=`): `CHECK called_by_dm`. The stop's
# NPC answers as the roll turned out.
journey_check() { # step text, cards, payments
  local head who designed dc tool prompt stakes out header options
  head=$(journey_field "$1" 1); prompt=$(journey_field "$1" 2); stakes=$(journey_field "$1" 3)
  read -r who designed options <<< "$head"
  dc=$(printf '%s\n' "$options" | tr ' ' '\n' | sed -n 's/^dc=//p' | head -1)
  tool=$(printf '%s\n' "$options" | tr ' ' '\n' | sed -n 's/^tool=//p' | head -1)
  local gate="" outcome
  case " $options " in *" gate "*) gate=1 ;; esac
  journey_seat_of "$who" || return 0
  who=$JOURNEY_WHO
  seat_line "$STEP" "$who" "$prompt" "" "$2" line "$((SECONDS + JOURNEY_ASK_SECONDS))"
  [ -n "$SAID_LINE" ] || return 0
  seat_say "$STEP" "$SAID_SEAT" "$SAID_TO" "$SAID_LINE" "${stakes}" || return 0
  wait_heard
  # A card that pays (`@pay <thing> | ..`) is the payment, not the test.
  if [ -n "$SAID_THING" ] && journey_pay "$SAID_THING" "${3:-}"; then
    return 0
  fi
  out=$(timeout $HUNG $D dev --port $P persona check run "${HERO_OF[$SAID_SEAT]}" \
    "sheet=${RULES_SHEET_OF[$SAID_SEAT]}" moment=narrator "intent=$designed" \
    "told=${HERO_OF[$SAID_SEAT]}: $HERO_SAID" "place=${here:-$STOP}" ${stakes:+"sight=$stakes"} \
    ${dc:+"dc=$dc"} ${tool:+"tool=$tool"} 2>&1)
  header=$(printf '%s\n' "$out" | sed -n '1p')
  printf '[check %02d] seat %s %s%s%s | %s\n' "$STEP" "$SAID_SEAT" "$designed" "${dc:+ dc=$dc}" \
    "${tool:+ tool=$tool}" "$header"
  printf '%s\n' "$out" | sed -n '2,$p' | sed "s/^/[check $(printf '%02d' "$STEP")] /"
  if ! printf '%s\n' "$out" | grep -q '^ *roll: '; then
    printf '[failed %02d] check %s was not rolled: %s\n' "$STEP" "$designed" "$header"
  fi
  # The roll's verdict as the rules engine gave it (`outcome=` on the roll line).
  outcome=$(printf '%s\n' "$out" | sed -n 's/^ *roll: .* outcome=\([a-z_]*\).*/\1/p' | head -1)
  # Someone of the stop answers what was tried, as the roll turned out (a toll keeper won over or
  # not).
  if [ -n "${NPC_ID[$STOP]:-}" ]; then
    seat_answer "$STEP" "$SAID_SEAT" "$SAID_TO" "$HERO_SAID" "${NPC_ID[$STOP]}" \
      "What was just tried, and the roll: $(printf '%s\n' "$out" | sed -n 's/^ *roll: //p' | head -1). $(printf '%s\n' "$out" | sed -n 's/^ *result ([^)]*): //p' | head -1)"
  fi
  journey_clues "$STOP" "${HERO_OF[$SAID_SEAT]}" "$HERO_SAID" look
  # A gate check (`gate`): only a success opens the way; otherwise it stays barred until paid.
  if [ -n "$gate" ]; then
    if [ "$outcome" = success ]; then
      printf '[gate %02d] open by=check %s\n' "$STEP" "$designed"
    else
      printf '[gate %02d] barred: %s %s\n' "$STEP" "$designed" "${outcome:-not rolled}"
      journey_gate "$who" "The way is still barred. Pay to go on." "$2" "${3:-}"
    fi
  fi
}

# A payment a card names (`@pay <thing> | <to> | <copper each> | <why>`, the lines after the step):
# when the card tapped carries `<thing>`, every hero pays `<copper each>` to `<to>` through the
# inventory's coin payment, with change (`world inventory pay`), and `<to>` answers, told what the
# purse recorded: who paid and whose payment was refused, and why (so a refused fare is never
# answered as paid). Returns non-zero when no payment of the step is that thing. A refused payment
# is logged (`[pay NN] refused`) and told to the seat in the host's `phone:` words.
journey_pay() { # thing, payments
  local spec to cp why seat hero name paid reason words record paid_by="" refused=""
  PAY_OK=""
  spec=$(printf '%s\n' "$2" | awk -F'|' -v thing="$1" '{ t = $1; gsub(/ /, "", t); if (t == thing) { print; exit } }')
  [ -n "$spec" ] || return 1
  PAY_OK=1
  to=$(journey_field "$spec" 2); cp=$(journey_field "$spec" 3); why=$(journey_field "$spec" 4)
  for seat in "${SEATS[@]}"; do
    hero=${HERO_OF[$seat]}
    name=${HERO_NAME_OF[$seat]:-$hero}
    paid=$($D dev --port $P world inventory pay "$hero" "$to" "$cp" "why=${why:-$1}" 2>&1)
    if [ $? -eq 0 ]; then
      printf '[pay %02d] %s paid %s cp to %s: %s\n' "$STEP" "$hero" "$cp" "$to" \
        "$(printf '%s\n' "$paid" | head -1)"
      paid_by="$paid_by${paid_by:+, }$name"
    else
      printf '[pay %02d] refused %s -> %s %s cp: %s\n' "$STEP" "$hero" "$to" "$cp" \
        "$(printf '%s\n' "$paid" | head -1)"
      # The seat is told the host's `phone:` line, never the raw dev reason (the log has that).
      reason=$(printf '%s\n' "$paid" | head -1 | sed 's/^refused ([^)]*): //')
      reason=${reason//"$to"/"${SHOWN_NAME[$to]:-$to}"}
      words=$(printf '%s\n' "$paid" | sed -n 's/^phone: //p' | head -1)
      [ -n "$words" ] || words="The payment didn't go through."
      seat_notice "$STEP" "$seat" "$words"
      refused="$refused $name's payment was refused, so it was not paid: $reason."
      PAY_OK=""
    fi
  done
  record="What the purse recorded just now for ${why:-$1}, $cp cp from each traveller:"
  seat_answer "$STEP" "$SAID_SEAT" "$SAID_TO" "$HERO_SAID" "$to" \
    "$record ${paid_by:-nobody} paid.$refused"
  return 0
}

# A way barred until it is paid (`@fare`, or a `gate` check that was not won): the party is asked
# with the step's paying cards only, until a card pays for every hero (journey_pay); words instead
# are said and answered, and the ask comes again. `[gate NN] open by=pay` when paid; after
# JOURNEY_GATE_TRIES refused payments `[failed NN] gate not paid` and the run goes on.
journey_gate() { # who, prompt, cards, payments
  local cards="" card thing note="" refused=0
  while IFS= read -r card; do
    [ -n "$card" ] || continue
    thing=$(journey_field "$card" 3)
    [ -n "$thing" ] && printf '%s\n' "$4" | grep -q "^ *$thing *|" && cards="$cards$card"$'\n'
  done <<< "$3"
  if [ -z "$cards" ]; then
    printf '[failed %02d] gate: no card pays (@card <label> | <line> | <thing> with a @pay <thing>)\n' "$STEP"
    return 0
  fi
  while :; do
    seat_line "$STEP" "$1" "$2" "$note" "$cards" line "$((SECONDS + JOURNEY_ASK_SECONDS))"
    [ -n "$SAID_LINE" ] || continue
    seat_say "$STEP" "$SAID_SEAT" "$SAID_TO" "$SAID_LINE" "" || continue
    if [ -n "$SAID_THING" ] && journey_pay "$SAID_THING" "$4"; then
      if [ -n "$PAY_OK" ]; then
        printf '[gate %02d] open by=pay %s\n' "$STEP" "$SAID_THING"
        return 0
      fi
      refused=$((refused + 1))
      if [ "$refused" -ge "$JOURNEY_GATE_TRIES" ]; then
        printf '[failed %02d] gate not paid after %s refused payments\n' "$STEP" "$refused"
        return 0
      fi
      note="The payment didn't go through. Try again."
      continue
    fi
    # Words, not a paying card: answered, and the way stays barred.
    seat_answer "$STEP" "$SAID_SEAT" "$SAID_TO" "$HERO_SAID" "${NPC_ID[$STOP]:-}" \
      "The way stays barred until it is paid."
    note="The way stays barred until you pay."
  done
}

# A fare step (`@fare <who> | <prompt>`, its paying cards and `@pay` lines after it): the way on is
# barred until it is paid (journey_gate).
journey_fare() { # step text, cards, payments
  local who prompt
  who=$(journey_field "$1" 1); prompt=$(journey_field "$1" 2)
  journey_seat_of "$who" || return 0
  journey_gate "$JOURNEY_WHO" "$prompt" "$2" "$3"
}

# The shop: every seat asked (`each`) or the one named, until each says it is done; Buy on the
# phone is the host's own, and talk meanwhile is taken as it comes.
journey_shop() { # step text, cards
  local who prompt seat seats=() left deadline=$((SECONDS + JOURNEY_SHOP_SECONDS))
  who=$(journey_field "$1" 1); prompt=$(journey_field "$1" 2)
  journey_seat_of "$who" || return 0
  who=$JOURNEY_WHO
  case "$who" in each|party) seats=("${SEATS[@]}") ;; *) seats=("$who") ;; esac
  for seat in "${seats[@]}"; do seat_ask "$STEP" "$seat" "$prompt" "" "$2" line; done
  left=" ${seats[*]} "
  while [ -n "${left// /}" ]; do
    for seat in "${seats[@]}"; do
      case "$left" in *" $seat "*) ;; *) continue ;; esac
      # The shop's own deadline, for every seat together; at it, Done is the default.
      seat_wait "$STEP" "$seat" "" "$2" "$deadline"
      printf '[shop %02d] seat %s is done: %s\n' "$STEP" "$seat" "${SAID_LINE:-<no answer>}"
      left=${left/ $seat / }
    done
  done
  for seat in "${seats[@]}"; do
    printf '[shop %02d] seat %s holds: %s\n' "$STEP" "$seat" \
      "$($D dev --port $P world inventory get "${HERO_OF[$seat]}" 2>/dev/null | sed -n '2,$p' | paste -sd ';' -)"
  done
}

# A cast: the seat is asked and the cast from its Hero page awaited; what the host says the spell
# shows is read by the clue judge as the hero's own finding. A line instead is played as a line.
journey_cast() { # step text, cards
  local who prompt
  who=$(journey_field "$1" 1); prompt=$(journey_field "$1" 2)
  journey_seat_of "$who" || return 0
  who=$JOURNEY_WHO
  seat_ask "$STEP" "$who" "$prompt" "" "$2" line
  seat_wait "$STEP" "$who" cast "$2" "$((SECONDS + JOURNEY_ASK_SECONDS))"
  # A cast was seen as it came (seat_cast); a line instead is played as a line.
  [ -n "$CAST_LINE" ] && return 0
  [ -n "$SAID_LINE" ] || return 0
  seat_say "$STEP" "$SAID_SEAT" "$SAID_TO" "$SAID_LINE" "" || return 0
  journey_clues "$STOP" "${HERO_OF[$SAID_SEAT]}" "$HERO_SAID" look
}

# The quest `$1` as `world quest offer` takes it: `<id>|<title>|<giver>|<reward_gp>|<advance_gp>|
# <site>|<clue,ids>`.
journey_quest_word() { # quest id
  local field word=$1
  for field in 1 2 3 4 5 6; do word="$word|$(journey_field "${QUEST_SPEC[$1]}" "$field")"; done
  printf '%s' "$word"
}

# The quest board: the quests offered, the board shown as a close-up handout, the party's pick by
# card or by Jev, and the quest taken.
journey_board() { # step text
  local board ids picture id specs=() cards="" out quest="" note="" heroes=() seat
  board=$(journey_field "$1" 1); ids=$(journey_field "$1" 2); picture=$(journey_field "$1" 3)
  for id in ${ids//,/ }; do
    specs+=("$(journey_quest_word "$id")")
    cards="$cards$(journey_field "${QUEST_SPEC[$id]}" 1) ($(journey_field "${QUEST_SPEC[$id]}" 3) gp) | We take this one: $(journey_field "${QUEST_SPEC[$id]}" 1). | $id"$'\n'
  done
  $D dev --port $P world quest offer "${specs[@]}" 2>&1 | sed "s/^/[quest $(printf '%02d' "$STEP")] /"
  if ! library_holds "$board.png"; then
    out=$(timeout $HUNG $D dev --port $P media generate "$picture" "as=$board" purpose=plate wait=true 2>&1)
    printf '[quest %02d] board picture: %s\n' "$STEP" "$(printf '%s\n' "$out" | head -1)"
  fi
  if library_holds "$board.png" && [ -n "$WATCH_PID" ]; then
    printf '@show %s.png | %s | %s\n' "$board" "The quest board" "$BOARD_SECONDS" >&4
    printf '[quest %02d] board shown: %s.png for %s s\n' "$STEP" "$board" "$BOARD_SECONDS"
  fi
  while [ -z "$quest" ]; do
    seat_line "$STEP" party "Which quest do you take?" "$note" "$cards" line \
      "$((SECONDS + JOURNEY_ASK_SECONDS))"
    seat_say "$STEP" "$SAID_SEAT" "$SAID_TO" "$SAID_LINE" "The quest board is before you." ||
      continue
    if [ -n "$SAID_THING" ] && [ -n "${QUEST_SPEC[$SAID_THING]:-}" ]; then
      quest=$SAID_THING
      printf '[quest %02d] picked %s by card\n' "$STEP" "$quest"
    else
      out=$(timeout $HUNG $D dev --port $P persona quest pick "${HERO_OF[$SAID_SEAT]}" "$HERO_SAID" 2>&1)
      quest=$(printf '%s\n' "$out" | sed -n 's/^quest=//p')
      printf '[quest %02d] Jev: %s\n' "$STEP" "$(printf '%s\n' "$out" | paste -sd ' ' -)"
      [ "$quest" = none ] && quest=""
      note="Say which quest of the board you take."
    fi
  done
  for seat in "${SEATS[@]}"; do heroes+=("${HERO_OF[$seat]}"); done
  $D dev --port $P world quest accept "$quest" "${heroes[@]}" 2>&1 | sed "s/^/[quest $(printf '%02d' "$STEP")] /"
  QUEST_TAKEN=$quest
  # The quest's giver answers the pick, told which quest was taken.
  seat_answer "$STEP" "$SAID_SEAT" "$SAID_TO" "$HERO_SAID" "${NPC_ID[$STOP]:-}" \
    "The party has just taken this quest from the board: $(journey_field "${QUEST_SPEC[$quest]}" 1)."
}

# The riddle at the door: asked, then each line judged until it is solved.
journey_riddle() { # step text, cards
  local id door answer_id answer clues policy word out verdict note=""
  id=$(journey_field "$1" 1); door=$(journey_field "$1" 2); answer_id=$(journey_field "$1" 3)
  answer=$(journey_field "$1" 4); clues=$(journey_field "$1" 5); policy=$(journey_field "$1" 6)
  word="$id|$door|$answer_id|$answer|$clues"
  printf '[riddle %02d] %s\n' "$STEP" \
    "$($D dev --port $P persona riddle ask "$word" 2>&1 | paste -sd ' ' -)"
  journey_dm "$door"
  verdict=""
  while [ "$verdict" != solved ]; do
    seat_line "$STEP" party "What is the answer to the door?" "$note" "$2" line \
      "$((SECONDS + JOURNEY_ASK_SECONDS))"
    if [ -z "$SAID_LINE" ]; then
      # Nobody answered in time and the door has no default: the narrator's hint, then ask again.
      [ "$policy" = stalled ] && journey_story_dm "$STOP" stalled
      continue
    fi
    seat_say "$STEP" "$SAID_SEAT" "$SAID_TO" "$SAID_LINE" "You stand before the sealed door." ||
      continue
    journey_clues "$STOP" "${HERO_OF[$SAID_SEAT]}" "$HERO_SAID"
    out=$(timeout $HUNG $D dev --port $P persona riddle judge "$word" "${HERO_OF[$SAID_SEAT]}" "$HERO_SAID" 2>&1)
    verdict=$(printf '%s\n' "$out" | sed -n 's/^verdict=//p')
    printf '[riddle %02d] judged: %s\n' "$STEP" "$(printf '%s\n' "$out" | paste -sd ' ' -)"
    case "$verdict" in
      solved) ;;
      close) note=$RIDDLE_CLOSE_NOTE ;;
      *) note=$RIDDLE_WRONG_NOTE ;;
    esac
    if [ "$verdict" != solved ] && [ -n "$CLUE_STALLED" ] && [ "$policy" = stalled ]; then
      journey_story_dm "$STOP" stalled
    fi
  done
  # The door is open: the quest is done, and its reward is paid to the party's heroes.
  if [ -n "$QUEST_TAKEN" ]; then
    printf '[quest %02d] %s\n' "$STEP" \
      "$($D dev --port $P world quest complete "$QUEST_TAKEN" 2>&1 | paste -sd ' ' -)"
  fi
}

# The crossing into another region: the world's region changes and its title card goes up.
journey_teleport() { # step text
  local out
  # `@teleport <region> | <arrival> | <card title> | <look>`; the world verb takes the title first.
  out=$($D dev --port $P world region enter "$(journey_field "$1" 1) | $(journey_field "$1" 3) | $(journey_field "$1" 2) | $(journey_field "$1" 4)" 2>&1)
  printf '%s\n' "$out" | sed "s/^/[region $(printf '%02d' "$STEP")] /"
  case "$out" in
    'ok region id='*) REGION=$(journey_field "$1" 1); CARD_UP=1; here="" ;;
  esac
}

# `--plan`: every step as it will run, checked against the beats and the story file, with no call
# to the host. Exits 0 when nothing is wrong.
journey_plan() {
  local index step kind text number=0 stops=0 id clue checks=0 talks=0 clues_at="" problems=0
  local rogue=0 wizard=0 casts=0 board=0 riddle=0 teleport=0 seen=" " who designed dc spec
  problem() { printf '[plan] PROBLEM: %s\n' "$*"; problems=$((problems + 1)); }
  printf '[plan] journey %s: %s steps, story %s\n' "$JOURNEY" "${#STEPS[@]}" "${JOURNEY_STORY:-none}"
  [ -n "$JOURNEY_STORY" ] && [ -f "$JOURNEY_STORY" ] || problem "the story file ${JOURNEY_STORY:-(none)} does not exist"
  [ -n "$JOURNEY_START_PLATE" ] || problem "no @journey_start_plate"
  printf '[plan] seats: %s\n' "${DF_RUN_PLAYERS:-${DF_RUN_PLAYER:-<from the host: DF_RUN_PLAYERS or DF_RUN_PLAYER>}}"
  printf '[plan] prepare: per seat persona (@journey_hero_hint), reference sheet, world inventory sheets, creation sheet <seat> out=.., scene party add\n'
  printf '          world inventory till <owner> for each taker of money: %s\n' \
    "$(journey_till_owners | paste -sd ' ' -)"
  printf '          persona story author players=<heroes> %s; persona story reset\n' "$JOURNEY_STORY"
  printf '          media generate "%s <looks>" purpose=plate refs=<sheet of seat 1>,<sheet of seat 2> wait=true; media locate; media locate identify <hero>=<look> portrait.<hero>=<sheet> ...\n' "$JOURNEY_START_PLATE"
  for clue in "${!CLUE_AT[@]}"; do
    id=${CLUE_AT[$clue]}
    if [ -f "$JOURNEY_STORY" ]; then
      grep -q "\"id\": \"$clue\"" "$JOURNEY_STORY" || problem "clue $clue is no evidence of the story"
      grep "\"scene\": \"$id\"" "$JOURNEY_STORY" | grep -q "\"exhibits\": \[[^]]*\"$clue\"" ||
        problem "clue $clue is not on show at $id in the story"
    fi
  done
  for id in "${CLUE_STOPS[@]}"; do
    [ -f "$JOURNEY_STORY" ] && ! grep -q "\"id\": \"$id\"" "$JOURNEY_STORY" &&
      problem "clues are read at $id, which is no scene of the story"
  done
  id=""
  for index in "${!STEPS[@]}"; do
    step=${STEPS[$index]}
    kind=${step%% *}
    text=${step#* }
    number=$((number + 1))
    case "$kind" in
      stop)
        stops=$((stops + 1)); id=$(journey_field "$text" 1)
        case "$seen" in *" $id "*) problem "stop $id twice" ;; esac
        seen="$seen$id "
        [ -n "${WAY[$(journey_field "$text" 2)]:-}" ] || problem "stop $id: no @way $(journey_field "$text" 2)"
        [ -n "$(journey_field "$text" 5)" ] || problem "stop $id: no destination words"
        printf '[plan %02d] stop %s (%s) | ask %s kind=move "%s" (%s cards) | move act <hero> plate=journey-<stamp>-NN-%s %s text=<line> "The way leads next to %s. ..."\n' \
          "$number" "$id" "$stops" "$(journey_field "$text" 3)" "$(journey_field "$text" 4)" \
          "$(printf '%s' "${STEP_CARDS[$index]:-}" | grep -c '|')" "$id" \
          "$(roam_walk_options "$(journey_field "$text" 2)")" "$(journey_field "$text" 5)"
        printf '          npc: %s | party=<heroes>, unanchored when the picture up misses the speaker; a move that does not go live is told to both phones and asked again (%s tries)\n' \
          "${NPC_ID[$id]:-none}${NPC_ID[$id]:+ (${NPC_WHO[$id]})}" "$JOURNEY_MOVE_TRIES"
        printf '          then: world memory capture <each hero> %s ..%s, world quest progress <n> <total> <place> once a quest is taken, dwell %s s\n' \
          "${SCENE_KIND:-<no @roam_scene_kind>}" "${PLAN_REGION:+ tag=$PLAN_REGION}" "$ROAM_DWELL" ;;
      dm) printf '[plan %02d] dm: persona narrate speak=true "%s"\n' "$number" "$text" ;;
      ask|talk)
        talks=$((talks + 1))
        printf '[plan %02d] %s at %s | ask %s "%s" | persona say text= | persona addressed -> persona ask%s\n' \
          "$number" "$kind" "${id:-?}" "$(journey_field "$text" 1)" "$(journey_field "$text" 2)" \
          "$(case " ${CLUE_STOPS[*]:-} " in *" $id "*) printf ' | persona story clues %s' "$id" ;; esac)" ;;
      check)
        checks=$((checks + 1))
        read -r who designed <<< "$(journey_field "$text" 1)"
        case "$who" in class=rogue) rogue=$((rogue + 1)) ;; class=wizard) wizard=$((wizard + 1)) ;; esac
        dc=$(printf '%s\n' "$text" | sed -n 's/.* dc=\([0-9]*\).*/\1/p' | head -1)
        [ -n "$(journey_field "$text" 3)" ] || problem "check at ${id:-?}: no stakes (third field)"
        case "$designed" in
          ability_check:*|saving_throw:*) [ -n "$dc" ] || problem "check at ${id:-?}: no dc= for the dungeon master's call" ;;
          *) ;;
        esac
        case " $text " in
          *" gate "*)
            [ -n "${STEP_PAYS[$index]:-}" ] || problem "check at ${id:-?}: a gate check needs a @pay"
            printf '          gate: the way stays barred until the check succeeds or a card pays\n' ;;
        esac
        case "$designed" in
          tool_check:*)
            [ -n "$dc" ] || problem "check at ${id:-?}: no dc= for the dungeon master's call"
            printf '%s\n' "$text" | grep -q ' tool=[a-z_]' ||
              problem "check at ${id:-?}: a tool check names its tool= for the dungeon master's call" ;;
        esac
        printf '[plan %02d] check at %s | ask %s "%s" | persona check run <hero> sheet=<rules sheet> moment=narrator intent=%s told=<the line> sight="%s"%s\n' \
          "$number" "${id:-?}" "$who" "$(journey_field "$text" 2)" "$designed" \
          "$(journey_field "$text" 3)" "${dc:+ dc=$dc}"
        while IFS= read -r spec; do
          [ -n "$spec" ] && printf '          a card with thing %s pays: world inventory pay <each hero> %s %s why=%s\n' \
            "$(journey_field "$spec" 1)" "$(journey_field "$spec" 2)" "$(journey_field "$spec" 3)" "$(journey_field "$spec" 4)"
        done <<< "${STEP_PAYS[$index]:-}" ;;
      fare)
        [ -n "${STEP_PAYS[$index]:-}" ] || problem "fare at ${id:-?}: no @pay"
        printf '[plan %02d] fare at %s | ask %s "%s" with its paying cards until paid\n' \
          "$number" "${id:-?}" "$(journey_field "$text" 1)" "$(journey_field "$text" 2)"
        while IFS= read -r spec; do
          [ -n "$spec" ] && printf '          a card with thing %s pays: world inventory pay <each hero> %s %s why=%s\n' \
            "$(journey_field "$spec" 1)" "$(journey_field "$spec" 2)" "$(journey_field "$spec" 3)" "$(journey_field "$spec" 4)"
        done <<< "${STEP_PAYS[$index]}" ;;
      shop) printf '[plan %02d] shop at %s | ask %s "%s" until done; Buy on the phone, haggling by talk\n' \
          "$number" "${id:-?}" "$(journey_field "$text" 1)" "$(journey_field "$text" 2)" ;;
      cast) casts=$((casts + 1))
        printf '[plan %02d] cast at %s | ask %s "%s" | wait for a cast in the inbox, its summary read by the clue judge\n' \
          "$number" "${id:-?}" "$(journey_field "$text" 1)" "$(journey_field "$text" 2)" ;;
      board)
        board=$((board + 1))
        printf '[plan %02d] quest board %s at %s\n' "$number" "$(journey_field "$text" 1)" "${id:-?}"
        for clue in $(journey_field "$text" 2 | tr ',' ' '); do
          if [ -z "${QUEST_SPEC[$clue]:-}" ]; then problem "the board offers $clue, which has no @quest"; continue; fi
          printf '          world quest offer "%s"\n' "$(journey_quest_word "$clue")"
        done
        printf '          ask party "Which quest do you take?" one card per quest (thing = quest id); typed words: persona quest pick <hero> <line>; world quest accept <id> <heroes>\n' ;;
      riddle)
        riddle=$((riddle + 1))
        for clue in $(journey_field "$text" 5 | tr ',' ' '); do
          [ -n "${CLUE_AT[$clue]:-}" ] || problem "riddle clue $clue is gathered at no stop (@clue)"
        done
        case "$(journey_field "$text" 6)" in stalled|none) ;; *) problem "riddle hint policy is stalled or none" ;; esac
        printf '[plan %02d] riddle %s at %s | answer %s | clues %s | hints %s\n' "$number" \
          "$(journey_field "$text" 1)" "${id:-?}" "$(journey_field "$text" 3)" \
          "$(journey_field "$text" 5)" "$(journey_field "$text" 6)"
        printf '          persona riddle ask <riddle>; dm the door; loop: ask party, persona story clues, persona riddle judge -> solved|close|wrong; stalled -> persona story narrate %s stalled\n' "${id:-?}" ;;
      teleport)
        teleport=$((teleport + 1)); PLAN_REGION=$(journey_field "$text" 1)
        printf '[plan %02d] teleport to %s: world region enter "%s | %s | %s | %s"; title card up until the next stop is read\n' \
          "$number" "$PLAN_REGION" "$PLAN_REGION" "$(journey_field "$text" 3)" \
          "$(journey_field "$text" 2)" "$(journey_field "$text" 4)" ;;
    esac
  done
  for id in "${!QUEST_SPEC[@]}"; do
    for clue in $(journey_field "${QUEST_SPEC[$id]}" 6 | tr ',' ' '); do
      [ -n "${CLUE_AT[$clue]:-}" ] || problem "quest $id names clue $clue, gathered at no stop"
    done
  done
  [ "$stops" -ge 5 ] || problem "only $stops stops"
  [ "$board" -ge 1 ] || problem "no quest board"
  [ "$riddle" -ge 1 ] || problem "no riddle"
  [ "$teleport" -ge 1 ] || problem "no teleport"
  for id in "${!NPC_ID[@]}"; do
    case "$seen" in *" $id "*) ;; *) problem "@npc names $id, which is no stop" ;; esac
    printf '[plan] %s at %s knows: %s\n' "${NPC_ID[$id]}" "$id" "${NPC_KNOWS[${NPC_ID[$id]}]:-nothing told}"
  done
  for id in "${!NPC_KNOWS[@]}"; do
    case " ${NPC_ID[*]} " in *" $id "*) ;; *) problem "@npc_knows names $id, which is no @npc" ;; esac
  done
  printf '[plan] every ask waits %s s (a shop %s s), then the seat is told and the first card is taken\n' \
    "$JOURNEY_ASK_SECONDS" "$JOURNEY_SHOP_SECONDS"
  printf '[plan] stops=%s checks=%s (rogue=%s wizard=%s) lines=%s casts=%s clues=%s quests=%s\n' \
    "$stops" "$checks" "$rogue" "$wizard" "$talks" "$casts" "${#CLUE_AT[@]}" "${#QUEST_SPEC[@]}"
  if [ "$problems" -eq 0 ]; then
    printf 'PLAN RESULT PASS\n'
    exit 0
  fi
  printf 'PLAN RESULT FAIL (%s problems)\n' "$problems"
  exit 1
}

play_journey() {
  local index step kind text began=$SECONDS
  [ -n "$PLAN" ] && journey_plan
  if ! journey_seats; then
    printf '[prepare] failed: seats: no seat with a hero (DF_RUN_PLAYERS and DF_RUN_HERO_<n>_ID, or DF_RUN_PLAYER)\n'
    exit 3
  fi
  printf '[journey] %s: %s steps, %s stops, seats %s (%s)\n' "$JOURNEY" "${#STEPS[@]}" "$STOP_TOTAL" \
    "${SEATS[*]}" "$(journey_heroes)"
  if [ -n "$REFS_MAX" ] &&
    timeout 30 $D dev --port $P persona watch </dev/null 2>&1 | grep -q 'refs_max'; then
    WATCH_REFS="refs_max=$REFS_MAX"
  fi
  if ! prepare_clean_tv || ! journey_prepare_heroes || ! journey_prepare_tills ||
    ! journey_prepare_story || ! journey_prepare_plate; then
    printf '[prepare] the journey does not start: the party could not be made ready\n'
    exit 3
  fi
  JOURNEY_STAMP=$(date +%m%d%H%M)
  for index in "${!STEPS[@]}"; do
    STEP=$((index + 1))
    step=${STEPS[$index]}
    kind=${step%% *}
    text=${step#* }
    seat_drain_talk "$STEP" pause
    case "$kind" in
      stop) journey_stop "$text" "${STEP_CARDS[$index]:-}" ;;
      dm) journey_dm "$text" ;;
      ask) journey_line "$text" "${STEP_CARDS[$index]:-}" line ;;
      talk) journey_line "$text" "${STEP_CARDS[$index]:-}" talk ;;
      check) journey_check "$text" "${STEP_CARDS[$index]:-}" "${STEP_PAYS[$index]:-}" ;;
      fare) journey_fare "$text" "${STEP_CARDS[$index]:-}" "${STEP_PAYS[$index]:-}" ;;
      shop) journey_shop "$text" "${STEP_CARDS[$index]:-}" ;;
      cast) journey_cast "$text" "${STEP_CARDS[$index]:-}" ;;
      board) journey_board "$text" ;;
      riddle) journey_riddle "$text" "${STEP_CARDS[$index]:-}" ;;
      teleport) journey_teleport "$text" ;;
    esac
  done
  journey_missed "$STOP"
  stop_watch quiet
  printf '[journey] ended after %s s at %s\n' "$((SECONDS - began))" "${here:-?}"
  for index in "${SEATS[@]}"; do rm -f "${RULES_SHEET_OF[$index]:-}"; done
}
if [ -n "$JOURNEY" ]; then
  play_journey
  exit 0
fi

if [ -n "$ROAM" ] && [ -n "$EXPLORE" ]; then
  play_explore
elif [ -n "$ROAM" ]; then
  play_roam
fi

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
# Played by a seat's hero, a plate is its hero version (HERO_PLATE, gauntlet_hero_plates) once made.
gauntlet_plate_of() { # station id
  local plate=${STATION_PLATE[$1]:-$(gauntlet_field "$1" plate)}
  [ -n "$plate" ] && plate=${HERO_PLATE[$plate]:-$plate}
  printf '%s\n' "$plate"
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
  local ask card left_out=""
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
    IFS='|' read -r id who scene goal ask card <<< "$spec"
    id=$(gauntlet_trim "$id"); who=$(gauntlet_trim "$who")
    scene=$(gauntlet_trim "$scene"); goal=$(gauntlet_trim "$goal")
    ask=$(gauntlet_trim "$ask"); card=$(gauntlet_trim "$card")
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
        [ -z "$goal$ask$card" ] || gauntlet_problem "station $id: a narrator station has no fourth field"
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
      printf '                   from a seat: players ask <seat> kind=line "prompt=%s"%s, then persona say text=<the answer>\n' \
        "${ask:-What do you do?}" "${card:+ \"card=Do it|$card\"}"
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
  # Beats that play part of the stations file (`@gauntlet_subset`) leave the rest out on purpose.
  for id in $(gauntlet_ids); do
    case "$seen" in
      *" $id "*) ;;
      *) if [ -n "$GAUNTLET_SUBSET" ]; then
           left_out="$left_out $id"
         else
           gauntlet_problem "station $id of the stations file is not in the beats"
         fi ;;
    esac
  done
  [ -n "$left_out" ] && printf '[plan] left out (@gauntlet_subset):%s\n' "$left_out"
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

# A gauntlet started for a seat's hero (`DF_RUN_PLAYER`, `DF_RUN_HERO_*`, as the host starts the
# painter's day): the hero (`hero-<id>`) is the character in place of `@gauntlet`. Its persona is
# kept or written (`@gauntlet_hero_hint`), its rules sheet is the seat's derived creation sheet
# (`creation sheet <seat> out=<file>`, the file `persona check run` reads), the prep names it by
# its locked look and reference sheet, and every station plate is its hero version. Prints
# `[prepare] ..` lines; a step that fails returns non-zero and the gauntlet does not start.
gauntlet_hero() {
  local out
  GAUNTLET=${DF_RUN_ROAM:-hero-$DF_RUN_HERO_ID}
  printf '[prepare] hero %s plays the gauntlet as %s (%s)\n' "$DF_RUN_HERO_ID" "$GAUNTLET" \
    "${DF_RUN_HERO_NAME:-?}"
  prepare_clean_tv || return 1
  prepare_hero_persona "$GAUNTLET" "$GAUNTLET_HERO_HINT" || return 1
  GAUNTLET_PORTRAIT=${DF_RUN_HERO_SHEET:-}
  if [ -z "$GAUNTLET_PORTRAIT" ] || ! library_holds "$GAUNTLET_PORTRAIT"; then
    prepare_failed reference \
      "the hero's reference sheet (${GAUNTLET_PORTRAIT:-none named}) is not in the library"
    return 1
  fi
  GAUNTLET_LOOK=${DF_RUN_HERO_LOOK:-a $DF_RUN_HERO_SPECIES $DF_RUN_HERO_CLASS}
  # How the hero looks, in words, for every hero plate besides the reference sheet, so what the
  # sheet may leave out (a blazing aura) is in every scene: the look studio's details for the seat's
  # hero (`look hero <seat>`, `details=`), else the locked design the host passes.
  HERO_LOOKS=$($D dev --port $P look hero "$ROAM_PLAYER" 2>/dev/null | sed -n 's/^details=//p' | head -1)
  [ -n "$HERO_LOOKS" ] && [ "$HERO_LOOKS" != - ] || HERO_LOOKS=${DF_RUN_HERO_LOOK:-}
  printf '[prepare] looks for the plates: %s chars\n' "${#HERO_LOOKS}"
  GAUNTLET_SHEET="$(dirname "$FRAMES_LOG")/sheet-$GAUNTLET.json"
  mkdir -p "$(dirname "$GAUNTLET_SHEET")"
  out=$($D dev --port $P creation sheet "$ROAM_PLAYER" "out=$GAUNTLET_SHEET" 2>&1) ||
    { prepare_failed sheet "$out"; return 1; }
  [ -s "$GAUNTLET_SHEET" ] || { prepare_failed sheet "no sheet was written: $out"; return 1; }
  printf '[prepare] sheet of seat %s: %s\n' "$ROAM_PLAYER" "$(printf '%s\n' "$out" | head -1)"
  # The hero is the party on the TV, its hit points and armor class from its sheet (the host's).
  out=$($D dev --port $P scene party add "${DF_RUN_HERO_NAME:-$GAUNTLET}" \
    "character=$GAUNTLET" 2>&1) || { prepare_failed party "$out"; return 1; }
  printf '[prepare] party: %s\n' "$(printf '%s\n' "$out" | head -1)"
  gauntlet_hero_plates
}
# The hero version of each station plate, made once per locked look and reused when the library
# holds it (`<plate>-<hero>[-<look key>].png`, as the roam's opening plate is keyed), so the
# reference sheet's character is the one in every scene: an edit of the station plate after the
# hero's reference sheet (`media generate .. base=<plate> refs=<sheet>`, the path the roam's plates
# drawn after the hero's sheet take), or, when the library has no such station plate, painted from
# the station's setting after the sheet. Sets HERO_PLATE; one `[prep] hero plate ..` line each. A
# plate that could not be made stays the station's (the prep finds no hero there, and its stations
# are the narrator's).
gauntlet_hero_plates() {
  local spec id plate name out began from
  local -A setting_of
  for spec in "${STATIONS[@]}"; do
    IFS='|' read -r id _ <<< "$spec"
    id=$(gauntlet_trim "$id")
    plate=$(gauntlet_plate_of "$id")
    [ -n "$plate" ] && [ -z "${setting_of[$plate]:-}" ] &&
      setting_of[$plate]=$(gauntlet_setting_of "$id")
  done
  while IFS= read -r plate; do
    name="${plate%.png}-$GAUNTLET${DF_RUN_HERO_LOOK_KEY:+-$DF_RUN_HERO_LOOK_KEY}"
    if library_holds "$name.png"; then
      HERO_PLATE[$plate]=$name.png
      printf '[prep] hero plate %s kept\n' "$name.png"
      continue
    fi
    began=$SECONDS
    if library_holds "$plate"; then
      from="edit of $plate"
      out=$(timeout $HUNG $D dev --port $P media generate \
        "The same scene as the base picture, everything in it kept as it is, except that $GAUNTLET_HERO_PLATE${HERO_LOOKS:+ How they look: $HERO_LOOKS}" \
        "as=$name" purpose=plate "base=$plate" "refs=$GAUNTLET_PORTRAIT" wait=true 2>&1)
    else
      from="painted from the setting, no $plate in the library"
      out=$(timeout $HUNG $D dev --port $P media generate \
        "${setting_of[$plate]} In this scene, $GAUNTLET_HERO_PLATE${HERO_LOOKS:+ How they look: $HERO_LOOKS}" \
        "as=$name" purpose=plate "refs=$GAUNTLET_PORTRAIT" wait=true 2>&1)
    fi
    if library_holds "$name.png"; then
      HERO_PLATE[$plate]=$name.png
      printf '[prep] hero plate %s made in %s s (%s, after %s, looks_chars=%s)\n' "$name.png" \
        "$((SECONDS - began))" "$from" "$GAUNTLET_PORTRAIT" "${#HERO_LOOKS}"
    else
      printf '[prep] hero plate for %s FAILED(%s): the station plate stays\n' "$plate" \
        "$(printf '%s\n' "$out" | head -1)"
    fi
  done < <(gauntlet_plates)
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
  local ask card note="" given=()
  IFS='|' read -r id who scene goal ask card <<< "$2"
  id=$(gauntlet_trim "$id"); who=$(gauntlet_trim "$who")
  scene=$(gauntlet_trim "$scene"); goal=$(gauntlet_trim "$goal")
  ask=$(gauntlet_trim "$ask"); card=$(gauntlet_trim "$card")
  # Talk the phone sent between stations is taken before the next one begins.
  caption_down
  seat_drain_talk "$number" pause
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
      # Played from a phone: the seat says his line (the station's ask and one suggested card,
      # the goal as a first-person act), spoken as written; the station's time runs from then.
      given=()
      if [ -n "$ROAM_PLAYER" ]; then
        seat_line "$number" "$ROAM_PLAYER" "${ask:-What do you do?}" "$note" \
          "${card:+Do it | $card}" line
        given=("text=$SAID_LINE")
        GAUNTLET_DEADLINE=$((SECONDS + GAUNTLET_LIMIT))
      fi
      out=$(gauntlet_timed $D dev --port $P persona say "$GAUNTLET" speak=true "${given[@]}" \
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
      note="Say plainly what you do, as you do it."
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
  local number=0 spec began=$SECONDS usage hero=""
  [ -n "$PLAN" ] && gauntlet_plan
  # Started for a seat's hero, the hero plays it; without the hero the gauntlet does not start (the
  # host reads the status, as for the painter's day).
  if [ -n "$ROAM_PLAYER" ] && [ -n "${DF_RUN_HERO_ID:-}" ]; then
    hero=yes
    if ! gauntlet_hero; then
      printf '[prepare] the gauntlet does not start: the hero could not be made ready\n'
      exit 3
    fi
  else
    # The TV starts clean for the dev run too (#68: an earlier fixture's title and cards stayed).
    prepare_clean_tv
  fi
  # The phone's talk and the answers to it name the seat's character (seat_talk, seat_reply).
  if [ -n "$ROAM_PLAYER" ]; then
    ROAM=$GAUNTLET
    SEATS=("$ROAM_PLAYER")
    HERO_OF[$ROAM_PLAYER]=$ROAM
  fi
  printf '[gauntlet] %s: %s stations, fade %s ms, breath %s s, limit %s s a station%s\n' \
    "$GAUNTLET" "${#STATIONS[@]}" "$GAUNTLET_FADE" "$GAUNTLET_BREATH" "$GAUNTLET_LIMIT" \
    "${ROAM_PLAYER:+, played from seat $ROAM_PLAYER}"
  [ -n "$hero" ] ||
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
  [ -n "$hero" ] && rm -f "$GAUNTLET_SHEET"
}
[ -n "$GAUNTLET" ] && play_gauntlet

stop_watch
