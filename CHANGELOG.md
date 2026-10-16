# Changelog

Record every integrated commit once, in six-hour work blocks. Close a shorter
final block when the work session ends; put the newest block first and retain
completed blocks. Record timezone-aware boundaries, starting/ending revisions,
and commit hash/subject in integration order, including merges and root commits.
Use revision ranges, not author-time filtering. The first baseline is empty;
subsequent blocks begin at the prior ending revision. Exclude dungeonflux.old.
The coordinator is the single writer.

A changelog commit cannot record its own hash. Close a block at the reviewed
integration commit, then commit this record separately. Carry that bookkeeping
commit into the next block: keep the recorded ending revision as the next
baseline and capture baseline..HEAD, which includes the bookkeeping commit.
Do not invent a self-hash or silently discard metadata commits.

## 2026-10-16T10:06:52-04:00 to 2026-10-16T10:33:59-04:00

Range: e881a61e02ec62ea0047f7f854da9e29c951997c..994b39044549af9c15682958ab83b5083c7160fd

- 3ebe5e251f66ecc6352ccdc9eb62d3074d1cb705 — Record integrated dialogue timing and scene-memory design work block
- cf2c4cc15070e107e3133422db3ef74f9ed7191a — Route character dialogue through portrait panel
- 6c093eb85d0907097d5e14c92bb1c9bcc884a6b4 — Refresh portrait dialogue regression coverage
- ec334de7bdc11673517ffd0eea36fcaf71af0368 — Clear portrait dialogue stage markers
- 994b39044549af9c15682958ab83b5083c7160fd — Document portrait dialogue presentation and live retest

Next block baseline: `994b39044549af9c15682958ab83b5083c7160fd`.

## 2026-10-16T06:39:31-04:00 to 2026-10-16T10:06:52-04:00

Range: 4f9a7516b5f73c61818afb67dd8a5480b1431325..e881a61e02ec62ea0047f7f854da9e29c951997c

- d99afc5f6a214927c83fe9865071b2baa5cab7eb — Record HUD camera and dialogue refinement work block
- 73cd54a5d29893a2e6a4a2e5c0e41aa1ad2a0a1e — Ground NPC answers and preserve intentional withholding
- 5b1a3e0e9403e940f3d198d42b148e46d2859c61 — Keep investigation targets grounded in authored scene objects
- d9b9573613a8e8eb37f53843414a5379cc26f8d4 — Narrate roll results and preserve readable plate presentation
- cd046bb6f8733a3bea34b89f637bf0fa0175e78d — Exercise grounded replies through valid conversation fixtures
- 6adb3e5a1a846e661f77046db5725b88070d9c81 — Cover direct questions about following the courier
- 28e9415578b9f84e8b6c0547342b2d55555497aa — Accept empty directory markers when restarting gameplay
- 52af08583772903c62b10af6b3316ec3f856c9a6 — Speak check findings after reveal and retire managed results
- 7b81b3083bb80381d2971d8c08fcf3f4ad6efb71 — Direct photographic plate coverage from public story beats
- ac4575e9545ad035cc45609b4fa0abbfb1ae324a — Show the accepted intention and complete clue on check cards
- f6d69ecdee425d8228bc6562479602b861216b7d — Vary dialogue coverage with public speaker continuity
- 7a7cc13cb5356372d7533bc1d5c56d5b101b1ac3 — Track filmic coverage from successfully published public story shots
- d4109e5f3efbc95368eb571b345932300a066208 — Recompose check cards around intention and a bronze die rail
- 74a3adcfe31362ed93adacfb0ce945a214c21e1d — Compile the settled check clock query only for the browser
- 31a7527bbcb6aaa636f5089ca176a6b40df78a5d — Keep measured plate serialization fixtures independent of camera policy
- 88a7febed0a2a32f6e79fcf85d36301e317e863e — Wire intention signatures and browser regression imports
- 9162f2fdbe06d065480349abdace37a3d1ba170a — Use the standard bounded camera cadence predicate
- 16877d072436728cdb94f45eb281b2f18a5ebb2e — Tighten check intention grouping and remove redundant roll chrome
- e29c9fceee90333c972cd0917ec1ff964c563b92 — Record check retirement with the shared reading reason
- e3caa3f1ad6664e0549ccaea60d71171325e86ed — Resolve caption and voice speakers through canonical case-insensitive names
- 97eeb9e69dc93c85277c0a395ec80700b0d71f42 — Give managed check results their own voice and reading lifetime
- 30a4857ab698dded5b06440a16ce850b50bfa786 — Authorize NPC name disclosure from authored identity facts
- fc916aea8335ef7c3e58d7bdbfcef51a5df5c21f — Fade managed checks and retire competing reconnect context
- a99bcf038e35547d96f4c5aecd3ede9b62c3732a — Preserve check-card entrance during reveal and DM handoff
- e881a61e02ec62ea0047f7f854da9e29c951997c — Document scene observation memory and integrated playtest evidence

Next block baseline: `e881a61e02ec62ea0047f7f854da9e29c951997c`.

## 2026-10-16T02:22:10-04:00 to 2026-10-16T06:39:31-04:00

Range: 33f5b3e51473781b300186f552bd0cad6feb10e2..4f9a7516b5f73c61818afb67dd8a5480b1431325

- 74c3986101614c7d493520f9366813cfa01b2dca — Record integrated dialogue routing work block
- facc4ca64413495533ca8a11eedf9d64d8a905fb — Redesign party HUD and phase fades
- 69dc4541ae30e737899cecebd82011c02e46a1c0 — Harden party HUD phase fades
- b9fc216bc5fc012c9998e0ee7c9f33d793347793 — Refine check-card readability and cinematic camera easing
- 0996445c6d285fdba82b6a9f5762a125e1dfd619 — Refine grounded conversational replies and add natural dialogue canary
- 4f9a7516b5f73c61818afb67dd8a5480b1431325 — Defer SAM character localization as a speech placement enhancement

Next block baseline: `4f9a7516b5f73c61818afb67dd8a5480b1431325`.

## 2026-10-15T22:31:46-04:00 to 2026-10-16T02:22:10-04:00

Range: 4a80dbb32322b829c7bbfaba473784d21ee51cee..33f5b3e51473781b300186f552bd0cad6feb10e2

- b66273f7f0203c860e1df74ab8c565539e543738 — Record integrated dialogue visual and diagnostics work blocks
- 9505c01360aa9096e031fdb5034883e63959cb8c — Constrain dialogue music to tempo and scene
- be01ddf0cac1ddcc1aaf644fc954c90b95b7d3c6 — Compile the shared music score for native diagnostics
- c0bdded0b1048721553c0493c93acf666e625e0a — Keep browser-only reading helper behind target cfg
- fc2b3caadf9df13fd3b31f6f226efe46d91984c3 — Keep dialogue readable and clear fixed bubble obstacles
- ac1304057019191b29acc772fdbd75e108d8da73 — Keep long dialogue visible and bubbles separated
- 9bdaa4e540da2d185b6d8bf8d03c45a8060d003e — Keep dialogue visible during readable check cards
- 2169c7c7176a24773ad692cc1a678b66cd9a42f9 — Queue check cards through readable result phases
- ab6badce6b498fcb0c3fd1cc5ab7bc941ae33c82 — Keep reconnect context clear of dialogue HUD
- 50134533be02647601c09af7c675a13617fa9e61 — Crossfade decoded scene plates and retain canonical photographic cast references
- 1132df8326e0b0c8500045e09f2de7b415c13b15 — Move reconnect context into lower caption lane
- ee6eeebe8c83cb66af071b7f9286274b636a3e31 — Keep settled plate jobs balanced after boxing identity snapshots
- 942c20182959f9ad5c798dec76d7cfbf35e21b19 — Record dialogue readability and live layout regression baseline
- 3ca2f4b57700dd9103c8c7801de94d261aea50a9 — Add presentation component lifecycle diagnostics
- abee6de0b5dd63ec4e2106479cb4ae00af0a2e75 — Trace presentation element lifetimes
- 91002add580d6645a5cbbe9483f40051de0c422d — Document live layout controls and component lifetime inspection
- 52a92cbdba43fe61b6e2f38aba0d5fa6724ce6aa — Save SAM API feasibility and access probes for later
- 74a7726344beacf86c41fc7df112e16eccec125a — Index layout ownership and cast continuity sources
- 5b59a17b81821bbbcc585e93e0a448eeb34276c6 — Add authenticated live display layout controls
- db015c5d3225926f036fcbfc2cdf82412ad56060 — Repair live layout readback and style preservation
- fbfc587c5bee6f6e41d680e839cf1a44188c6a37 — Complete typed live layout inventory, transforms, and per-display acknowledgments
- 4e00e958c596a3db1ef99def3a51b38a7b095c0d — Bound live layout readbacks to paginated RPC budgets and verify layout ownership
- c0ea5b1fee2eac1d6aca460982e3fc8c2537af18 — fix(display): keep decoded plate work on the owned clock
- 1bcb4d448b346a2f44e685e4de13f9107c843f16 — fix(display): retire check events and keep long lines readable
- 8fd307b8f80addbf2a508647502a3851029b7111 — Record live layout acceptance and presentation lifecycle regressions
- b6cb696d19ab1722ae7803f05684220122cd5ea8 — fix(display): refresh pinned plates for committed story beats
- db88e3faa6b13a942733825315e7de8fffc4cb5e — Record automatic plate refresh and caption timing acceptance
- 312aa2352189fe693c7373cc93ddaa8725834888 — Route NPC replies by canonical addressee and locate every plate actor
- 8eec2b6b532250d8653738bf3d54fb2b8976d038 — Keep failed plate measurements transient and bounded
- e1ec5e03045cfe2815866e17eaf9de26d514eac8 — Mirror plate head diagnostics and latch late bitmap geometry
- 9b42661f639508ac959c24397ca5f4f267a6e74b — Distinguish absent addressees from focus and discourse in Jev routing
- 0187f83e482680be669b7e693b877914eddedeae — Place plate captions close to their own measured speaker
- 33f5b3e51473781b300186f552bd0cad6feb10e2 — Record Jev routing, head traces, and speaker placement acceptance

Next block baseline: `33f5b3e51473781b300186f552bd0cad6feb10e2`.

## 2026-10-15T19:46:19-04:00 to 2026-10-15T22:31:46-04:00

Range: 3d7cd9696743eab2db27d06f71dd7b9bf9082a22..4a80dbb32322b829c7bbfaba473784d21ee51cee

- 5da6068965d5bfd2d022c7a31c531566e3f40154 — Fix paid credential imports in gameplay launcher
- d8f16998d7774dc20d81e802d37e69440e762a04 — Keep display speech clear of wrapped choice labels
- 45e33a1c0574bc33e226af2d552ac04ca3ebb9f3 — Make display voice media observable and attached
- 79b7bbd895867b06b5526b905f3b60405662d989 — Test second-player archer creation flow
- a2327cddd054eed92d6ca2c0c7cbe6e53fcb7fd4 — Enable generated plates for harbor scenes
- dae5b98b69773c7ae9e8e208d4ec6bff19345119 — Keep scene plate requests within provider reference limit
- 1800cf5e6f058934e7f799ce0001e6aa5dc22b2f — Mute display music output immediately
- 37ace03e8be0199b207a42fadf0a164644716c83 — Keep scene cast visible outside the battle lifecycle
- b9eb4f0cd71cd90880fb96b301c85cc860353254 — Keep local rejection receipts within the shared bound
- e7a713117daf823186e416492eff3cf8c0dc1737 — Make dialogue play quick-start create valid heroes
- f2ac4ed5b3364a3506846379b6ea99550158dad7 — Record dialogue quickstart and display regression findings
- 589afa713ea27b5141597a68bf8c54dc5f18c7f2 — Make scene plate references style neutral
- a6cdf22c5670eb10bdf4681d31729e64da7ca06d — Exclude painted art from photographic plate references
- 6002bb15c4f9351c2fd91b97c52f49f3e88afabd — Fix photographic plates without identity references
- 614fade5a901c90bb746f6118a6705c1f354bd2b — Use photographic scene and hero fallbacks
- b766b8342a04013bedd35cee212444a5cf723d16 — Keep existing fallback for hero three
- 9fc47616d9e00ceb10d92d0c4f29c6394842ac44 — Serve photographic fallbacks through generated routes
- bce05f717fa05c4d7a1b20acc5b9c02a31f431ad — Route all dialogue cast art to photographic variants
- 6ce95f95b6aa17c33ce6f2330ce1e7d41dbb0557 — Prefer photographic art in the lobby
- 179e9c85a5105c3f7a235f292b42c57e5de2c3f3 — Keep dialogue fallback photographic
- 7b08c2574b4996b234f22408080558654368f349 — Use medieval photographic sprites and harbor
- a3ecd0f87a5e79bc954a990895562a0dd4ae8ab1 — Prefer photographic sprites in the live stage
- eb4ba5bcf0fa3e639fcba63af958708bddc306a2 — Remove illustrated stage fallback candidates
- bbe08771dc2660f098844b7668528f5a3711949e — Fix browser photo candidate type
- fb50e7a3e8f9dc058b3f307589ad9b7113cca8d5 — Direct plates toward candid cinematography
- 67cf0d8b30ff6aadfda7f599571b3fd91a6fe251 — Add visual pipeline diagnostics
- 790bbd10c398acc76d749545472a5d70a6b55c6c — Use display plate revision in diagnostics
- 1e6d0d5007bc796d6911bb93617c23f2316f0ebd — Replace polished CGI references with documentary photographs
- 97f88c5651faf16a8ab03a06572d5ed2c76fba80 — Scope documentary cast fallback to its harbor scene
- c0dbaad1e66db2f10d963b58b78f4b121e5a5b02 — Document visual resource inspection workflow
- b0dcecefd3eb0476c53c663d18958929477a2aa2 — Clarify conditional imagery provenance
- bd8643c87e8122df2f515dc1c98a54dd64bf564f — Document visual trace identity gap
- 337ec64ee0b535b76601fb3dcf02ad5bc2f3848a — Add correlated runtime resource timeline
- 02e8e656eecfe3d1ed0e63f73c91c2515332bbd4 — Edit the candid harbor photograph as the runtime plate base
- ee633e73eff4f91d0aba4882852c5242b6630d13 — Document resource timeline filters and gaps
- 196d72e18156e6a92d7467001c53606287e57c85 — Repair diagnostics test and index photo edit source
- a45dcca99bbb2520e97edfddb1759be80ad19a81 — Link visual resource manual from repository overview
- a7d0c6365eda1ba52cf7cb9b5e93079826571774 — Keep both heroes in the photographic harbor edit
- 4a80dbb32322b829c7bbfaba473784d21ee51cee — Record photographic plate acceptance and cast continuity gap

Next block baseline: `4a80dbb32322b829c7bbfaba473784d21ee51cee`.

## 2026-10-14T20:33:27-04:00 to 2026-10-14T21:23:21-04:00

Range: 5e52f893f54fa146673dc9a9e6c18e875f66d818..3d7cd9696743eab2db27d06f71dd7b9bf9082a22

- 67d54be14387950b5352cf35c6a9acf461fc1981 — Integrate the day's remaining work: characters who know their room, a reachable ending with reward and XP, heroes defined by menus or text, SRD rules, the TV at a person's pace
- c1bfa27745866298f995883a8b96cc4cef5b2d2d — Backlog: a full session on the pushed build (painterly pictures from painted references, the opening narration repeated, a search that describes nothing, a true answer thrown away, exploring buried on the phone, the item card without its picture)
- ac877f29612d28b48ca4b25862613e69b3895777 — CRE-10: the phone's hero creator loads again and a described hero can be confirmed
- 10d63be2a30799973601a969d8ec17f47eefd7bb — Backlog: the rest of the session (the promised gold is not paid, stock sentences repeat, the named kobold never fights, a subtitle on the wrong person, an honest rest label, the contact not knowing her part)
- 8af367769194fe92e824283c7d51a8153a966555 — Backlog: the scene picture never changes (every plate edit refused as invalid_request; the picture budget spent on health cut-outs)
- 5c3ea45d5aa79fd251ea2eae287f02fda40a539a — Backlog: genre-specific loaders and spinners (the waits the game has and which loader fits each)
- 024f395214880a6c5fc2aebeabab2613cae18abe — Backlog: a benchmark of NPC dialogue and scene exploration (the story model's budget runs out after about ten replies, slow replies end in stock lines, questions about things get their cards, check cards flash past under quick play)
- 3c54ca999d4e30398c8ad998af0b14fcad4d0c22 — No default spend or rate limits: characters keep answering after the tenth reply
- 3d7cd9696743eab2db27d06f71dd7b9bf9082a22 — Backlog: the TV shows no text while a check card is up; DLG-11 and DLG-12 fixed by removing the limits

Next block baseline: `3d7cd9696743eab2db27d06f71dd7b9bf9082a22`.

## 2026-10-14T13:38:07-04:00 to 2026-10-14T16:53:08-04:00

Range: 27d0e2f127476f8011df25982f1bb707f5ea4811..5e52f893f54fa146673dc9a9e6c18e875f66d818

- 58cf6e4b507dbe17be2a81fb6c62e90c1b101566 — Triage: a fight and ending from the phone on HEAD (the fight turn is a form; the ending turns the errand round; no reward on the end card; the epilogue's contact drawn twice)
- 64560aede20217b24cce4d958c0fb9fa83d75ca3 — Triage: the hourly independent review's rules findings (TV and phone hit points disagree; a rest promised but not offered; range wording)
- 5ef2b09229da483f7c19cf034350164d1618d47b — Triage: the phone's hero sheet, notebook and menu (a hook in the reward's place, one place twice, sheets that block the tab bar, no saves listed)
- f1794dbfa75757afc92481244e923c1241a4aea8 — Filmic, not painterly: one shared film look for every picture, clip and screen
- c855bcfdddc0a15dc2225bbe8b119e80332de4d2 — PHN-12: the phone and TV show 'Setting the tableâ€¦' from the first paint instead of a blank black page
- a6385f4aa1894a6159d0f10272a1a5587be975be — DEV-11 (part), PIC-16: picture and voice timings in the server log, line timings kept on the TV, no cut-out drawn over its own picture
- 675020f058170a083f2f7143d6d43e3169b0a23e — SYN-10, RUN-04, SYN-12, SYN-13 (guard), SYN-08 (part): the TV fight frames both fighters, asks for a reaction only when they are in view, and plays one fanfare
- 1af7806e3061dbdb0f08755f067db8e200fc6c84 — DEV-09, SES-09 (part): a burst of watchers no longer closes the TV's and phones' view streams
- 5e52f893f54fa146673dc9a9e6c18e875f66d818 — STO-01, STO-02, FLO-05, OBS-17, FLO-02 (part): the generated plot names its job, stake and reward, and only a threat the players have heard of is fought

Next block baseline: `5e52f893f54fa146673dc9a9e6c18e875f66d818`.

## 2026-10-14T06:28:57-04:00 to 2026-10-14T11:09:59-04:00

Range: 126dde132f0b3e6cabdd1e9a645c28e84606c68e..27d0e2f127476f8011df25982f1bb707f5ea4811

- 5d6c96fb08a9f6efe8cc61cc053c0cfa7d5929d5 — ENV-06, ENV-07, PIC-13, MOT-02 (rendering side): the picture is drawn from the scene model and redrawn once per settled change
- 221511f9f63852ba6e3ced158e687f6b81fc3eb3 — SES-05, SES-06 (part): story text is not refused for a forbidden term inside another word, and a slow group no longer holds back the rest
- b44a9198be118cc43bbdb08b33ecef4d1c9b8a09 — RUL-23 (part), DTH-04 (part): Hit Point Dice per hero, spend-or-keep at a Short Rest, a dead hero takes no aftermath turn
- d5d76e2abd8427f1f940ac7989696be724ae346d — TLK-08, TLK-11 to TLK-22, ENV-05, ENV-11, RUL-19, RWD-01 (part): characters answer from this adventure's facts
- ae3f2ffe88fddb7e3534d644161fff86d518b7fa — RUL-04, RUL-16, OBS-44, FGT-10: the phone's fight panel and roll card tell the truth
- 4562905a6b2422610b6d499bfd451ece5380dfdb — Triage: a played session at human pace (ten rows: a dead end after a won fight, unreadable lines, the background replaced every line, feelings that change too often, the check read by no one, silent fights); AGENTS.md: measure scene and interaction timing
- 7ba7ba976fc7400f9d39a0146e7c4cacb3aadd8b — Triage: play-through at human pace from the phone (ten rows: hero creation cannot define a hero, the phone's send button under the tab bar, a blank first load, sound off by default, the patron not knowing his errand)
- 95b58798a7f32bfcffd8ad38b32004c7b0e5e6c7 — STO-04 (part): the storyteller grounds who is present and any injury in the scene's facts
- 0b86bba345685263396518f8f1ed85784fcf5811 — INV-07 (TV side), END-06, TVX-06, TVX-07, PIC-07: the TV shows a check as a roll, ends on one composed card, and keeps its bubbles clear
- 43e3251c1239f2b4a211d7306436c0995f9b5739 — HNT-25, B17 audit: every creature's core stat block pinned to SRD 5.2.1, and what is not modelled is disclosed
- 38826ea2c355b2757de2676f74dd1362e91aa271 — Triage: play-through on Q01 and Q02's work (Ilsa knows her room; a search with no target still rolls; the TV's check card names the wrong hero; a late TV has no context; a purchase reads like a log)
- 151709c547d4a8d120aabc609483eb217bc10cb3 — Triage: ENV-12 is a recorded improvement, not a severity-1 fault
- 8d09c41f264e3a405fa97576928f4c27b50db3f3 — TVX-10, PIC-14 (part), AUD-01 (pacing): lines are read and heard in full, one background per scene, figures stay
- 27d0e2f127476f8011df25982f1bb707f5ea4811 — DTH-01, DTH-05, FGT-03, FGT-04, FGT-06, FGT-08: the phone shows the fight's rulings as they happen

Next block baseline: `27d0e2f127476f8011df25982f1bb707f5ea4811`.

## 2026-10-14T00:53:42-04:00 to 2026-10-14T05:39:59-04:00

Range: 1ad51941a0603222b0ed11f5905750594b3f206d..126dde132f0b3e6cabdd1e9a645c28e84606c68e

- b4e06d086351aa4f4c398c23e9460fef7f701dbd — RUL-14, RUL-17 (part): the latest enemy policy approaches instead of dodging, and a goblin keeps its shield
- 4c919660a35a226e91aa44e02a05ba39bd344c83 — HNT-14 (server half): cancelling a spoken line stops the transcription
- 59ee1ebafed5632080046b3daebdd258d61422af — STO-07, STO-08, RUL-09 (part): the story's own words
- 85ed4b4ff3db21f03ecc6767c0df2d74c6906142 — SYN-13, SYN-14: the TV's health cards never go back, and headings are not replayed
- 6299d7f20265d9d00b59b3062a6b9ffcf7c308ea — INV-03, PHN-11 (part): looking, searching and watching do something, with Jev reading the line
- 3729111d04aa7616fbc180bf94d335b192128dba — Triage: investigation after the first fix, six rows (INV-04 to INV-09)
- 4705e2f0228e9f71fb948e4a2a99e4f241b54a11 — SCA-12: a server that refuses a game stored by an older rules build says how to go on
- 14f2157bb461cc6e95a5859d678d11b58ce46068 — Triage: the speaker does not know her own room; items refused; four rows
- b2f9c4b5a1d3022051841e658a1dfe5fa4f14e03 — FPX-01, OBS-42 (part): the dice are seen landing and stay long enough to read
- 68130965ba74cf38a145f350ffce3def2f04e14d — SES-06 (part): a story text retry tells the model every refused slot's reason
- 9668deca55ae07d8444dae3e687db81baa89f9e7 — NTE-03, NTE-04, NTE-06 (part), FGT-10: the notebook and the table log read like a game
- df97598092695de012bb92917c805cf2c7a8ca38 — MOT-02 (part): every scene picture can become a short looping video of ambient life
- 5fe971bcbd683e6f295995c481d124dc90d930a5 — EMO-01 (part): characters have an emotional life, written by the models and grounded in their history
- d487302e43bef1dd44c529f7bdd8f5223a5e0a6f — HNT-14 (phone half), PHN-04: cancelling a spoken line stops the server too; the send button reads 'Say it'
- b1ab3ea0363fc29cb8499b11b056247a0b870230 — RUL-17, RUL-12: a goblin shoots its shortbow with its shield on; the javelin's Slow is proven
- 14a3611e99181d984fca278be848eafa9db3ecf1 — ENV-01, ENV-06, ENV-09 (part): every scene has one model of its people, things and ways out
- c6d7248200543d9231c64d93bba27d81fdbb4a6c — MOT-03 step 2 (part): the scene graph and a layered stage on the TV
- 66f36415b12aa51813f345c6d89934b577cf0f8d — Fill the scene clip fields in two scene literals so the committed tree builds
- b0040be7bb907d991a490d32d6568e95e44d88d0 — Name the new outcome and setting fields in three test literals so the committed tests build
- d9d6befebfc3bbcb64549ee2462125c672310433 — The server starts again: a scene's setting is bound to the record's by everything but its model
- a41f5c0678721cd5b6a30268374ff0895b44ca6a — CRE-01, CRE-03: making a hero reads like a game, not a developer form
- 79efebe3007dfd09be7449ab12228054bd7deb24 — INV-04 to INV-09 (part): an investigation module that binds every attempt to its target
- da7beec03e1d1ea704ff4c4da0f93d24768c6782 — The eighth failing test: retained courier wire witnesses no longer collide between test processes
- a64fee3a46dc31bd3cb443c144325f655793c8b0 — PHN-10, PHN-11: the phone says plainly what it did with your line
- ad6ed69297d0862116331e96b264088acc0f49de — Triage: HNT-14 fixed by 4c919660
- 7f4b9a30ef68eae6a653d920c706064c5486a961 — FPX-08, FPX-09, RUN-04 (part): upright initiative numbers, labelled status pips, no stars over a fallen hero
- 126dde132f0b3e6cabdd1e9a645c28e84606c68e — A generated adventure up to 1 MiB is stored: the 256 KiB bound refused the grown pack

Next block baseline: `126dde132f0b3e6cabdd1e9a645c28e84606c68e`.

## 2026-10-13T21:24:34-04:00 to 2026-10-13T22:40:03-04:00

Range: 06e7a7e4a618890ec6677566fee5c64db5a2718b..1ad51941a0603222b0ed11f5905750594b3f206d

- fa616c01ce9febe46722ee1f9e8afd94bf098ebd — Changelog: 24 commits from the afternoon and evening of 2026-10-13
- bc36f3b5471ea89244d2e3741fb46e6bc17afbcb — END-06 (part): the TV's end line has no scroll bar
- 90e8d6021d9b11ca3268946263d6fd92f200deda — Triage: anything and anyone in the scene (the owner's direction), six rows
- 68c1b48f5bce28387d7edcd983cc27e0dac0066d — Triage: scene state, items in detail with clues, pointing at the picture (the owner's direction)
- f8d7a95dd71a715b0d1080a5628112083e7b7406 — Triage: the emotional model of characters and enemies (the owner's direction)
- f43d3fa767eb5245399e9f5b1735b89c38d44a92 — TV scenes: a living camera (layer 1 of more motion in the scenes)
- 32767710bfd061f9d2bf34e61d158e26ebb623c9 — Triage: motion in the scenes (the owner's direction), three rows
- 3002c60b4d83a78a3a0fc987dbf9838dde07984f — Plan: scene rendering engine v2, a layered stage with baked frames
- 1ad51941a0603222b0ed11f5905750594b3f206d — Triage: MOT-03 points at the scene rendering plan

Next block baseline: `1ad51941a0603222b0ed11f5905750594b3f206d`.

Integration history through 2026-10-11T15:25:07.071270-04:00, reconstructed from Git at this session close. Six-hour blocks use first-parent integration timestamps; each newly reachable branch commit is listed once in topological order. This history records commits and does not replace task acceptance. Empty intervals contain no commits. The bookkeeping commit writing this record belongs to the next block.

Integration history from 2026-10-11T15:24:05-04:00 through 2026-10-12T04:33:02-04:00 on the integration branch `claude/playable-20261011`, generated from Git by revision range: six-hour blocks by first-parent commit time, each newly reachable commit listed once. The last block is open and shorter than six hours; the work session is still running.

Integration history from 2026-10-12T18:31:36-04:00 through 2026-10-13T06:51:17-04:00 on `main`, generated from Git by revision range `558654d38a30..e5034d48311b`: six-hour blocks by first-parent commit time, 173 commits, each listed once in integration order.

Integration history from 2026-10-13T06:51:46-04:00 through 2026-10-13T10:55:22-04:00 on `main`, generated from Git by revision range `e5034d48311b..cefa2864f788`: six-hour blocks by first-parent commit time, 26 commits, each listed once in integration order.

Integration history from 2026-10-13T10:55:42-04:00 through 2026-10-13T14:04:49-04:00 on `main`, generated from Git by revision range `cefa2864f788..493477ca31cc`: six-hour blocks by first-parent commit time, 14 commits, each listed once in integration order.

Integration history from 2026-10-13T14:16:31-04:00 through 2026-10-13T21:24:24-04:00 on `main`, generated from Git by revision range `493477ca31cc..06e7a7e4a618`: six-hour blocks by first-parent commit time, 24 commits, each listed once in integration order.

## 2026-10-13T20:19:48-04:00 to 2026-10-13T21:24:24-04:00

Range: 2d2504f7252f5a2c898b5f8ec37d33f53423d660..06e7a7e4a618890ec6677566fee5c64db5a2718b

- f3f65bb01620bec869d362976f601d2256968f43 — Triage: dialogue and investigation in depth, and the owner's questions
- 4b4c8dcd6e54f41e78ca512e6b08fd1e6209a16d — TVX-09: a drawn screen while a new adventure is prepared
- 83ea51fd5b230a6f379bcdd463cf69465ccb6557 — Say why a stored game will not load
- bc7461e040fa48d2e7781aeeedeec36aa9196dbd — Triage: the preparing screen is fixed; the TV's end screen, creatures as people, rebuilds ending games
- 801358ba605d9f0a14738e6334be67f0ffbe96c5 — Triage: the TV does show replies as spoken; an attitude change is shown as speech
- 30cbf3599fca466fa5b541b0befb0e61f1f2d485 — Integrate nine workers' changes: notebook, dialogue answers, scan start squares, enemy rules, and the unfinished rest and stabilise flows
- 06e7a7e4a618890ec6677566fee5c64db5a2718b — Triage: the phone sends an attempt as speech (PHN-11); the Short Rest now dead-ends; notebook statuses

Next block baseline: `06e7a7e4a618890ec6677566fee5c64db5a2718b`.

The commit writing this block remains pending for capture in the next block. This block is still open: the work session had not ended when it was written, and it is shorter than six hours.

## 2026-10-13T14:16:31-04:00 to 2026-10-13T19:29:55-04:00

Range: 493477ca31cc41a14d0ac0a32c59787ee9817cfd..2d2504f7252f5a2c898b5f8ec37d33f53423d660

- 6d6b5d29359cc1da0fd344b883ce5385f01f7359 — Triage list, roadmap and changelog: the scanned battlefields looked at with the owner, and the day's other fixes
- 94ba35ab5b0a4154906f3bb8d4158cf15995a710 — Triage list: play checked against the rule book (SRD 5.2.1), first twelve rows
- 7fc89b5056088ff80e204ec3fb5600ec50793608 — Triage list: seven more rows from play checked against the rule book (stabilising, enemies that only Dodge, a pocket start square, hidden dice, the goblin's shield)
- 45ea9a22aa89eee01ab2fa8ac154e906c5474b95 — SYN-01, RUN-03, SYN-07: the TV shows every blow of an enemy phase in order, each under its own heading, with no stall
- 147a995e06303d0a83b712b740596e4a4ee2bca8 — END-02, OBS-53: after "The End" a seated player can start a new adventure
- 0086ff8ecc1e81210d008ed624452244224e5941 — RUL-01: a wolf's and a mastiff's bite knock a Medium or smaller target prone on a hit, with no saving throw
- b5117626a5391fc1dfd94e41071f3edecdeb2e75 — Triage list: a new adventure after The End seen working; the enemy phase shows every blow but takes 49 seconds
- b9beccc0a3d554d882f8447d10a01a0ab7d36dc1 — Triage list: what agreed with the rule book in ten creature test fights and an archer's game
- c775d7b29e3e99d27587ad1fd2b02a1d4881a587 — Triage list: the Influence action, Search and the unoffered actions, checked against the rule book
- e5a7f734ee563e667001bca6a95dd0f9f61bbe9a — RUL-08 (rule only): the Short Rest's Hit Point Die healing rule
- 455cd56e1d9b6392fc5e49bfabce1bba9ce44977 — Triage list: dialogue, investigation and notes played with the story model on (ten rows)
- 60a76918ee1ee7cef9af1901a0a6758ef0129644 — RUL-02, OBS-37: enemies take opportunity attacks; a stored fight replays under the policy it began with
- 91f5f24c23e006763b2e1b24025fffa6b5dfb2b4 — Triage list: a lost fight played to its end (death saves, a dead hero's player, a raw rules label); opportunity attacks fixed
- 4a03f813f0422f62a42a0596e964434d2b29e58f — Triage list: characters are not told what happened in the game; the villain has no stable identity
- ca629d0007201827df4bd5d714d7ad87d93278a5 — SYN-01 (second pass): an enemy's turn on the TV takes about four seconds, and nothing replays after the last blow
- 0d95857b3ebf78ccb192153078edd09745016fc7 — RUL-05, RUL-10, RUL-03 (layout): Second Wind warns at full hit points, the TV's floating result stays in frame, the phone sheet is grouped
- 2d2504f7252f5a2c898b5f8ec37d33f53423d660 — Triage: second play round on the combined build

Next block baseline: `2d2504f7252f5a2c898b5f8ec37d33f53423d660`.

## 2026-10-13T10:55:42-04:00 to 2026-10-13T14:04:49-04:00

Range: cefa2864f7887557351490c88279a4112ca52c1e..493477ca31cc41a14d0ac0a32c59787ee9817cfd

- 929063ff693318d9ff354da81d1feb493e226ee1 — Roadmap and changelog: the fifth playthrough and its fixes
- 5a18675ca860d99bfdd2c8ee25a9b438075b8104 — Provider tests: the closed-port case is given time to be refused on Windows
- 432709201c4b409c3310b82d667b594587902490 — REV-21: seat activity is written at most once every five seconds per seat
- a97b36485b553c8bfe80c9b8186d2a3f4e4b4ce5 — RUN-13: the phone's page before joining shows the lobby's picture
- 9e53905c2bd71f5e6db09e562c765326647d59b3 — OBS-07: the two greatsword packages say what makes them different
- b4c7cbdf9a881c3f99bb1bb29e9815302d69876f — RUN-08, STO-03: the storyteller is told how many creatures take the field and what they carry
- 8b0967d41fe765615e4a63ead235fe92c0645301 — Rules auditor: a thrown javelin lying on the ground does not empty the hero's hand
- b1488ab86f2d4ccab65e78893abf7d746ec1df85 — Format the package ability text and its test
- fc1fc3f9d200169561f95df919f0cbc72ed50de6 — TV fight picture: tilt-shift on the figures, nothing drawn close to the lens, and a scan can end before its ragged rim
- d29a3a4f0a304b3497ff5f4980d715c4cb7e0ea2 — SCN-01: scanned battlefields get their walkable ground from the scan, so nobody stands in or walks through a fence
- 3b0e411488915ad25637d320590713081d73013f — TV fight: a thin side-coloured ring on the ground under every standing figure
- 23c8fa4844812694cfe5637a34fd39297286cc57 — RUN-03, SYN-07: between two enemies' beats the TV heading names the next enemy, not the last hero
- 050729eba7bf46dd87bff5e76c3a71c2bdeee22e — Combat simulator tests: supply the Strength save a wolf's bite now draws
- 493477ca31cc41a14d0ac0a32c59787ee9817cfd — FPX-03: the wide shot comes closer; reachable ground may run under the side cards, figures may not

Next block baseline: `493477ca31cc41a14d0ac0a32c59787ee9817cfd`.

The commit writing this block remains pending for capture in the next block. This block is still open: the work session had not ended when it was written, and it is shorter than six hours.

## 2026-10-13T06:51:46-04:00 to 2026-10-13T10:55:22-04:00

Range: e5034d48311bc22079174965b58199904c878ab4..cefa2864f7887557351490c88279a4112ca52c1e

- 503a541470578a5b53d70ebfa483719a55e74e89 — Changelog: the blocks from 2026-10-12 18:31 to 2026-10-13 06:51 (173 commits)
- b42aa199e256242ea74b2985141cd5a14590317a — SES-04: a hero whose phone has gone away can be taken back from another phone
- 9beca9560cbc1f6d8e9f7272b19635347787be0c — FLO-01: a new story scene's choices are withheld for four seconds
- 0b1fa9d11cffa34704044fcb4090f5a66b483253 — Triage list and roadmap: the seat take-back (the last severity 1 row) and the story hold
- 10a3c565989fbf632db4271a03a582b83ee6a52a — Browser code: four linter findings
- 103fe7a0bdfb886f4db69e5dbeca6d1c9cefa828 — After a won fight both phones went blank: a healthy hero's empty condition line was refused by the rest screen
- c5ba0b4de48aeb690849b0da8c973a4fb8f16a67 — Triage list: second hand-played session (room RUREGO), thirteen findings
- ff1dfb078608e32707f1eb2d9a317ce5d52542d7 — DEV-03: three long-failing preview tests pass again
- a9ea26a1f959c98fbc63e72edf4f783f11b78227 — DEV-03: the reply-cache and speech-capture tests pass again
- c4eeeb81caeb2a05ccfcb6f0104617cddf65ef43 — DEV-03: three stale pins and expectations in the generation and picture tests
- 7ecfbb17d98f5d7016a625b897730bd77bc0e472 — DEV-03: small talk and a private clue are answered again
- cde1bce117091c4615ba09df857a3bb7f8f01909 — FLO-07: the death-save and stable-hero choices are worded as what the heroes do
- 57f0bfdea70e63cca3849cd0cfb52cdc8208d52e — PHN-06: a story scene's log starts at the action that opened the scene
- 21d2f25cecb2c0683c5abbda960cb2a5710730bf — OBS-63: the phone says a refused request was refused, and stops re-sending it
- 8c0f9759f8bccd2c2f5015801291549dfb765318 — RUN-06: story choices in the table log read as actions
- 44792e864e4a366261b04976decadd414471ce3a — RUN-05, OBS-22: plain threats are recognised, a threat is not read as a bribe, and stock replies answer it
- 1db1e80e41800ea384a94c3f0b41fb655eb9b125 — The TV fight: health cards follow the presented beats, heals show, thrown weapons and bites get their own presentation
- eeeb2093fff466245cab88feb2ffe6b5f89893a2 — RUN-11, RUN-12: a look left as the only option is selected by itself; no composer at The End
- 6ee86b2fcc2ed27804c2b7af46065fc73739c5ed — Phone fight map: never a few enormous squares
- baa51c539c95062ed67be1e31137633a4262a023 — RUN-09, RUN-07: the continue choice names its destination once; no second 'Your turn' card under the controls
- fbe5a3ef8f734ea67c1f890928026a6492104e1b — Triage list: the second hand-played session's fixes, and ten of the twelve long-failing tests
- 8645c254332e829230a300ac911ba5bb1f0e1982 — RUN-02, OBS-31: a TV display running behind the server no longer stages a heal back to the old number
- 9c6886d278aa0440048eeda4ec4545e8ffe76d3a — RUN-10: a speech bubble is placed through the picture's real crop, so it does not lie over the speaker's head
- dbdcad0c8d3ec7e93f0f3e7e4443c3af70fd690d — DEV-03: the last two long-failing tests pass, and live replies are held to two rules only the transcripts were
- 7edf7afd05168b5091a1cf19a15e0414abf6dcb2 — A model, not a phrase list, reads how a hero speaks: Jev classifies each spoken line before the reply
- cefa2864f7887557351490c88279a4112ca52c1e — Triage list: the health cards, Jev classification, the speech bubble and the last failing tests

Next block baseline: `cefa2864f7887557351490c88279a4112ca52c1e`.

The commit writing this block remains pending for capture in the next block. This block is still open: the work session had not ended when it was written, and it is shorter than six hours.

## 2026-10-13T06:45:20-04:00 to 2026-10-13T06:51:17-04:00

Range: 07ac688e3ee33f0235d00fb90c6624045db5c9d8..e5034d48311bc22079174965b58199904c878ab4

- f911eb056741c427209a56e12ed180b1e8ae1d09 — PHN-01: tap an enemy on the phone's map to aim; the map stays when no move is left
- c67a8419fc41d3ff308c950f2c602691867b8feb — OBS-62: what a hero says aloud is added to the table log when nobody answers it
- e5034d48311bc22079174965b58199904c878ab4 — Triage list and roadmap: fight speech and the tap-to-aim map

Next block baseline: `e5034d48311bc22079174965b58199904c878ab4`.

The commit writing this block remains pending for capture in the next block. This block is still open: the work session had not ended when it was written, and it is shorter than six hours.

## 2026-10-13T00:43:37-04:00 to 2026-10-13T06:39:44-04:00

Range: dccea92ccebc34f586ea66f50ed9f9f12d1ef3a2..07ac688e3ee33f0235d00fb90c6624045db5c9d8

- 16819357776702321025bc389e29d44ad79d0420 — CRE-07, OBS-06: form text no longer shows under the confirm bar
- 98ba2bf2189669a2fb299cd6fb7ec5f4a723b50a — TVX-04, OBS-12: the scene title greets the scene, then leaves
- 209c7cd6162373b8b6afcc0e63c1dafbedb193f2 — END-03, OBS-54: the health strip is hidden under the end card
- ad9572a33b267a2bd45ce2388f7ddb287d35fed6 — OBS-44: hit point number stays readable at the edge of the bar's fill
- 49704baf8c0fba419818aff75a82194839f99e86 — CRE-06, OBS-01, OBS-09: a local-only game offers no QR code or address
- 283a76d64bb61219043896751dd7f787d4fff86c — OBS-33, SYN-06: the redraw waits behind the finishing blow
- c342181f213a9efb29a559b7fc25630476f5feef — OBS-36, FGT-03: the table log words a graze, a shot and a hero's move correctly
- 1026cc4ad8c4a9ae1afe8f9847e01daeb6cfc6ca — OBS-43, FGT-06: attack options are ordered best plain choice first
- c90814b647439c207a4ef8796101cd41e1ca0e7b — OBS-35, OBS-43, OBS-45: phone fight panel wording
- b787a80851dac1433ff236d0812637ceed7db191 — OBS-11: the narrator's box clears before the speaker's line appears
- 399e137991aad9dc0ca7c6a258ed27defd322c2b — Triage list: mark the first hand-play fixes and record the causes found
- 631ca1ebd4383eee4d5a1566ac95daa5552fd552 — OBS-55: the ending no longer asks for music that was never recorded
- deb37c2e98a84be7b6d626bf13029705c715368b — OBS-32, SYN-04: a heal during a fight survives the TV's catch-up
- c05f53c7d07fe240062ce0104839cfacfafbc4b4 — OBS-34, SYN-03: the phone's roll cards wait for the TV
- c4d94432734f1e939a165620095855e8bfa1d679 — OBS-36, OBS-51: the phone's table log keeps actions from its first view
- 79388033fede371c558558045ca89d29d2442686 — OBS-38: a creature with no language gets no spoken fight lines
- 847473bec02e48d58e4e99cd05d182d4a87eb29e — OBS-25, STO-09: a character's reply is told what is happening in the scene
- 0ac1d6710370749f7b203b3f58630b4e00a877bd — OBS-47: the victory card drops 'Awake' and shows the patron's line
- dd7954088ce5d3174ed90ce46687a978f4394098 — OBS-19, OBS-21, OBS-05: the phone composer says what it does
- eca342df2fb1eb5625612c39b89d6d518a268462 — OBS-13: a speech bubble stays paired with the picture on screen
- d62a3020d269186ae05ba80ac04be9a914e2b81b — DEV: the browser-side test build compiles again
- 70a9d60834130f6f3940667aee1862fd6257292f — CRE-03, OBS-03, OBS-05, OBS-15: player wording on the creation form and hero sheet
- 30ecf8b201d5602003b04fb2bb0114ea94e0ebfd — Triage list: mark the second batch of hand-play fixes
- 37f678d5a365a1f4e313f6c249ddf96937600cbc — SYN-01, SYN-03: opening enemy turns wait while the battlefield is still loading
- ef7747de75f1248dd078b353b321d887f3218dd4 — OBS-51: an action's log line keeps its place ahead of what followed it
- 06b4e213277225161d51a968f599f2480d74abc2 — OBS-14: the phone story card shows the newest reply, once
- ea9430a9ccedacb9a758211ddbb526f5dff30725 — OBS-52, STO-06: an ending does not repeat an earlier scene's sentence
- 9f26d62ae85b8fe695b7b28ab1349843a0774f47 — OBS-08: the tie rule names 'the enemy', not one adventure's bandit
- 431f24650f0754a6054ebe736fe6b62f689ebf8f — Triage list: mark the third batch of hand-play fixes
- 876d2fdef8e3942bce2b2aebe88a24e89a54b798 — HNT-26 (part): a mover paused on an ally's square no longer refuses the fight
- 0f97c59d7ac601965311421d38a2c52a726f238d — OBS-57: the server log records turns, heals, endings and story steps
- 7218c949ae93da94da5bb104b5d710df8c3127ec — OBS-10, FLO-08: the opening narration is not replayed when the picture refreshes
- f0a14bf48d65032d533e3554a6263790e2cd4746 — REV-03: a heal that follows a fresh blow on the same hero is staged after it
- 1a66b6be4fc35a3105bd2f37b0c33437776f8404 — REV-05: the picture setting is checked before the session is admitted
- 4852c21d395e3f7eaa4f7cca823142f0efa69344 — REV-07: scanned close-ups are fitted at the push-in the shot will use
- a6efb1374852a5c8ace36db6d7151af8226112d3 — REV-09: cleaned model text may not start with a list hyphen
- 14fc1fee5e5157d1cb4e44475390cd938aa47fc6 — REV-02, SES-05: story text that arrived before the deadline is kept
- ab0fc916e5885358664059371190bc98664035c0 — REV-04: a typed line in the lower strip closes before the reply's bubble opens
- d44deca02257c4214b6ae89637de84c03b69c016 — REV-06: the dice clip is a still under reduced motion
- 1cb72de375f801a056702ef0bfc4033be1132af4 — REV-08, OBS-44: the TV shows the rules note; the phone's fight controls are laid out
- 2d76c3e641730031df639b581c7ca85ffe1fdddc — REV-01: a rejected fight action does not leave the phone waiting
- 3042d02f408709fdf1604ac4dd64549d4c8a0dce — Triage list: mark the fourth batch of fixes
- a0ae78d60adf800a189582f13988f239aa5afa63 — OBS-46, PHN-03: the rules note for a fight reads as plain sentences
- 585d88ea5ac383b4f1711451d364323308d8c8c0 — REV-10: the play-and-record check presses only story choices
- 87950360d810d80d637de76f912908868ea897c3 — OBS-27: the fight's introduction is told before the first blow
- b2f7239f952d665ea3187070192b9419573b8b5b — SES-03: a refused action tells the player, briefly; creation page heading
- 807201f440ffdaf4f90d164641b3ef13d12d4429 — Triage list: mark the fifth batch and the items that need a design decision
- a98723ca30aea4779a74d2fa961b06c75f92ff0b — Roadmap: mark the hand-played session's items
- 270548ad6cfc1b5f9b9a6f57c9677bb7ecf31fb0 — Fix: the phone's waiting page draws for a local-only game
- ce75ec385ac63560fe454dc8f308be4d9a037427 — Fix: the phone's story screen draws again
- 134d711b4389c65b4a0bba169282e567afe81cdb — OBS-20, PHN-08: the phone's fight controls are updated in place
- a70bec667b5eee4a0378883159e38384bd63faa1 — OBS-34: at a fight's start the phone shows no roll the TV has not shown
- 918cdee31ba89a3401c0e8dcc16ea2f22cda5721 — Triage list: results read back from the running pages, two regressions noted, three new rows
- 1b5a67d302fd42900b4e26140c971a9b12dae2d3 — END-01 tests: the defeat text opens with the fight
- 7e524f80d9dab444bd5f2012bd7eef5fdd3079c3 — END-01: the defeat screen says what the party fought and who fell
- 761b5356bab4cb0cb20fe9e620e382e9001320a2 — OBS-58: a crashed game can be restarted on its database
- da21f17b9dd22e9bcde08ce22f5bbf4bfc626a88 — OBS-15, OBS-59: hero cards and the creation form use player wording
- aa982f700f5482caebaca5d8bac9480a30652a61 — OBS-60: the server log is in order at a fight's start and records the beginning
- 2cde9ae0ed938eb27b9bb4f847037acac71a5987 — OBS-02, CRE-04: the phone's join and waiting pages use the adventure's own place picture
- 4fc941ef970d40a74da156918265ed51126adcaa — OBS-40, FPX-07: the finishing-blow banner is kept for the blow that ends the fight
- af39b8bb8fd7451826d6ca6526c63220ce8438ed — OBS-39, FPX-09: a dead enemy keeps one mark, and it is not drawn over other figures or the health cards
- 00c6078571f6b7877cedff1bb0ce6bc29de40f89 — HNT-26: fights with three or more enemies were refused at the start
- a7673dd5f8f4dca7a47094350e129a68bddba143 — Fix: actions are accepted again (storage query keeps its lease column)
- 009bb09f4ff2dbf2d609d6f3e845faf7c71db736 — STO-10: a reply that draws on a hero's private clue stays off the TV
- 268e0180f0ff7bba48ccc6dbeeabb6b512961141 — Triage list: sixth batch, the fight-start bug closed, results read back from running pages
- 182fad8a3a0d52f8119baaa203d54a088ba2da8f — flow-a3 1: offered action kinds come from the session machine
- 9f58bedfaa428127c88f40b0f14c0ba0acc71f4e — OBS-45, OBS-56, OBS-02: small wording on the phone
- 6a013ebd93ce0d358cc7526bc6af8ff37cc37d57 — TVX-05: a heal's caption is brief and says what was regained
- a27da6471d32257b1986031da71650c0a30fbcea — OBS-41: floating fight text stays inside the screen
- 5067cda1d3db2f19679dcf7bb5d34b3d22107c54 — OBS-42: one die per d20, shown beside the attack card
- 5272a1755b12771967b2afe5dde531201b429a95 — Start-up retries the adventure database connection
- 8a9fe3cb9c58dcd52745bff2193514ca461cc2c7 — OBS-18, FLO-06: offer labels stay actions; the model no longer rewrites them into titles
- f81314b66f55cb5febe4df2ddcc24bb64a7abb45 — Triage list: seventh batch
- 8abe71b6205909a7070a559c49629a5a477286ae — Triage list: correct one row, add the start-up connection row
- 3d6eaa6b4476f36d5746fb2e5a3b06f10e0ce1fb — Triage list: seven faults from the second review of the fixes
- f3e9f99e6b31f394650a4f809d81bdc28bce27b3 — flow-a3 2 / OBS-48: the short rest is an Aftermath event, once per aftermath, only while a hero regains
- 05e96ab26ce10861a83b672b455f3e0d86781599 — REV-11: a speech bubble follows its speaker when the same picture gets a new layout
- 8309c3b1d5684adfd90f7119c64806d261965ad8 — REV-12: kept fight controls are bound again on every redraw and follow the refreshed offer
- 7726562898a13669fc8ecd06c77e48dcbbc715c8 — Phone fight panel: the button rows apply again
- 8b538a8256b0d38809e1ec751870bf64e5e46635 — REV-14: a character's line stays in the phone log during a fight, and the story card ignores the previous scene's reply
- fc5f7d6cc7c58a9b9a9c25390ffecb8b72adc864 — REV-16, REV-17: the rules note is for the phones only, and every disclosed gap reads as a sentence
- e62c349d8f825791f75a46c3811794725e50cda9 — flow-a3 3: the fight and the conversation are entered only through the session machine, and every staged outcome is checked against it
- aabe35a9fb9263acfcd4cdabe0ea25bc8202d26a — Triage list: second-review fixes, the short rest, and the state machine's third step
- 8ef03800d212162bb0579a132736c6a67c4842ca — PHN-01: the phone moves by tapping a map of the fight
- 523a00b8d27dfefb193e71a31075a5e86820f4b2 — The session machine's transitions and refusals are written to the server log
- 953f1462e52b41c419c48f5381e14854292f7bf0 — REV-19: the developer command line keeps its seats when the server restarts on the same game
- a30f6295d8214f9755a03b60e79ac1315d8c2525 — FGT-02: damage dice are kept, sent and logged
- 397e6e3d878eb71467a2ee2af91c81c44be5efef — OBS-22: a threat cools a character toward the hero before the reply is written
- 99f570030a5008e591a1997fa764fc311e0632a5 — OBS-43: the attack list has one entry per weapon and target; mastery and Savage Attacker are their own choices
- af3b71bb8b142c30f0b17d2614365ff5a76b2139 — REV-13: the page a phone sees before joining shows the adventure's own place
- 1e2bd33edd6b14d372ed0319027dec92c4ae58fa — OBS-53: the end card no longer promises a control that does not exist
- 07ac688e3ee33f0235d00fb90c6624045db5c9d8 — Triage list and roadmap: eighth batch (state-machine log, damage dice, attack list, move map, seats after restart)

Next block baseline: `07ac688e3ee33f0235d00fb90c6624045db5c9d8`.

## 2026-10-12T18:31:36-04:00 to 2026-10-13T00:25:32-04:00

Range: 558654d38a308ef898da618ce1dbe9731a8bcafc..dccea92ccebc34f586ea66f50ed9f9f12d1ef3a2

- c400f956cb804ee52a047ab08749cf9f08e821a7 — Record the 2026-10-12 afternoon block: the play-session work in progress
- ed8bda033eef4c3afc51de0ab6dff34a492e8a21 — WIP baseline (compiles, not verified) before the bug-fix campaign
- f0358ee3c57f02c0ffcd499d9a0a9a6d575ab9f1 — Support: make the lib test build compile again (directed-plate test)
- 8a59a71fad1c74a555c6ac7e4e0989660c496f94 — Support: implement report_presented in the view-revision peer fixture
- 301391eac3f86bfcf240930d5beb2866781f06c9 — CRE-04, END-02: neutral join page; phone shows a clear end state
- 8fa68c82743854ffa942289b7a91fffe51fef79c — PHN-05: show the attack-recovery control only when an attack is unconfirmed
- 3720b367676f576248f909941830330a03f0cf99 — END-03, TVX-03, TVX-04: scene titles and a clean end card on the TV
- edece581731289d89974217fdaca9b18fae65cae — CRE-02, CRE-07: memorable-detail field usable; form clears the confirm bar
- a252a2244a543b302415b6439b7dcda6b96d12f8 — PHN-04, PHN-08: composer labels say what they do; talk buttons stay mounted
- 4c7e69bb2cf9e38701b47fa6d80ae73eb07a9d31 — TVX-01 (part): the typed line leaves before the reply's bubble enters
- dc17a41ef4fbcdb7e4bacb829ffdc8351020c95f — AUD-04: a large centred prompt while the TV's sound is off
- 7ecd7f957372a963f7c9b0dd6a62220fdd6653f2 — PIC-03: picture refreshes let poses move on
- 97f9d1eedf2210b70feab823252fd498cf4d8fc6 — SES-11: battle scans default to level of detail 1
- e8a800f5c567d23eab90c562537ecf0ed70b5acb — CRE-01 (part): a group with one option is pre-selected and submitted as a fact
- 0a9436f55890800233a76225d5c951fedd201f13 — DEV-05: whole-game play-and-record browser check
- e73f496a8f94860a8bd02519473342f9cbf2e7ba — Mark fixed and part-fixed items; add the code-review findings to the triage list
- 65ff23cf1aa0bc0f126aafcc675d7a40e33f1638 — Support: refusal detail on REFUSAL and UNAVAILABLE lines
- d495f61be1febfb6b15242f90cef1dc15e9f0fda — Support: name the refusing step on the fight-start and enemy-turn paths
- 5cc1d7618e05311f821a480f3fa703b3b213b5c5 — SYN-timer (WIP, not seen in a browser): the fight's pacing no longer waits for ever on a silent display
- ffed4a489c9b139f09fba9ae839f4b53ca1a7b42 — SES-05 (WIP, not verified against the live model): start-up story text in small groups with retries
- 5abd72e4d98beccf4a582acfb1c81c4e52351df4 — HNT-01, SES-02: a fight starts whatever order the heroes were confirmed in
- 4abbe0f23e936eda71143cb47a86e3c51afda9f4 — HNT-06: a healed figure can act again on the TV
- 67b9586629a05e28100912eb3670557679147d9f — FLO-06: ending titles are quoted in the log, not conjugated
- dce149caf01f4e4121aa8e6645f68cce4cafcd87 — PIC-08: the opening picture is requested once
- 0ad7d6712d1d2e4f7b397e477c092288570cd7e8 — PHN-07: phone hero portrait shows the face; fight status line is readable
- 6b5c6914eb1f112dab78f457670a3334678b33d0 — CRE-05, CRE-06: the TV lobby says what is happening and never prints a loopback address
- e995a874f7d0851ad12970b1e87f50ab3c9b381a — AUD-01: music comes back when a line is only lingering on screen
- cafd68cee75d6ab6aeffc2d4e76c332c4eff256a — HNT-03: a stale attack command is refused on the simple combat path
- a1b3e2510bd42cac0c5ec625cfd4a30c2d564e40 — TVX-02: a hero's typed line is anchored to that hero
- 1d664bd794651d4ad37d7917fe16e7b23be54ec3 — CRE-01: a single-option creation group reads as a fact
- 0761a5b1185cd590a98445f0e1bfddb3c29833a6 — SYN-05, PHN-02, PHN-05: the phone shows a spent turn as spent, and death saves
- 307c2b4c02c4fe2e80a768a219aa80440bd9f5f4 — AUD-04 (follow-up): the sound prompt stays in the lobby and covers nothing
- 126bec7d26f11ce61207a239826794dc5443f99d — Mark the round-2 fixes in the triage list and the roadmap
- 30a2551bce429138d4e74d8a9015e0b8da32ae34 — Triage list: add the second pass of code-review findings
- 8d5425b2dff3ef6df5ec25b2e09985df2fa159b0 — SYN-01, SYN-02: the TV queues every fight beat once, by its server sequence
- 0d8934fc944b4126e1fd5b63ceb3048d0d9d7c7d — FPX-01: a presented roll plays its tumbling clip
- 0bf723707af46bf63d4db1748e79545c092f290a — SYN-04: a heal reaches the TV's health card with its beat, not before
- 129547c8fe8a9f4a47fecf553ec2842954fcad41 — HNT-17: a catch-up no longer erases a pending scene change on the TV
- a470d5e1e1975e3d0b0f9759a3718206d4cd076c — HNT-07: missing story-text slots are asked for again
- 33b0ba925a0bb911dfe3d3e50fea57cb242eaf02 — HNT-10: a lost session no longer spins on an expired heartbeat
- 1222ca161e6c53dfc0452e9868214751a6710120 — HNT-15: cleaned-up model text must keep its words and carry no Markdown
- 0f5b6959130e9eda56b4001b36da7ff97e3ea8c2 — HNT-11: served scan and lobby-video files must be inside their real folder
- 5e97f9790352a4a295bf55ddfcdbbb5fa01cdc96 — HNT-21: scene lighting matches whole words; a valid library scene keeps its picture
- 224b74f7b43ae09452e0080ee2ce93976dd06528 — HNT-18: a defeat scene with dialogue is not labelled a victory
- 4570a60e3feaccee81d7ec2ad932b979fb6744af — SYN-06: the finale music waits for the last fight beat to be shown
- 7526b4d8686ec60bd09922d88f6606480b2441da — DEV-01: dev health needs no seat; read commands work on browser-started sessions
- dc9c81f500bc3eb3343184ab4bd6a3fd4325cc42 — HNT-22: an oversized cached picture is replaced, not served as missing
- 6dcfb0e17218f993c13d716021092e20a0c88dc0 — HNT-23: a speech-intent loader is cleared when its line closes off the fight screen
- d4246c48a1212d022bdfdb98bf308ec5210a966f — STO-08: the phone shows a private clue once, not on every later screen
- d8e4a5f07afc30d387ffb1c1e9424c8cdb500c32 — HNT-13: push-to-talk audio is dropped when the view it was recorded under has gone
- 7d0f493d9e6d6d41339166f03cf076f0fcc79756 — SES-08: a start-up that fails after admission releases its session
- c64fe4aed3618bef41d7bcd4b30c1ac9a7db3462 — Tests: compare fight outcomes without their view sequence numbers
- cd92d25e48fcaf2e27940cfdd0d7a4c249c19c9b — HNT-05: a wolf's or mastiff's bite knocks prone only on a failed Strength save
- 327e45c8b41b06827284035aecbdaa8043c6c3dd — Mark the round-3 fixes in the triage list and the roadmap
- 7a3d50541ea0741320a76c4a937c2d06994ab296 — FPX-02: close-up camera poses are checked against the scene's occupancy
- 7e6c6e19127d9d8001ce4e3237f7481d245bbd5f — HNT-16: duplicate speaker names in a content pack are refused
- c813ce70dc88d7437421c43a6e676a61d41014c1 — HNT-14: cancelling a speech upload aborts the request itself
- e47c1a32a32454563bd1869fc03302203ba5e98c — HNT-12: a stale End turn (or any stale combat action) is refused on the simple combat path
- 0e8e4a1e05758f540eb71d9be563bee93e68e0c0 — HNT-02, HNT-09: restored moves obey the corner rule; thrown weapons land and can be picked up
- 78771c3d9f104acd5ec5637230671f7af3010c4d — Mark the round-4 fixes in the triage list
- 1817d9e791df9119c766779b5a2e5cc2aed0261c — SES-01: a refused command no longer kills the room
- 9f4f1ffaebf5fcacf1a1de9fb1227361461de6c6 — SYN-09 (part): downed combatants in the turn strip are named and marked
- 80f4bb81b0c21e8d22e113fcae26ab9db4d56168 — HNT-19: scan level-of-detail ranges are checked against the chunk's record count
- b0f0eb46b4764bd3e7198dcc0cce13997554822b — HNT-24: a dice folder setting that is not valid Unicode is refused at start-up
- ed7d78bb0234290ed469c9e2027bb8aa61095116 — HNT-20 (part): the browser's WebSocket connection set-up times out after ten seconds
- 5b9700c7abc146baab73a33d9389b02c71cf822b — PHN-03, PHN-07: phone fight panel shows resources as chips and folds the rules note away
- 233ac51aed29fbd7fad059e0e0f2d69395f0ff03 — END-03 (part): the end card waits for the last line to close
- 56b69c72d9bf1a24d3609b543e2f9ab33cc9f597 — SES-07, HNT-20: choices are disabled while a request is in flight; join retries are bounded
- c9ab22ab6ff47d0128dfaeacac1d508503682f84 — Mark the round-5 fixes and the dead-room fix in the triage list
- 3b05752a83dd248db55946751146c224655111c2 — AUD-02: a narration longer than the voice limit is spoken in parts
- a44e30b543cc6eabd053bc7f8596e741ef8f4a97 — CRE-06 (phone): the waiting page does not offer a loopback link to share
- b3f740ee73ff2d83a85df29fe4bbb06d172d6b33 — PIC-03 (follow-up): drop the test assertion the new refresh wording made obsolete
- 897c4c6ced7e315ad8f2dedc61edb6975ef57936 — Triage list: add the independent review of tonight's fixes
- b4d7f43283d6bcad9910b4c36b1a294e61d36ed3 — SYN-03: a beat held for the battlefield entrance is not presented
- da6dc50a9e5804fc3b14be171c91ed902dbf3b0d — Support: the forward session state machine, as a gate in front of staging
- ebc63886947ea69c0c865e5a91c1a1f293d26b3b — Triage list: closing state of the 2026-10-12 bug-fix session
- 6d4ab2f703693d60c1b6211a3f49f87eee21253d — Triage list: record the full test-suite result at the end of the session
- dccea92ccebc34f586ea66f50ed9f9f12d1ef3a2 — Triage list: findings from the hand-played session (room KESEZA)

Next block baseline: `dccea92ccebc34f586ea66f50ed9f9f12d1ef3a2`.

## 2026-10-12T12:08:54-04:00 to 2026-10-12T18:31:22-04:00

Range: 2994d1bff06b11de6863b77cd1e5851e91cffbcd..558654d38a308ef898da618ce1dbe9731a8bcafc

- de32f94cce016b7d3e5a7a1512cb5632056aa573 — Record the closing block: leftover tooling merges and the bug notes
- 558654d38a308ef898da618ce1dbe9731a8bcafc — WIP (compiles, not verified): Windows port, dialogue, fight sync, dice video, music, victory screen, dev control CLI

## 2026-10-12T11:28:38-04:00 to 2026-10-12T12:08:54-04:00

Range: 22ba05f61230a46765695aee52b6b800d49718e7..2994d1bff06b11de6863b77cd1e5851e91cffbcd

- aecb8f5101374fe3a0221e9a8de4b16b5bf09bd8 — Record the closing integration block and the final merges in the hand-off notes
- 4e1c771cbed5c34da13b2a8dc22452bf23f86c37 — WIP (not green): TV layout check (PLAY-V14 M1), overlap, covered, clipped, ellipsis and TV-safe checks over scripted TV screens; scenarios lobby, story, fight partly verified
- 5afc44a34465c3a0d5e721f8611b5c7e4f427026 — Merge branch 'claude/playable-20261011' into worktree-agent-a9531e4e2a6599b86
- fd7c7714ba298d27f272c910f6cdbba92a8a2dfc — Merge the TV layout check tooling (PLAY-V14 M1, work in progress): overlap, covered, clipped and ellipsis checks for the display
- 0afbaea272a7a0f2b4e4e263f297cd0652bb5d06 — WIP (not green): native real-scan figure visibility test (CPU copy of the splat blend over every cell, level and shot); written and compiling, never run to completion, thresholds unvalidated
- 24bc0fe3858dbdeed056bb11afd3dc2aab1ba6f2 — Merge branch 'claude/playable-20261011' into worktree-agent-a8ccec3abe79264ce
- e1c4bab33c5cd944c396574af40fc5131418b42d — Merge the real-scan figure visibility test (PLAY-S03, work in progress)
- 2994d1bff06b11de6863b77cd1e5851e91cffbcd — Bug notes for main from the last live playthrough; ignore the unvalidated scan visibility test

Next block baseline: `2994d1bff06b11de6863b77cd1e5851e91cffbcd`.

This block closes the day's work on `main`; it is shorter than six hours.

## 2026-10-12T10:33:51-04:00 to 2026-10-12T11:28:38-04:00

Range: b161111268f5c9a600711ce5ca0d6522cb5a220e..22ba05f61230a46765695aee52b6b800d49718e7

- 4a24a788a1015d93150c646f496653d572e65d73 — Record the integration block for the owner-review merges (fight endings, table settings, toggle to talk, home screen, establishing shots, fight staging, battle-scene slice)
- d7c6ffd8b47745aca61086e1c998e41426a0321f — Hand-off notes for the playable-game session and merged tasks moved to review in the queue
- 77caa61f4ed81eb7e8fd994ca3a37fa6e6f98aa3 — Measure the home loop's joins every frame, and give the flight wait a longer bound (PLAY-V10 pair prep)
- 5066fa7381d55b614e8a7dc58cbedf84940068ce — Merge the home-loop join measurement and a longer wait bound for the video tests (PLAY-V10 follow-up)
- b42a844064bec891fe3ea383ab6787fbfbb2b0a5 — Draw a fight's closing scenes as story scenes, and offer the way out of a settled defeat
- 236b05778b9da62d3db01795553f1899ae1804d1 — Merge the fight-ending fix: a fight's closing scenes draw as story scenes and reach the end card; a settled defeat offers its way out (PLAY-E04)
- 282942dd4e370f0ec4b372f4d5cce82522d9e7a0 — Level of detail: the chosen level is drawn complete out to as far as the splat budget allows, then steps coarser with distance (no thinner ring just past the play area); level 1 is the second best on disk
- ebc5bdae2459faac409fd0e14a6bcf06aaf32d38 — Resting camera keeps a clear line from the lens to every figure and the field: scan occupancy in the scene file (authored by the splat_occupancy example), the solve raises or turns the camera rather than looking through trunks and foliage
- 6dbbbb047559e6d97fa749161900740ebf9a60b0 — The TV draws no cell grid by default: no borders, reachable tint, blocked marks or side rings; a thin ring under the acting figure, the target ring and contact shadows stay; DF_GAMEPLAY_BATTLE_GRID=on|off and preview &grid=1 turn the hairline grid back on
- 881297e8b82c0dc2bacd8a6406f0f016787d3379 — Merge the battle-scene follow-ups: no grid on the TV by default, the chosen splat level drawn complete, a camera line clear of scan geometry (PLAY-S03)
- 6767bc9a010762d15b070dd4ba76b6167b23b8eb — Show a plate as soon as its picture lands with the predicted layout and measure the heads afterwards; raise the lower third clear of the choice chips (PLAY-V12)
- 44c29e9e113a632506e67d2036ea6fb3aff65165 — Merge branch 'claude/playable-20261011' into worktree-agent-a0d8e630308b47719
- c16d6c60b6b67f88109015746c6189b64c1080e7 — Merge the scene-plate follow-up: a plate shows as soon as its picture lands and the lower third sits higher (PLAY-V12)
- 445ff0a87f029de5a2e3a5448ba37536804f54cf — Cue stream waits for the battlefield; unheld figures stand where the newest view puts them; card waits for the walk; browser checks (PLAY-B03)
- 4d8f2edaf812c1c9206b580cbd3e0871ac70527a — Merge branch 'claude/playable-20261011' into worktree-agent-a68fb704e2220053d
- cfa28f7655ebee3ac3afab0df8da2f2d54c923d3 — Merge fight staging slice two: the director waits for the battlefield, no snap-back, the card waits for the walk (PLAY-B03)
- 6456c3bbe890b307beac47ee7382d187ce2d33f7 — Show the dying hero's saves and make the post-fight advance work; add the independent rules auditor, audit fields, Kobold sunlight and thrown dagger, creature feature table and coverage matrix (PLAY-R01)
- 22ba05f61230a46765695aee52b6b800d49718e7 — Merge strict fight rules: audited movement, cover, roll modes and timed death saves on the table log (PLAY-R02)

Next block baseline: `22ba05f61230a46765695aee52b6b800d49718e7`.

This block closes the work session; it is shorter than six hours.

## 2026-10-12T08:11:26-04:00 to 2026-10-12T10:33:51-04:00

Range: c5af1e5418d5e2ae8feb105da955d7ed381f1bd3..b161111268f5c9a600711ce5ca0d6522cb5a220e

- a6b508487b84182b5b237ace40d71ca83ae22d8e — End every fight chapter with a resolution, an epilogue and an end card
- ec4eed30d2ec6b6735fd4b4d8f021518dac8310f — Record the eight-branch integration block and queue the tasks from the owner's live review
- e8eb3c9697a28380a8edf396f56c7816b223c5ed — Queue the twelve tasks dispatched from the owner's live review
- b17a889e62ed1eefe846ef7fa9b111f4f92c58b5 — Merge branch 'claude/playable-20261011' into worktree-agent-abe2121761b0d618f
- e31279ff334e5d92d68d2b3a2511ce56e2e2212f — Merge fight endings: every fight chapter reaches a resolution, an epilogue and an end card per outcome (PLAY-E04, first milestone)
- 5358e24585fc67a045722a166922e9d00e681030 — PLAY-U02 M1: table setting for the choice chips, changed from a phone
- 81ed9dce06f20fb6e90263089e0aa02c9d5435a9 — Merge the table setting that hides or shows the choice chips on the TV, changed from any phone (PLAY-U02, first milestone)
- 10d8afc432c41ea9a4bcb27e25f68fd609fc58e4 — Toggle to talk: one tap starts, the next tap stops
- f3ee2f6d663c64b290f46feef7ad9f44f3872533 — Merge toggle to talk: tap once to start listening and again to stop, with listening ended by typing, a hidden page or a lost connection (PLAY-U03)
- 6f2283623ad15ddb97f831c48fd35a98a142612b — Compose the TV home screen as one fixed layout and drop the per-picture box placement (PLAY-V11)
- a3032774b828dcd4d0569e42221530750024bcde — Merge claude/playable-20261011 into PLAY-V11
- 28b814a42149a21c9a1211738373193cbd8c1a1d — Merge the composed TV home screen: one fixed, aligned title-screen layout for every place (PLAY-V11)
- 56b5fd1278e1588bf0a142bc2c98578d714ce2c7 — Open every story scene on an establishing shot, never draw the cut-out row while a plate is on its way, and ask for the opening plate in the lobby (PLAY-V12)
- 2f89e2d366483b5e053e9cd812f7adf753a8a65f — Merge branch 'claude/playable-20261011' into worktree-agent-a0d8e630308b47719
- df6be861b8c527f20a9f8b48acd8ec46a6214155 — Merge scenes that open on an establishing shot: no cut-out row while a plate is pending, the opening plate requested in the lobby, plates made ahead across a fight, a journey and the ending (PLAY-V12, first slice)
- e1a2d26d6d99dac6580bfa748565703cc1c998ab — Stage enemy turns in the server's order; speech, one sting, damage type, test fights, appearance list, current creation offer (PLAY-B03)
- eabb0ed8121f286291dd44b70cfd61db42d08a31 — Merge branch 'claude/playable-20261011' into worktree-agent-a68fb704e2220053d
- 0b8dafa5cf4157abc16a5da8d2d938f98e534524 — Merge fight staging in server order: walk before the hit, labels that follow the beats, the Opportunity Attack prompt, named damage with its type, speech over the speaking figure, one sting, test fights against one to four enemies, the stale creation offer fixed (PLAY-B03, first slice)
- 7dfafa9d5c830dba5e181c73db7f6c2b4d6654ec — Grid: hairline cell borders without glow, the reachable region as a faint tint with one thin outline, targets and the acting figure as thin rings; preview &reach and &beat=rest
- 1f321e8ec4afdf74cbaf777e07aab9b6ff687993 — Figures are never hidden by scan splats beside or behind them (faint ghost where geometry truly stands in front); the camera rests on a solved wide three-quarter shot with a short lean at the start of a turn; the tilt-shift blur only during focus pulls
- 2c57fbceb99ed173213b3b2a8254867ea0ef831c — Merge branch 'claude/playable-20261011' into worktree-agent-a8ccec3abe79264ce
- b161111268f5c9a600711ce5ca0d6522cb5a220e — Merge the battle-scene slice: hairline grid with the reachable region as one faint outlined tint, figures never hidden by scan splats beside or behind them, a solved wide resting camera (PLAY-S03, first slice)

Next block baseline: `b161111268f5c9a600711ce5ca0d6522cb5a220e`.

## 2026-10-12T04:33:02-04:00 to 2026-10-12T08:11:26-04:00

Range: 5310dcf601c50b60ecf17c3d297d524a3bf65029..c5af1e5418d5e2ae8feb105da955d7ed381f1bd3

- 6bd89b11fc7c902eeb2b7432355669e57910c151 — Record the changelog blocks for the playable integration branch through 5310dcf
- 8af22ffcff82f734d06a35c514e7dcea0c2b7ba8 — Queue the join QR, NPC conversation, investigation and ending, scene plates, tactical live fixes, scan layouts and battlefield health cues
- 572c48b831d92aa617c729410cf7207c84de4221 — Add just-in-time scene plates: shot director, plate prompt, party sheet and plate requests (PLAY-V09 M1)
- 2b24277418f17f284522102dac9c6486e01409f9 — Show the scene plate on the display with bubbles beside the speaker's head (PLAY-V09 M2)
- 43482ce4207e4245f1098dca3334a63c6bffc3ec — Tune the plate recipe, make plate edits reach the display and request the current plate first (PLAY-V09 M3)
- 86ff52e855f0be53748fb2e09a1fcbab4ffa2939 — Merge the integration branch into the PLAY-V09 plates
- 522d67ba51f721c93535c574b216346c8c1bbc7d — Merge composed scene plates: one photographic plate per story scene with the bubble beside the speaker's head (PLAY-V09 milestones 1 and 2)
- 12f4debacd827a4276ea1c625c6460c385ecb376 — Measure where the speaker's head really is in each plate, and tune head and hero heights (PLAY-V09 M3)
- 11ad86a49557ed8019f6b9467b2b7cd39f988cfb — Merge scene plates milestone 3: a tuned recipe, the speaker's head measured in each finished plate, the current plate requested first (PLAY-V09)
- 130ff9e53f43c90d1766d9f77559aa12d543d21b — End peaceful chapters with a resolution, an epilogue and an end card
- 82ba3e9868ef45eecca9a4cc2897c0af14607d95 — Show the end card and accept the resolution the phone lists
- df65a3ac18c3b38acbe99d43d7e27e207746ec0f — Conjugate table labels correctly and put the contact on stage in script scenes
- 3d4bd93ba182bcb09745e4b10b7b1f7af90f2b76 — Merge the peaceful adventure ending: resolution, epilogue and an end card for mystery and parley chapters (PLAY-E03, in progress)
- 58c221a7eb658526389ea3cda1d74149fe2b56fc — Turn second-person labels into the third person in the table log
- ee702920997eaae9aabbbd306956dc34fc1ef69e — Format the third-person table log change
- 77200fdcf62420a660295b1339b9b6497bcffb36 — Draw the adventure's end card on the TV above the scene
- 3c6abeabafeea2ad0acb180e3400a56a32c04b50 — Merge the TV end card and third-person table labels (PLAY-E03, fight endings still open)
- a807a06998f5c91e0d1cd8258bc4a53878f6ae70 — PLAY-H01 attempt 2: health cues on the 3D battlefield (plates, variants with cross-fade, hit reaction, tremor, laid-low look), photographic house style
- f53a77f5e98ced0dfd4d7e59dd1cba20a6c56794 — Merge integration 11ad86a: reaction composes with the stage's recoil and hit-stop, plates read the stage variant
- 97a19e7f2824103f009be237a2abddd0a701e38a — Merge integration 3d4bd93 into PLAY-H01 battlefield
- 64d80b6dbf650614f5b5962043f43b14a5b21752 — Battlefield plate face crop wider for slumped variants
- 6fdc4bdcbcc74216fd589ed77f990995124ee254 — Merge health cues on the 3D battlefield: plates, figures and hit reactions follow each hero's state (PLAY-H02)
- 90a2d1e835e92486a10f0f1b3f2e79042b3a6e58 — Show plates without an empty frame, find the heroes' heads, keep the layout beside the plate, draw plates on phones (PLAY-V09 M4)
- 28cb2e4399edd0183006358fa5bb1b4b9443d855 — Merge scene plates milestone 4: no empty stage at handover, hero lines beside the hero, plates on phones, steadier faces (PLAY-V09)
- 6924da0d783cbf779b6e00b9ca7ef86de3f79a0d — Fix archer creation and the acting phone's dropped fight stream (PLAY-T03)
- 1b58ebda67480080a4976a5dfe28168bd3da387b — Merge the archer creation and fight-stream fixes: larger receipts, 64 KiB session messages, refusals logged (PLAY-T03)
- 0535f9ebeb2e36775fdaa841424c822fef98a4e4 — PLAY-N04 M3 and M4 (server): speech-intent bubble and cleaned-up hero line
- d68f4da8c75a7e8fae273147463f762a81d3160b — Merge branch 'claude/playable-20261011' into worktree-agent-a879d6be2836900f0
- 88f8e856a5c412939060f0c246e9f06a6345f558 — PLAY-F06: fight entrance and exit held to the battlefield, voice clips waited for, sound click by event identity
- 821ce584d97e1f0dc761458bf39a9869d8e0a90c — Merge the integration branch (health cues, join_url) into PLAY-F06/N04
- 9a060645420c2e8366fd330f6648630171ccdee8 — PLAY-F06: victory card goes straight from the fight, text kept clear of figures; name/question cleanup fixes; checks
- 47411d06317a7792099680a4e15227bf8fd81d76 — Merge the integration branch (tactical combat) into PLAY-F06/N04: the battle machine stages several outcomes in order, blows are never dropped when the display is behind, restart into a fight enters it, the session view may be as large as one RPC message
- e5de89f4e9e25db7b04e55b0e2ab264a152793e5 — PLAY-F06: tactical fights through the battle machine (blow-only cues, outcomes never dropped, turn reopen), session views may be one RPC message, narration told once per change, chips clear of the narrator box and data-chips off, longer finish cap; first-view and tactical checks
- caddc17dfe5a08c98183e32531de35882777f2a5 — Merge the battle machine on tactical fights: entrance, staged server turns, exit, narration told once, chips can be hidden (PLAY-F06)
- 36c2bcd2284a2a0723138dda486baa9ee4c73713 — Tactical scan scenes: the server stages a forced scene on its own authored layouts, no cosmetic walking
- 82d7eeb4bbc19eb989e8a4d31b1842be2fa70dc0 — Keep action shots within 75 degrees of the scene front, preview shoot demo and story-place diagnostic
- 55d889ef3f1ca5d68d13e22193c1b15974ee6de2 — Delete the reduced-motion switch: motion is never reduced (camera, director, status, stage, figure shaders)
- 5033316664eb5463f950dd4ac7cbe96856658ea3 — Keep a combat view inside the transport message: the attached table log drops its oldest lines until the view fits
- 06e122b8641d4cbae6ffbbacc4c9561cdabca54a — Thinner grid lines: hairline core, narrow halo, fade toward the centre and the border, stronger reachable/target lines
- ed67871f6543d9bf80cfa35d6877f5f6cb6a5834 — Splat level of detail: DF_GAMEPLAY_SPLAT_LOD (default 1, the second tier) and the preview &lod=
- 63273b608121d84f08f5661b61342fd76e2c694f — Fight-loading splash: place, mark and real chunk progress hold until the scan, art and figures are resolved; the stage clock waits behind it, then one fade reveals the scene
- f2b518728ab115a14b46635a4eedbda4ad108268 — Who is speaking: speech bubble anchored to the speaker with a tail and camera ease-in/out; damage text names the attack, holds 2.6 s at rest (3.6 s for critical or knockout) and stacks without overlap
- 9ff5fee7f42c9bbb97970e1a7deaf15cf3cf58ae — Victory and defeat beats: the stage holds the party wide for 6 seconds from the deciding blow, with the sting and the heroes reacting, busy throughout
- a97a8f51da4b7487b36aae6f953ee9699a914fb3 — Preview harness: &heroes=n, &enemies=n and review demos for speech, named damage text, chained turns, victory and defeat; document the full preview syntax
- b7cfac687e78389365b36f69d5cb5034fcf744a3 — Speech bubble: set the SVG tail's class as an attribute (an SVG element's className is read-only and stopped the stage mounting)
- e37d098fcf3697fe15fb57c6a2e209123cf69dc0 — Battlefield marks data-battlefield-busy as the stage starts and stops playing
- ef0657033811062d92eb411d699fbb39c0b205e2 — Speech bubble beside the speaker with its tail to the body's edge; floating results stay between the side plates, attack names go in order per attacker; the party shot after a decided fight is framed tighter
- c83c44aefc06d986e3431ef13fdba866042dcb92 — Hold the party a little closer after a decided fight
- b2730b5e419eec424c40a60917d8e8975b02662f — Clippy: const assertions for the timing constants, scan tier applied to the layer before the load
- 65fb9b007c9fa661ee88cc46459a5bdde62d1c7c — Merge tactical fights on the scans and battle polish: thin grid lines, splat level 1, loading splash, speech bubbles, named damage text, six-second celebration, 1-n preview scenes (PLAY-S02)
- dbd63066426e48fc365e3a770d5068fc0756c3c3 — PLAY-N06: live NPC conversation transcript harness
- b691fada5ced3420a44786f7d84cfe5d32387803 — PLAY-N06: NPC replies answer first, know what they told this hero, and stop padding
- 3bbd8c41f779a79f5423900eb5080b6babf65c64 — PLAY-N06: keep names the scenes say aloud out of the private clue's terms
- 9c8790babab73fe540dbbb07942bf9a4e8d2c550 — Merge NPC replies that answer the question: per-question facts, private clue known to its speaker, reply guards, model-free rule checks (PLAY-N06)
- 95dda536650ac8f4e1d6be709f1efada61a18bae — Make the home screen's loop just in time from its still, served with byte ranges
- 4ba563787e98c61132ace307b54a8e864343b374 — Fade the home loop in over the still and cross-fade it at each wrap
- 5932d5eb051ca263863e60570c576d81e4ddb239 — Wake the home loop from the server's cache and play it on the lobby (PLAY-V10)
- ba8e3d5ca2b33505e938207da488824a1bc042be — Merge the home-screen video loop: a clip made from the lobby still, cross-faded on the TV, with its own budget and setting (PLAY-V10)
- c5af1e5418d5e2ae8feb105da955d7ed381f1bd3 — Re-pin the adventure record hashes after the archer copy and NPC knowledge tables

Next block baseline: `c5af1e5418d5e2ae8feb105da955d7ed381f1bd3`.

## 2026-10-12T03:33:42-04:00 to 2026-10-12T04:33:02-04:00

Range: 9c56992ca60d8049aaa4c230e642a877d2aba43a..5310dcf601c50b60ecf17c3d297d524a3bf65029

- 31cff3f34e26a6b64a9930ea259bd1b4d4f3be35 — Tactical combat in the live game: positions, movement, ranged attacks, reactions, multiple enemies and the archer build (PLAY-T02)
- e460e2e2fde4b0d22833af47d3fba35b23b6443e — Merge tactical combat: server positions, movement, ranged attacks, reactions, several enemies and the archer build (PLAY-T02)
- d43b7e0a65753a1e50ea9dcf08dc7e53dcf8a6e6 — Admit investigation choices and report refusals as typed
- 78b58c030a987242b4a9404d33042ce5d4518960 — Merge the investigation fix: scripted choices reach the journey, a refused action says so plainly, four choice chips show whole (PLAY-E02)
- c2635279996086d60cb49263418867a68f87e82d — Battle feel and attack framing: shake, hit-stop, slow motion, sway, particles, action shots
- 17433e1a17cfec0741f028d05d8944e541e8007a — Battle feel in the post pass and renderer, the page hooks, the story-place title and a quieter village grade
- 1351b78554dbe9a77d61cca1ec9695725637b782 — Attack and finishing shots off the line of action, pans that keep the fighters in view, stronger idle sway
- e6eab66db5d9180043f780fc75f2d8caecac58fc — Action shots stay on the front side of the set
- 0a7facb068d361bdf14f71f1826733396855ddc4 — Document the battle feel preview switches
- a849923fc33656ba7c574ace6d5b8e9df7f15ae1 — Merge battle feel and camera rules: shake, hit-stop, slow-motion finishing blow, sway, particles, and framing that keeps both fighters in view (PLAY-S01, PLAY-F07)
- 6286a345acf999712769b0e409f757012cdc6484 — Merge branch 'claude/playable-20261011' into worktree-agent-a507ceccb18c319e9
- ce718c98025e946ca2e9ba611fd1f2107cad8f01 — PLAY-N05 M2: NPC replies carry a conversation
- 6d2eb7241c1ad6d6fb1dddff6e5adae94fb9705b — Merge NPC conversation memory and pacing: replies lead with the answer, do not repeat themselves, and every speaker knows their side of the errand (PLAY-N05 milestone 2)
- 0c87a8ce3baa18e2b8dde9fe261a9adf7e895491 — Analyse each home painting at runtime into a validated home theme
- fb8c8a189634bdff642979ceec660ddb9665e300 — Merge branch 'claude/playable-20261011' into worktree-agent-a7a373d701ad1f2b2
- b3f975b20455f63a5a150ab0c6ab5b485ea704a1 — Place the themed home screen blocks and mood on the shared display
- e61f1bb2ce1ec2891f2f3c1da051e9fe49bf0810 — Keep home panels opaque and measure text by glyph extent
- ccea1fc83d522c121944d0f63780892dacc7c74b — Merge branch 'claude/playable-20261011' into worktree-agent-a7a373d701ad1f2b2
- b815b76da060c85d0336a281c625c4685b2b11ab — Give the join block room for the real QR and make the theme cache explicit
- 357049ede1a8d3bc4ea6944dab755ad893168950 — Carry the theme into the panels and drift the atmosphere, masked off the blocks
- 5310dcf601c50b60ecf17c3d297d524a3bf65029 — Merge the themed home screen: title, join block and party placed per painting, mood and atmosphere, with the real join QR (PLAY-V05, in progress)

Next block baseline: `5310dcf601c50b60ecf17c3d297d524a3bf65029`.

## 2026-10-11T21:23:37-04:00 to 2026-10-12T03:33:42-04:00

Range: 27e6e917073350e8f7dbb73adbadfff67ff34f56..9c56992ca60d8049aaa4c230e642a877d2aba43a

- 8280b8d517265e511f2370741b6c5f47c8689b2f — Add the battlefield core: camera, grid, director, stage, scene files, splats
- 6cc63aba8c6ee888cde846609ffcf8022773ae67 — Add the WebGPU battlefield backend, HUD and #battle-preview
- 9b6ef66638d204f691eac372937103073c69526e — Add the splat route, notices, combat feed, and tune the battlefield scenes
- 38f914a85b0a3d43208f06e2a7d10b38f1ca5358 — Merge claude/playable-20261011 into the battlefield branch
- 4803cf4a2757dcb2927c59000635289fa0c88496 — Battlefield: cue-driven live adapter, approach/strike split, roll card, figure bounds, lighting
- f0483927dd191e0a8860099d4de3d179dc5ba0ca — Battlefield: soft reduced-motion blink, reveal after art loads, clippy fixes, splat route test
- 02981ad25d45e63aeb74cd5da3cf5cf758dfcd38 — Battlefield: frame-budget ladder skips splat cap without splats, tighter ground, scene tuning
- c51ee69d17c206f074db426f8676193a4503364d — Camera: reduced-motion blink decays from its own peak
- 0546b194fdce4466856dcdca08a2bf9d4b8f7d9b — Merge the Rust WebGPU battlefield: 3D combat stage with Gaussian splat support
- 64f80b60b983e1bc1cf1fbb75ef724580530be24 — Raise the launcher database's connection limit
- 3936ab884353e41dffcd89f15b74ba5afa772ff3 — Add bounded speech input route and hold-to-talk contract (PLAY-N03)
- 3874ced86a3f31e50ffdc152e592fe3aec8f494b — Add the hold-to-talk control to the dialogue panel (PLAY-N03)
- 2830385bb7c2b10480992c02e159236be1e9615c — Document speech input and box the active speech route (PLAY-N03)
- f0464c0553698570b40023f13d66b08fe8a348c7 — Merge hold-to-talk speech input
- f9cc8bc9e5846414f1e302fb84d75363851f624e — Roll NPC dossiers with every generated adventure and persist them in its record
- 28a6e83fc571060387a1ccfe21e4ccd3270da35b — Give NPCs with backstories attitude, trust and in-depth replies
- 3bef1c4b17ae72eefa5f33953a3c8c59fe97636c — Test NPC replies, attitude bounds and persistence; document the dossier play layer
- 63484e991213266767f04a72350f19bf7cd6274f — Merge backstory-guided NPC replies with attitude and trust
- 1df12ab11cb69762aca534680967c9028f626219 — Queue battlefield layouts and sound, tactical combat rules and their integration
- 5324346027fd9fc8b98fd52343b97d9bd56f77c6 — Queue living scenes, the investigation scene and status effects
- d97605ce96a26865eb1ec9cfa6f79169aeffed56 — Queue real splat battle scenes and the content rating setting
- cba67abc87b9f49201841b2f6c2fdb0c20d0664d — Add status effect icons, battlefield obstacle props and the archer hero art
- e059c1ad19cb69cd8f0e9209b8b23ae751867ecb — Send rules wording, the place and the table's choices as separate view fields
- ab7940d3415f8e2867e23bc1298511235603be11 — Use the server's tempo cue for the score, with the view only as a fallback
- 185900788513afb88f87b3975d1071417b18a471 — Stage the TV from the server's cast and table log, with title and closing cards
- 46e712520b7ef5ba62d5fb403ba0a9cf3beeeb3d — Make the words follow the voice, restyle the scene title, hold the fallen, move the sound control
- bddda47d8359d2da3cff6f70fbf773d66cc2364c — Merge the integration branch (3D battlefield, figures) into PLAY-F03
- 54fecce9e58f5ccb1786ca729b2a8bf44f586c68 — Stand the cast in the painting: depth, floor contact, HUD, bubbles beside heads
- 812e956610786b1fe0cd4ed20d0dd54d6d62785b — Keep speech bubbles clear of a two-line place name
- 3129bddb2b09787649ff014ff677a0b79264bff9 — Merge the story stage pass: bubbles that follow the voice, depth staging, title and closing cards
- 914ad9b0098e7a0d40f78cf5f56cd3a83a369630 — Make the first press on the locked sound control enable sound
- 71f570f1264485eb16d404a03fb6252ed19a5018 — Show a typed line and the speaker's reply on the TV as they happen
- 568b9cfddbefd99b57b7ca4db7f6f2340aa8839c — Test that only a deliberate press can ask for the microphone
- 2e3ce6100bdf73f61a8f6cc6ed63aa5fafb3ece8 — Send the real release duration with a Release tempo cue
- c9dac077c50e411c19f857fd1caca83e01ea70ae — Reconnect with backoff, drop stale seats, release silent sockets and give each run its own room code
- 48b4076669174cd341b308eaae3f639bde69f8b3 — Add content rating setting with mature default, overlay tables and rating-split guidance
- cd3b94ebe802bb7d93eb68a066aafb95d7e0bddf — Test the content rating: parsing, guidance split, word lists, mature tables and limits
- e24c9048a6978c86eb79e043da5d281f73047258 — Document the content rating setting, what each mode changes and the limits in both
- c84501cff73fbcc63c0d0f7549442c07b6027452 — Add live content-rating checks and a menace sentence for hostile enemies when mature
- a6b040b3392107fa2d4dbc0bbc2065ff1fffe1d2 — Merge the content rating: mature by default, restrained as the switch
- 52245d9b9425f3e76027ee2576c8023121c58805 — Give slow commits time to finish instead of fencing the session
- b16bfcab36b98fd4e920544448f86a4545328d3a — Merge the integration branch into PLAY-F05
- 3c92f24123a992ab6ff902733c85fda5fb65682b — Let a fenced session heal itself and say so when it cannot
- 51642ddb4c32944ba89ff22b2cebbaaa486604e7 — Renew the owner lease and grants on a heartbeat so an idle table keeps its session
- 4d2ca0a3d60628299df5b028658cc9bdb817eb2f — Keep tuning bytes unchanged by the release hold and box the retained attempt
- 76551ddc21cfbf426d894491a79f271513b664b8 — Add grid tactical combat rules: movement, cover, ranged attacks, enemies and the archer build (PLAY-T01)
- b106a100b235ef97fef22d5b18a0c6af5d45e7aa — Merge grid tactical combat rules (PLAY-T01)
- 306a9507befec2d811e97f1451e8dfdeb4b82aba — Battlefield core: layouts, A* paths, status set, combat sound events, ranged staging, downed state
- 3d1ecd1041acd5929c82707e6969efa6ce833f21 — Author 27 battlefield layouts across nine scenes (schema v2) with validation tests
- 222b328488aea85f4d2cd9a298727c3280a45051 — Add 38 combat sound clips with provenance, content-rating clip sets and allowlist entries
- 5da4abd94a2a0c9593c555dbfc388919fa89abec — Keep the invisible outline in the resolved edge set
- 6578d8e44c3604db996d719e257fba4712ec109e — Merge integration branch into PLAY-F04
- 53cd456506758cf151afdcb504c946bc04bf1526 — Server: carry the recorded initiative rolls and the fight's location id in the views
- e0c72d1ff28691c0965a42d7259117fa6e31482b — Hold the impact shot after a damaging hit
- e5dcd94d0b9a0dbaab8e9369749ba4559973bc25 — Initiative roll-call: dice, whip camera stops, order badges, tracker fill, flourishes, rising stings
- af9e1a5dc352693b9c8bb81e42f05c9bfe6ae82b — Renderer: props, tile treatments, arrows, status shaders and overlays, HUD pips, target tag and initiative overlays
- 9315dc275b938739bff2935f14f17dfc3ff0ac2f — Adapter: location-keyed scenes, layout choice, archer, statuses, initiative, sounds, preview modes
- cf3af19943d5762f49b1ef0d3d609c1b0df3fb37 — Fixes after first browser frames: figure mips, softer ground sharpening, screen-side archer mirror, roll-call tidy, phone initiative line
- 705cfec31ad2cc7a104d3afc6e635c5b94b91651 — Visual tuning: ground detail, rings, aura, invisible body, review placements, rustfmt
- 3015007ac2ea0d40e1e723d6b307d7fe9ecd7ea3 — Clippy fixes, ranged impact framing, cheaper ground shader, faster quality adaptation, close stills
- 5880597872f3b525e7c52eda47e7fdfccfe7f128 — Arrow and cover pip legibility
- 4990f4bed9ee63b19c1fdc46696efcaa437cadb4 — Merge the battlefield second pass: layouts and obstacles, combat sound, ranged staging, statuses, initiative roll-call (PLAY-F04)
- 62105b7abf06f10b3632ebc5557a1b2b6df77db3 — Drive scene flow from a validated beat script per adventure (PLAY-E01)
- ceeafa3eb47302a6d27a383cb8a01699d5855a24 — Merge the data-defined scene flow: beat scripts interpreted by the server (PLAY-E01)
- a0e7977b75968490846b2ecd9714b43c27d0071f — Design backstory-driven character creation mapped onto SRD 5.2.1 (PLAY-C07)
- ff9091dae662c37fc7cb90aec57f93d4eed78a43 — Merge the backstory-driven hero creation design (PLAY-C07)
- ae919ff8f85b17dbe591b66e592523519c668b6d — Queue the stage pass, live-session fixes and backstory-driven hero creation
- 5117177abffb39ce15a9c848639a10cf50f24a35 — Queue speech-led display sequencing with explicit stage state machines
- adb5030d0a065fe6bad2695c627a9253c5a86b30 — Merge the integration branch (tactical rules, battlefield pass) into PLAY-F05
- 8cabc2aff1e015bc6618face27c06be74df16254 — Browser checks: fight loop for the tactical combat and a release-hold measurement
- 8e0315aa2db46442d92b2ecc5679c956155d0727 — PLAY-F06 (first part): a scene change waits for the line being spoken
- de90e81ba12b312a0f3443cf335cbb2ec78fcdbe — Queue hero speech on the table: intent bubble and cleaned-up line
- bec356d0db258f8c66c106c56501105a31011e95 — Record the owner's decisions on hero licensing, creation time, persistence and secrets
- 21cdfbeb3592de0d1afe5f77192632c8b7a7727f — Queue the backstory creation build: hero tables, picker with saved heroes, catalog validator
- b09971c7247f7aa00fa6cf26d47f127628d8e014 — Fix the stage of the hero tables task so the queue file matches the database
- c4926dddf94730982cbc1f35759cd00235121dc9 — Add live_backdrop and live_sprite presentation fields for server-generated scene art
- a204edb12b808cc53dcd313e5d66fa17a09358ac — Add runtime image service, scene mood and living-scenes director
- 3002719e6bdc16eff7b35e95bbcf73399ca53aeb — Accept generated scene art in the display: live backdrop, expression sprites and a fade-out hook
- bfe9a64f753181485b5b27b34bdaff1510bb0c9f — Test the image service, scene mood, living scenes and imagery settings
- 517992fab575d55e258438242b3242cfb52d049b — Rename the two-person shot, tighten expression phrases, add provider tests and the provider research note
- 746dfe97843922c22b1eaa18c2cbb66c3e440b3c — Wake only the display's stream when scene art is published
- 5dfb8762de358560b86debf3f93bf926a0b04dd4 — Let the display take generated art from a view of the revision it already shows
- 15cf5aa82425d60267576fe69cbe6764964149c2 — Keep backdrop prompts from inviting painted people; add the live operator checks
- b82efd3ff145ee8b647fe8890682179be2f89167 — Merge claude/playable-20261011 into the living-scenes branch
- 33879004d1eb5489cf3ff5a7dafdc6116a58e106 — Harness knob: DF_GAMEPLAY_DEMO_DATABASE_PORT names a private PostgreSQL port
- 5197d388f257e8355fe68129c19c441d8705b611 — Merge living scenes and the runtime image service (PLAY-L01)
- 29d90960e662dd698c6ddbc6e807df9e01403dc0 — Queue health-driven faces, bar colour and shake
- 9d14b3b4991eebf785c2701824fcfff11c5372bb — Health cues play for everyone: drop the reduced-motion exception
- a9413cb95e18d372492a3cf90b60a47d308b2899 — Never reduce the game's motion for the device's reduce-motion setting
- 7aa4f26befbcce1e2492dc3c8b6050d688c7cb3b — Merge the integration branch into PLAY-F05/F06
- 08d846ad802530ef0a3cd5c055fc2b27b4277a03 — Merge live-session fixes: lease heartbeat and self-healing, typed lines on the TV, sound control, room codes (PLAY-F05)
- 1461c77c2ff3ff799e6c049651ff56bf53143d4d — Record the battle frame-rate lock and straight glowing grid on the splat scenes task
- 335a9c330cd1bb3bd60009a03d12298f6a4f2584 — Add hero creator data: life-path stages, jobs, custom backgrounds, SRD catalog
- 926f00aeb9c5a7b01704710f9e7f90b710085c51 — Merge the hero creator data: life-path stages, jobs, original backgrounds, SRD catalog (PLAY-C08)
- d99960b6b4dff807ee34a2ba2edefd5bff062eaf — Queue composed scene exploration, the looping home-screen video probe and the theme-driven home screen
- 8ae203215068e77c65c64c3213494cf6e504fbd4 — Backlog a table-chosen graphic style for the art pipeline
- 699961607f23ba3928f67eb61335f7aadf73eb83 — Put speech bubbles above every character and require strictly rules-backed fight presentation
- 2fa3cfa9e51fb61fab2e45f7873f7bc3cafe6aa1 — Add a pure-Rust image decoder to the server for runtime image analysis
- f30da76c2f725e54100957746f50087ec3faf1d1 — Queue battle feel and the video dice roll template probe
- deca89802f5600ac1dd55ae24974eef980c2a0a9 — Add splat LOD index selection and scan ground height estimation
- 2721881a83049cb1c19cd7b9736bac7ee38e7eb9 — Stream and render real splat scans on the GPU
- e23745168d58255984b735489aa78c27df7ebaf7 — Serve nested scan layouts, add splat directory and battle scene settings, real-chunk decode tests
- 68a8aef3a73026b195696a65e0fcac7c6944cfc2 — Terrain-following grid tiles, GPU pass timing, splat-resolution ladder, ceiling cut, scan scenes and fit tool
- 5244410f1b542ff39ebac8ee0e7b7dc191f11d7e — Pass scan settings through run-gameplay.sh, document scans, tolerate missing chunks
- 2dc74c1fd7ead8936bff5d2ad16b2c07bc3a9659 — Extend third-party notices for the splat pipeline and the CC BY scan
- 2d927a7f559dee04fdf9082263249f1457093959 — Test scan scene fields, terrain-following figures and the GPU depth key
- 83dcfb9c84addcc064fc97353797e2ee4dd9c59a — Format, lint fixes, depth-off debug switch, figure positions in stats, nearest-first coarse loading
- d2a955be85e63f3b213f8e62f65942da6782230c — Tidy submit_frame, final scene cameras
- fef7e81000b8c83dd749a69b4900a4389949b229 — Scan depth fixes, decal lift along the line of sight, final scan cameras and scene biases
- f2070e3840db86d87e407832d36abe0d3af62dd2 — Merge claude/playable-20261011 into the splat scenes work
- 8a69c07d573debe8fe276b224092840026220207 — Fixed 24/30 fps battlefield cadence, straight glowing grid on a flat play plane, painted-size scan arenas
- 4b1d9233c0ec6ebaa1cf071ab5b0fe02a2fa6c0b — Eight by six scan arenas with a closer wide camera, lit scan grades, enemy card clear of the sound control
- 4ff3c0baa9f85573a2cc23ac9c8163a7de16c1be — Merge real splat scans as battle scenes, the fixed battle frame rate and the straight glowing grid (PLAY-S01, in progress)
- 835bf87b6dab9f8ad2d25f61e777854b5a3858fa — PLAY-F06 M1: display scene, line, clip and actor state machines
- 7164565c16869f24ef7a867568aa04b6fcb897b6 — PLAY-F06 M2: battle entrance machine in front of the battlefield
- 014ac0dc849d024be77f15bf4bf04f90eb96676e — Merge display stage state machines and the battle entrance machine (PLAY-F06 milestones 1 and 2)
- 14d3dcc8801363accd4332b07bcc0ddf72c1b144 — Add a QR encoder to the server for join codes on the home screen
- c7b6c7ae2f3f77944481631c39007c8d748d6194 — Figures on a scan are never hidden by the ground they stand on
- 758c9be73056f4e32446dac33e44a390e07c06a4 — Merge the splat-scene depth fix: a figure is never hidden by the ground it stands on (PLAY-S01)
- 134d297d1aa0fd8523c0bb501e8bb3d173650333 — WIP PLAY-H01: shared health tier, health faces, WebP output (does not build yet)
- e9cc29c622c5656848a5aaf3f55a9730eeaefc4f — Merge integration a9413cb into PLAY-H01
- 630d06bb8bb71947766f8e3c34aae9969b19b153 — PLAY-H01: server health faces, pre-warm book, cached-only combat, hero live faces on every audience
- 6972c8cc6a98986e5c3ba9c4669478eb02634a73 — PLAY-H01: browser health cues, bar chip, shake model, phone card cues, fight-scene warming
- 47cb54d1a88b207fe633c30bdbfe3d23ff61450b — Merge integration 014ac0d into PLAY-H01
- afd48825b12932ba0af351e3c9957f975e52d0e1 — Show art that finishes after table talk: merge it into the on-screen table-talk view (live defect), with a native test
- 6da991d04f953bc8d2111d0b05cf7feff38b88bb — PLAY-H01: retry a failed health variant once after the untried ones
- 79cac842bf283033dbf4e17a43a7ee12752a3cae — Merge health faces, health bar colours and shake cues, and art that finishes after table talk (PLAY-H01, in progress)
- f7771244f2d0ca3a90996a48d9ff470229f5aa1b — Phone: attack switches no longer block ending a turn, and start switched on
- cd51d11bcaba3cba6ab7cf807480180327e52ff5 — PLAY-Q01 M1: public join address, join link and join QR code route
- fa101a25c6cd52538d41469be9ba3cc4bce965ef — PLAY-Q01 M2: QR code and room code in the shared display's join panel
- e90b39aabfdc9cc8301b26769672e88c887f6243 — Merge the join QR code, room code panel and LAN join link (PLAY-Q01)
- 1e833d5a386f86c23393f68ee9eda495b36d9a53 — PLAY-N05 M1: NPCs know what their adventure asks of them
- e3b5dcc0e593544ffde8fe2a11a7d2737302ce49 — Merge NPC knowledge: a speaker knows the errand they give and the names the scenes speak aloud (PLAY-N05 milestone 1)
- e88694fb1951a05a8aac18a83510d84b2102a446 — PLAY-H01: move the table-talk art test to the view delivery contract, asset-engine note
- 9c56992ca60d8049aaa4c230e642a877d2aba43a — Merge the rest of the health cues work: the table-talk art test in the view delivery contract (PLAY-H01)

Next block baseline: `9c56992ca60d8049aaa4c230e642a877d2aba43a`.

## 2026-10-11T15:24:05-04:00 to 2026-10-11T21:23:37-04:00

Range: b7216f693e62c37a2c1abd3aeb31e247b9e10491..27e6e917073350e8f7dbb73adbadfff67ff34f56

- 0b32c6b001673db1dd4862dd486a7ffc219afb50 — Close development session and preserve reviewed progress
- 162251c20d40be40db79fcad9f964fd311b92559 — Repair df-presentation committed eligibility test against current contract
- a8be0a5daf4a6f77c3c2f1a80ed1c8accb69f90c — Mount owned dialogue clarification and confirmation recovery
- fe85a8b80c1da54ff670b43e2f21e6713f568876 — Preserve ordered dialogue recovery guards with let chains
- 9c8423e49c5661d222d8573d837edcb78b54d38d — Make gameplay demo session limits configurable
- 0e30ccee26ff84c8528e9bfdef4ad2620c40362f — Add a one-command launcher for the playable game
- f7f32db64aa81f5a81156556a0ecaab3ad6327fc — Add the playable-and-tunable track to the operational task manifest
- ee7720b2fc07927893ee2d6cdc01ab0792f1d35e — Define gameplay tuning surface
- f6e3b4df2ccf5665a3e5c9a56339883a7d6d93d3 — Resume players after reload and tidy the playable client
- 5adfac5802f8bb3b255069ed8fd958b10d5a18b9 — Keep the saved player seat per tab so two players can share a browser
- 1d6d86b9a1de4c4126a8d8c137f1465345c19db1 — Bring the README, agent rules and start guide up to date with the playable game
- 1ecbefd0dc5ea43e379342606a6f7f15337d1f1c — Do not exhaust the call budget in normal play
- 0a12ab546208038275f2fed4e675b0e2d3ce9815 — Parameterize the local journey rules for tuning
- e24907024e8627b7e356bb0fef6c16678dfa280a — Add the gameplay pack and tuning loader with shipped harbor journey files
- c3cd3c06dffddc8f3e4597397e743f3ff125b051 — Mount the designed lobby and exploration screens in the live client
- 7d3766b53c1ba32d124ab780e1216afc22b87a82 — Test the allowlisted extra asset route
- c44c81833d8833d585a09b1b3c74d9e60e9cb704 — Add generated interface art: wordmark, frames, d20, icons, portraits and scenes
- c8ab9c72b02d7085bcefd6840652db1d61fa597c — Refine lobby and exploration screens toward the concept art
- 7e00920c4fc74c88b5c5ccf4cc8eb5ca8c53598f — Mount designed combat screen in the live client
- e8ef9559b3ebaf8cea28a567aff573b17d7402a2 — Mount the designed combat, aftermath and sheet screens in the live client
- 0d911ad07668a779a4fe2f79e3781b93d0dee310 — Merge playable branch and mount combat inside the display stage
- 1c2ab9845c60b0ea5e064810d52d415ef95bd5c6 — Mount the designed combat, aftermath and sheet screens in the live client
- 9469df159137e57c2d9f59db9cf367bc5bdaf09d — Merge the designed combat, aftermath and sheet screens with the refined lobby and scene screens
- 1b9be2be446085ca5764f8f94e5b283e137333e8 — Drive the game from the tuning and pack files, bind their revisions into the run, and mark custom rules
- 6dd6a4adc62edccc7c57c34976eabbf77415ac3f — Merge the tuning-driven game with the redesigned screens
- ae7aab83df71221a6332947518799b4c34c07d6e — Voice scene narration and NPC lines on the display
- 392de25e68b9b0712570d56efa1ae7bdd5e36971 — Polish combat, aftermath and character creation toward the concept art
- b317ce6d68c59d54b05fb60657f687cdefd4948a — Size the display sound control for a TV
- e4ec0955d785e9771c0ecc62df919a56e0553472 — Design the random adventure generator and author its roll tables
- 54ed003ad44905c6ded92a2bf4f9b1d58fccc113 — Add the campaign layer to the adventure generator design
- 2196f2c06ab56d15a03096b05e36f3af48badb10 — Add the validated storyteller expression service over the text adapter
- 15e97d9325b51148e528f0b10fde7f2cf11abd27 — Let the storyteller model voice NPC replies to typed dialogue
- 34e414651cf6654cfbbb4789382e7e1b4b6a77c4 — Let the storyteller model voice NPC replies to typed dialogue
- 169f7ac7fb2045319d8288155a58f9db49adf1f4 — Add the generated art pool for the adventure tables: locations, patrons, antagonists, contacts and villains
- 06323d04c24b6de65a158ef4ed36639112132375 — Generate a dice-rolled adventure for each new game
- 2134653015b30c925ef6e4240a812ede5d8564e8 — Merge the adventure generator with the storyteller, voice and redesigned screens
- 2b57797454a91e5189d67dc050b12aa220f7b876 — Draw generated adventures: accept served scene art and report undrawable views
- 3e6ea4f998069236abc5a77989431419135b5f9c — Fix the display crash when enabling sound
- 2d0690e83db44f40549727a9e55db5f01bcb40aa — Add the first character cut-outs for the composited stage
- a641dcc01dba6a61243358173778debab4de4448 — Queue the living-cast work: NPC backstories, backstory-guided replies, spoken input
- e5e1a998328f3e2479fc2bc31fd91bbd6572583f — Queue data-driven scene flow, player photo likeness and pre-rendered investigation rooms
- d4ac3a1ff599f44c7850615ecfc19aeb04c46c97 — Add generated adaptive score: layers, stingers and ambient beds
- be08de4d7948441ab68b3f2f69de54d2cadf0c3f — Add adaptive background music that follows the story's tempo
- 4fd22af736423049b9ce1421362b5e74adf519ce — Merge adaptive background music: layered score, stingers and location ambience
- 372d3aa9f480ecafc5c0ee7538343038347624a0 — Add the campaign layer: rolled frame, chapter chaining and a durable campaign record
- 4fd834a5567f0974a33071a9615f5c86b572e78a — Merge the campaign layer
- 4733a7be40a7258a6bbce2413da8006b03247720 — Queue scene settings with matching backgrounds and gender-correct voice casting
- b8af8e9b7b3fc2056feeb26e8a562735d10e3d0c — Add ten meeting-place backdrops and more character cut-outs
- b1a47b5af4162afff91db6cdb2ba6c7ecd291c94 — Queue the staged battlefield (owner reference video)
- 17741a53b129be6308cb04cb41130dccb8089778 — Rescope the battlefield todo to a Rust-only 3D splat renderer
- 8e47bae789e77cb18b27682aca76f9d2a001fe5f — Add the storyteller expression step, explicit secret terms and speaker profiles
- ca3bd33374028490e44afbc52dc3b1dfdc5c31be — Fit generated text on the TV, fix the aftermath eyebrow and lobby lede, refine the DM guidance
- 3ba85ef721b0ffac7ccd58f5070a8590f0e45168 — Keep rules wording out of narration, add scene cast and the DM lint
- dada79f02b22c47ecb0cbb4b0d5bdb54b2e43ce7 — Publish a public table log for the shared screen
- 14a4a13c58edb87f980c98ed5d3bb02e5833cc77 — Record gender and age explicitly on archetypes, speakers and cast
- f7f1eb4abba14ae5fc2112d9b6163709b1a75237 — Flag narration inside spoken lines; fmt and clippy clean
- a6b9510f63836dc32b5b2445ebbce58a65287e5f — Merge the storyteller-written adventures, scene cast, table log and speaker profiles
- 6622bbd1263da35630f0abd80cbda0b0cf0f7eff — Add provider-verified voice casting roster and its source metadata
- 930a4da7e7b8cd1851e5403f13e15f9e6e87d877 — Cast each speaking character a gender-matched voice and key the audio cache by voice
- 6379fd5c2c7b699d2d366855791bbc7c6ca09d71 — Merge gender-correct voice casting and join it to the storyteller and campaign work
- 119fcc970e947b000e51bf7ab4d2a9dbc000d53f — Add generated tabletop sound-effect pack
- 6c775fcf311151c9051723f3851319d75ff5fbc1 — Add heal chime to the effects pack
- cc0a2d1b9efa22f80be1e8a62dd752fa0375a91a — Add presentation planner, stage director and sprite stem rule
- 8a4a30d788ebae608686b9896868cfcd91cf78b1 — WIP: stage, motion, director wiring and effects stylesheet
- b75a9acf9bf358c40b7133eaa233cba2dc4a4a56 — Fix combat stage layering, seat glyph, portrait fallback allowlist; darken backdrop and enlarge the speaking figure
- b20e0f87bc1d53e42b52187aa6aa50f60ac2ee10 — Add seat states, dice and scene animation, transitions and sound effects
- 1d860043264384e651803cf8761a04dcfefbbf54 — Merge the display feel pass: composited stage, director, dice, transitions and sound effects
- 07b89203b5e13bf87d43d15de57a1254cf66275b — Add the hero cut-outs and the remaining villain, harbor and antagonist cut-outs
- 3fdb343b41fdb020acc5fab099d8e9f35110170b — Wire the tempo engine into the running game as a live intensity cue
- 4af2745b954af02406397c0b34b05693dfb95842 — Merge the tempo engine cue
- 5ccc8eba3eb6719828849f417dfd35c149ab598f — Give every generated scene a defined setting and a background that depicts it
- 21cb00eee283cf615d82f1ad26ccbbed99bf66e0 — Merge scene settings with matching backgrounds
- ed5cc8519e684c18a4856bb745d08649710e5524 — Add NPC dossier roll tables: background, character, story ties, knowledge, social
- bc251b0428faa842c15c2d24cd46c84ff6020399 — Add pure NPC dossier generator with per-NPC dice streams
- 254e8e8d626e5674ffff6a2ef083a0db21f17bd4 — Document NPC dossiers (section 19) and refine dossier tables
- f3c4da535476959cb4699d84e61b17df762224ee — Split dossier rolling into helpers and tidy clippy findings
- 36d9c029ff0607c6fdc3531ed4692f5810ccbbec — Keep contact ties inside the destination's own location
- d28eaca154c231473111a8e6f2bcbf7d7c0079e5 — Merge the NPC backstory dossier generator
- 155a45c648a74e972ebaa27b435e95388d2a3756 — Enable every destination with its own painting and move the library fight to its forecourt
- bc773d48771bfb938a8617beec6c4ce667290166 — Add full-body battle figures and top-down battlefield grounds
- a5c4f119530d912a3658b3600b1c73647a25f1ce — Add table engagement model research and design (PLAY-X02)
- 4a042f6d9d45ca829a911bf5af438a7078f82922 — Merge the engagement model research and design
- 821df2f671fb4667be98149a820697e701bf8505 — Queue the engagement model work
- 27e6e917073350e8f7dbb73adbadfff67ff34f56 — Cast the nearest age when the roster has no voice of the speaker's age

Next block baseline: `27e6e917073350e8f7dbb73adbadfff67ff34f56`.

## 2026-10-11T10:38:04-04:00 to 2026-10-11T15:25:07.071270-04:00

Range: a4bb079666eb54894e868fddcc32cea7658634b8..b7216f693e62c37a2c1abd3aeb31e247b9e10491

- 6f22f41fd67a9ae25321a3b10d866b8f0e69fc29 — Define UI keyboard, target, and reduced-motion behavior
- 7e2c0bb96236db6419e0029c15bebc69f08cf1e7 — feat(ui): preserve drafts across honest operation feedback
- d6f099b00439e582ca7110d5a5219d8a14ac019a — Define bounded player action draft receipt lifecycle
- 98c1010829037138aec6c131e0075f78146d13da — Add receipt-fenced captions and exact playback drift
- b7216f693e62c37a2c1abd3aeb31e247b9e10491 — Test persistent player keyboard and focus navigation

Next block baseline: `b7216f693e62c37a2c1abd3aeb31e247b9e10491`.

## 2026-10-11T04:38:04-04:00 to 2026-10-11T10:38:04-04:00

Range: 03e9b484d3da50a5e3f702d8b34b6687cce9e363..a4bb079666eb54894e868fddcc32cea7658634b8

- c9374a3247b30665c66c042f11f68d71b1618997 — Stage source-admitted noncombat encounters through registered rules
- d03b4f9ec335f57d1602ce2282ad46ea781fe20f — Integrate source-bound noncombat encounter rules
- 912ce2a39cdfdd945c3197b9c21caf41c977ddb2 — Connect accepted logical time to atomic paused world scheduling
- 9fc2d8e590205eb8f053eca2ee76ce540cc5fc2b — Correct world schedule duplicate-operation refusal expectation
- ca3e5a59233fad9038ee7d5b35eb18d3c668ea78 — Integrate canonical world scheduling contract
- eea4a0f39825d35a09ef29c26741c8a6243cd5dd — Admit authored thread ending eligibility from exact source consequences
- 2dc85511c79daf12303d85c6656540a4f99ec04f — Integrate source-bound thread ending eligibility
- 78f33c6d50b06d6c4d796c7965f5a16bbeea508a — Connect checkpoint participation to consent and accepted source evidence
- 2b987d2ebd3273e4a88b8df516f87ae57320577e — Integrate checkpoint-derived opt-in activity observation
- 91f27a511ae3fb3510c5d720f5192078e4b7406f — Connect registered NPC relationship axes to current accepted consequences
- 13a55b85932091f988a594564edd2e00ac633427 — Integrate registered NPC relationship axis staging
- e5b191c4d7a4aead67a76488f1d812701db7f279 — Define causal world ownership with executable Rust contract
- be1db4f3515db61041e1fb434af57408366ee8c0 — Merge commit 'e5b191c4d7a4aead67a76488f1d812701db7f279'
- 456283ac84d49d5053656e39c8e14d6f774c78ef — Restrict tactical scoring to the current actor's perceived evidence
- da5068b174664adf7c952a3351e8e01bc23178ca — Merge commit '456283ac84d49d5053656e39c8e14d6f774c78ef'
- cce86edcda8c109f893bb651a78aadee9c1d13bb — Register owned native jobs before task execution
- 27309ef7609ab9399387d66d9ed3dabd219295e7 — Merge commit 'cce86edcda8c109f893bb651a78aadee9c1d13bb'
- 554b98d8d490e95c22c6cb5954d739f7863bdc9e — Specify canonical host pause checkpoint and session contract
- 0e63cab89a994651d51032fbd7b5fede23db4e80 — Fix host pause fixture durable pending status
- 86e312077e1d0188be2b6a623bd9461776bdcefe — Merge host pause checkpoint contract
- f6afc4f0981eb9b155788cd060e9d3240baf8822 — test(engine): define compound discretionary source boundary
- 903cd898d38b4a26c8fa84c7739643ef106621da — test(persistence): specify honest recovery epoch view
- 326f9eaa608813eaeec4b6c9026c9f4a9d3488c2 — Merge commit '903cd898d38b4a26c8fa84c7739643ef106621da'
- 76a0d12a417db5cd1b43a4a57fc96c4db037aafa — Qualify private capture and output lease routing
- a4bb079666eb54894e868fddcc32cea7658634b8 — Merge commit '76a0d12a417db5cd1b43a4a57fc96c4db037aafa'

Next block baseline: `a4bb079666eb54894e868fddcc32cea7658634b8`.

## 2026-10-10T22:38:04-04:00 to 2026-10-11T04:38:04-04:00

Range: 1852c00eb40c3353652b24f6c024f54924c2d492..03e9b484d3da50a5e3f702d8b34b6687cce9e363

- f22381e3bb67607375ae648685fb54378565f156 — test(world): pin deterministic due event cursor contract
- e969e95b9d525d1d5458121157072dc5cdca5c28 — Merge deterministic world due-event cursor contract
- 59779a0cb3a19a6146c8b933b5ce8f639723d766 — test(narrative): specify bounded intervention budget contract
- 2f8b18de0ab63902859e1b432a6eee0a9b0db946 — Merge bounded narrative intervention budget contract
- 2f069f88362e87f814576e3a903eaa30d6442a7f — test(engine): pin character reference identity continuity
- b05a39731d66ddfbf4b83e880d0c2c00944514ad — Merge character reference identity continuity contract
- ea06100ba6ef6f54fc8c7c8ccb3c90c826acc771 — Define recovery epoch ordering and acknowledged game loss contract
- f2446cba7f1de99318b848e855a361fd341cb324 — Merge recovery epoch loss disclosure contract
- 85933449298436b972bb732a1cad6ab37cf8ce20 — Implement native ElevenLabs transcription and speech adapters
- 48be539e195012326b859a389a61743be4453dc3 — test(providers): compare speech locale with admitted canonical tag
- 165470df05a4dc0530c9e982b261712c019aa6f0 — Implement native speech and transcription provider adapters
- 7c1ea0b4e87a15d1183c846b1cad529c23e62301 — Connect admitted gameplay encounter initiative through canonical engine
- 132e916fbd646b47042d04956a6802448069d4e4 — Implement source-pinned journey initiative and encounter admission
- 0096913086255bcd027fc712970f70eeb3332063 — Pin witnessed fact projection to the current audience basis
- c2944e404cbf58c49cc63d5e9fa07da5ef70fcdf — Merge audience-bound witness provenance
- fb037eb4a29658b2c68bd32ec112abec49d2c47b — Filter retained journals by exact source decisions and observer scope
- f19535a91298570e2d99b306babde02d94b716cb — Merge source-bound journal and belief selection
- 6d81d98a95665f49873ea157716d5e210508d861 — Validate retained causal outcomes in committed moment selection
- 81df590363bb7857d2f42ad9f058b4028875dc3a — Merge causal provenance for committed moments
- 21808ce1d3e35ece6a77c8f6df262916f3a2a464 — Implement bounded native Fal image video and sound adapters
- e70e3ce07a944743b69bf9386ac8815bb92b0dca — Fix blocking mode for accepted Fal media fixture sockets
- 5a65884da066398d6df349150cbf113c48a999d1 — Integrate checked image video and sound adapters
- d0633bf4f6799f805e65da1848985030c0cc1064 — Recover exact room joins after uncertain durable commits
- 6bbf2b7cb18a46edd2173b15eef6341ca1133163 — Integrate durable room join recovery
- 35662ef3f5bd9860b1f4d427d366b7a66b09ea15 — Wire authored beat choices into schedule staging
- 7dc753ca0e8c1e640994f480072700dbb34ef5c6 — Integrate authored narrative alternatives
- 8662f90d3139e4777bfedef23ad077373bb788f6 — Admit source-owned encounter objective end states
- 303e6468f8cfb8a234e624701b5d9b7a47b89613 — Integrate source-admitted encounter endings
- 0c6b5328b041f599d282fe9fbc59d5749364b7d9 — Connect bounded Tempo intensity staging to Engine candidates
- b30be8a0b571cd63f0d9b6d477f50b37160d513b — Integrate bounded Tempo intensity staging
- 83ccc6d8d6bbb36554dab9f525ab80620a3fdfd5 — Admit source-bound registered action candidates
- 5daeeb822621160801d3b9756cfa1643634de9fc — Fix registered action rejection test fixtures
- 1729804f70d9b04b19c5ed9e52320ea1c70f606d — Integrate registered enemy-action candidate staging
- c22698c70a947dbe9f29280f86e391eef46cbb85 — Gate dialogue mechanics behind explicit final input confirmation
- 03e9b484d3da50a5e3f702d8b34b6687cce9e363 — Integrate source-bound nonaction dialogue admission

Next block baseline: `03e9b484d3da50a5e3f702d8b34b6687cce9e363`.

## 2026-10-10T16:38:04-04:00 to 2026-10-10T22:38:04-04:00

Range: dbbc39502ab0efabc9a7b468e01272f3b536a70b..1852c00eb40c3353652b24f6c024f54924c2d492

- 827c7eae3f50f7ab286e3fceb491a84747dd790e — F01 D02: add campaign readiness aggregate
- f90f1cbe9a7ddda8da9982e4bb762a8a7916bf78 — Merge commit '827c7eae3f50f7ab286e3fceb491a84747dd790e'
- 25dab356fa0f7039204eb6cb9533e0eaec22c25f — Implement invitation redemption contract
- bb8100ab9a1648f6768c24867fd5298324381ae5 — Merge invitation redemption contract
- 1c8db987123cb4ef7a1235db2c8e9b4c3d5c6231 — Define checked provider modality and recording contracts
- e402cf556820689baccea1393ff5c78d65ae9687 — Merge typed provider modality and recording contracts
- 2a793cd652120cddf841a1d3260d8ad4a21e1631 — test(interaction): freeze canonical boundary and consumer contracts
- ff707a601347f18bcd2d4eb7a719859cb50a3304 — Merge canonical interaction consumer contract
- 9aaee8831e4ed968539c9db615e0773b870dce54 — Add public shared-display projection boundary
- 8ccd9bc8addaf8bf51af64abb6063b0d29dcd170 — Merge public display gameplay projection
- a3d0a6be8ac42c1f4c799f7a5cecedb4acbe640c — df-commerce: add bounded settlement policy and native port
- c7896fa4327afc3a7811e3584cf6cbb3e182fd0d — Merge commit 'a3d0a6be8ac42c1f4c799f7a5cecedb4acbe640c'
- 2b32341e33662994b30ea6c6250e264ad4ec74ba — fix(df-model): prevent resubmission after character acceptance
- 731da9010905ef63ff50011b53724827bfe4c88b — Merge commit '2b32341e33662994b30ea6c6250e264ad4ec74ba'
- 4fea3b3cb8907e605fd393ae6eaf46bbaae6219f — test(intent): pin semantic boundary contract
- c31dcde0727d89a31f6c525f3832ddf5e24ec0e1 — Merge commit '4fea3b3cb8907e605fd393ae6eaf46bbaae6219f'
- 1218ae85db321949ce87f53da0f3f1f0467d4267 — Implement pure session readiness decision policy
- 4211953991fb2bcd7acadc468e23d3050fcb6609 — Merge commit '1218ae85db321949ce87f53da0f3f1f0467d4267'
- b8a293612505501d832064ab0f9f51a89bc29df4 — test(df-client): specify resume checkpoint decisions
- cf976b0a137367ffb4dcdffb2d96a1265ba28e7e — Merge commit 'b8a293612505501d832064ab0f9f51a89bc29df4'
- 71797e30d62c3bf7c167fc217b8199bd4468235c — Validate whole affordance batches before cloning and bind rule provenance
- 6a219f85b7ec44ed795b50bf0c38f0f8d56b1975 — Merge commit '71797e30d62c3bf7c167fc217b8199bd4468235c'
- 3cc40ec536d1bc23fc196ad5226b3771ef723ba7 — feat(locale): admit canonical catalog source revisions atomically
- 8c5043bdaf1e5734aa69d5e5122d33d9d65ad5a0 — Merge commit '3cc40ec536d1bc23fc196ad5226b3771ef723ba7'
- e907138fb0338839a186e6647dcc34da029b837c — test(narrative): cover scene entry contract
- ab056d6fcbcdf8fc273d46cb83ba0c01075d8d37 — Merge commit 'e907138fb0338839a186e6647dcc34da029b837c'
- 0dbd5c958ec7df6c37b64a133f164a07cb52a2a6 — df-client: fence asset cache fetch scope
- dd07f35021324dfcd73723e6de53c5788a5054be — Merge commit '0dbd5c958ec7df6c37b64a133f164a07cb52a2a6'
- 0b2997fb72d19edc014bc4e3b3946fe6b897cf3f — test(render): qualify supplied scene layer inputs
- f7866b31beb729a037786a03243c95f9fa179a4f — Merge renderer input-boundary qualification
- 0c1ffd0a97dd530e3138e0c5bf3948bbf9b83089 — Classify provider failures without releasing unknown liability
- cc1b6ba63b0239df837d569990659e46215e6add — Clarify provider documentation list continuation
- 359836188208ba8691167553209a029aa9d996d9 — Merge provider recovery failure classification
- baa94fa3b844cd41a42b4edd004d67174187c79e — Bind native display pairing to current publication authority
- fae2e4c95fbdae9c55db193200584337c946130b — Merge native display pairing boundary
- b515365f479f788bb2fb754f2770675059282d7c — test(tempo): register recovery policy contract
- d238b002398caf6ef0fa4a79900f542db7a7d6c2 — Merge fatigue recovery qualification
- 15bda8a9bcb4bc50c824da3d1b2d87170058a67d — feat(engine): stage attributed private beliefs
- be373670e64cd4aef88b5de815dac12206a9fec0 — Fix belief staging duplicate rejection expectation
- 4cfae5c55e7fc0ab7f207be7bdc6e3953a9a82ab — Merge commit 'be373670e64cd4aef88b5de815dac12206a9fec0'
- 3bc5f87825c919647ad86c7a99d3d4ac8c7fa0ad — Publish complete web build sets atomically
- 8acca639aa94317482aa3a97f74ea1b5e9f51f6a — Merge commit '3bc5f87825c919647ad86c7a99d3d4ac8c7fa0ad'
- 1d06bf30fbe56197bc5a5cc4911094698cfcc9a4 — Bind conversation permission to expression requests
- 14d6ffd63fdc6451aa88ef67b37e34a8059fb815 — Register changed topic in expression contract fixture
- 52aaf9106c8361946b9c1ea5f96594e8c6143a16 — Merge commit '14d6ffd63fdc6451aa88ef67b37e34a8059fb815'
- 00cf2bc46f50fd03f4145f3297b2bcbfc5c5b2ee — Implement bounded native text response adapter
- b7655eca853eec2f65035462130178e4be855265 — Implement native text response provider
- df38581c6eb0973f633703e7e4a935dde857ed11 — test(ui): exercise closed component state and variant policy
- 1e3c0daa2ac95263b33b85acce1610a795018996 — Merge UI variant policy acceptance contract
- 812a48fb6de1f627cf0b40e223f31fa2b7641e6c — test(gameplay): prove committed encounter trigger deduplication
- 106d72097939c40e5e23710558074401564712fb — Verify encounter trigger deduplication across session retries
- 08a99d41ecb232229b10939e66a4d24a23889917 — test(engine): qualify compound-step source revalidation
- 2970c71b16a7d17e5a8b7f8d7eedc714d6158588 — Verify per-step source validation in compound actions
- 09dcb32ec07ab6544a46e9f4734395e68eceec5e — test(workflow): qualify worker append-only devlog authority
- 91b00b806b6cdafeddd8ce8b42e6e45d738fae95 — Verify worker append-only devlog authority
- d342b76e6ad7039a4819032c2183fade6aa7c810 — fix(workflow): keep devlog fixture scratch within project artifacts
- 1852c00eb40c3353652b24f6c024f54924c2d492 — Keep devlog fixture scratch within repository artifacts

Next block baseline: `1852c00eb40c3353652b24f6c024f54924c2d492`.

## 2026-10-10T10:38:04-04:00 to 2026-10-10T16:38:04-04:00

Range: 5af9cd8084c58cdada720726b284227fca61a779..dbbc39502ab0efabc9a7b468e01272f3b536a70b

- 9f251a1d6f0f86a258d5a64f2934674f8fdfd8c1 — Add qualified provider route registry
- 32ecfcf45f38837fbc302fcca69fc0e78860ebe6 — Map documented provider HTTP request and response fields
- 4dbea134a100bd472c5b6a52805e7b7c7af75c7a — feat(df-providers): add supplier reconciliation boundary
- 06f05df2b7795d42faf82505bf54fd8c709f3cb5 — fix(df-providers): bind reconciliation to attempts
- 1b588c5557dc37bd539d1cbc07243a752c5f8e3e — feat(audio): own capture sessions by lease
- 7b5bb88d0691fed5fbcaa9aa45f6c5b55bd0c00c — fix(audio): bound capture buffer allocations
- e5baf2a3764d57853b5d68d0d8a91af3bb07eb38 — Merge commit '7b5bb88d0691fed5fbcaa9aa45f6c5b55bd0c00c'
- b19e85fb5b20f5ecb49b9e6bdce71ab9200f0256 — Define native composition ownership registry
- 8dc4ce8b3ceb0ab2c58f3c181de4443d3713913b — Merge commit 'b19e85fb5b20f5ecb49b9e6bdce71ab9200f0256'
- 087e2f235dae3261f4ce2a0221f0d1f0424f41e8 — Define bounded provider output fetch inspection
- 7df229bfc408623316d15ecd2f5de77f0db861c0 — Merge commit '087e2f235dae3261f4ce2a0221f0d1f0424f41e8'
- a0af961c68f4c25379971d5b7ec8fc248014e4d3 — Define df-server deployment ownership policy
- cd7119357606b5f3b585f2b44b7470b070bb87ac — Merge commit 'a0af961c68f4c25379971d5b7ec8fc248014e4d3'
- 1c032a763a378d976c6554995d41291e223bd52d — Implement df-audio capture capability policy
- 714f7261309360092297e4662b472f565aba4708 — Merge commit '1c032a763a378d976c6554995d41291e223bd52d'
- efd5a54d641233e1b0490e88f355a2f3e70bb6e4 — Define server drain and incident admission policy
- 577a0a0ef61430523e295c0ab886b6494dba780f — Merge commit 'efd5a54d641233e1b0490e88f355a2f3e70bb6e4'
- 561ca9a0c71e5073ab1937f1a3c6abf22eab33ba — feat(auth): define current-version capability renewal
- 46b1aa6f6a4489e6cb740834c46f5725383731e3 — Merge capability renewal at current access version
- c5336b172d5daccdfedaef97d2a0bc25825d39b1 — Add committed presentation moment projection
- 8034c3265a5f3c7abe512e3d60bc709b9e7759f9 — Merge presentation selection from approved canonical facts
- 2b5f0c1cade404613022d349f304abfbb94a769b — feat(player): adapt generated gameplay views
- dbbc39502ab0efabc9a7b468e01272f3b536a70b — Merge server-provided player gameplay adapter

Next block baseline: `dbbc39502ab0efabc9a7b468e01272f3b536a70b`.

## 2026-10-09T04:38:04-04:00 to 2026-10-09T10:38:04-04:00

Range: ff6f2cc4f700d869095beb048ca24e98aa2b73b6..5af9cd8084c58cdada720726b284227fca61a779

- aa659de759c8a4add95477219993759f3595cacd — Isolate browser audio presentation from native service types
- d84069594f89b1ec9336916f183c10cc573060d2 — Expose existing PCM ownership assertions through Rust fixture controls
- f9ee8ba4d870302302f7fe756b679a8609aa97f8 — Merge commit 'd84069594f89b1ec9336916f183c10cc573060d2'
- c3cfa675438ef97c1e589e4ce528c92020730e2d — Verify browser callback progress and bounded socket overflow
- d0463f52345f6bb34652d9e3da1fd9ca557d7ee1 — Implement bounded Rust browser socket callback qualification
- ea68b75982761ebc06305aafcd78a6e54f2f9ad5 — test(provider-api): refuse bounded work before recording reads
- 3972ec7e5ab8e7840c41946024dc7fb592d31e08 — Verify provider admission before prepared recording reads
- 39393744a5516f93a62d069df48bd6cd763a3627 — test(provider-api): distinguish buffered liability from invoice forecast
- 643c1d154294be6d6a7b7ed50e993fcd0710ac08 — Merge provider maximum-liability budget qualification
- 001ba79f6be558897e94fa9dece64cda1a27ec6a — Bind provider recording identities to locale
- 981db6f34e65a0b4a9b3a22b206fa065b79b9b11 — Merge locale-bound provider recording identity
- 1296ff32d21574e2e567a3e605d1014bd7350546 — Qualify bounded provider stream finalization consumer
- 5af9cd8084c58cdada720726b284227fca61a779 — Merge bounded provider stream admission qualification

Next block baseline: `5af9cd8084c58cdada720726b284227fca61a779`.

## 2026-10-08T22:38:04-04:00 to 2026-10-09T04:38:04-04:00

Range: 9079d004d6bda65b058fbf52d9b09c89798440de..ff6f2cc4f700d869095beb048ca24e98aa2b73b6

- 448e6c63f9588d285e6de4215dbbae57c5980934 — Implement versioned allowance renewal and downgrade policy
- 9824a458a9e65085705456115446bd68040e219b — Represent accepted pending and rejected transitions explicitly
- 637f93d7416dcb069ef9172d1ac1261dbdcc1a45 — Merge commit '9824a458a9e65085705456115446bd68040e219b'
- d54da4aa9d911cca184df70c0fffea6b86141a70 — Implement deferred credit and refund principal policy
- fb92789ac8f6167dc121776e3532669bc628cc9d — Merge commit 'd54da4aa9d911cca184df70c0fffea6b86141a70'
- 5afe1f70cfc4b771202e2cf1eb9a5b1b5e88834c — Qualify the complete observability boundary with current Rust fixtures
- 48c9544e48376298fbf7c0260701ae250d1f0a96 — Merge commit '5afe1f70cfc4b771202e2cf1eb9a5b1b5e88834c'
- 2bd0d316589c7d11b06b7eea0c416e17ad798464 — Define classified provider retry and unknown billing outcomes
- 90dd65602a0cbafca05ea4a779643bd50d12575f — Merge commit '2bd0d316589c7d11b06b7eea0c416e17ad798464'
- 7752b827a08db820d817c37dd772390e9a1d73ac — Define bounded heuristic forecast contract
- 6c9c08cdf652411b6a07ff5d38762f55df1751ab — Merge commit '7752b827a08db820d817c37dd772390e9a1d73ac'
- 0614d98643b1b63679de50809a67d6a6b35b3b6e — Qualify actual server startup configuration and owned cleanup
- 81c4e630008c29861f37c7f3e75ce33aa645fa0a — Merge commit '0614d98643b1b63679de50809a67d6a6b35b3b6e'
- d8c6b90860c27335d474922165e478d844a5f61e — test(assets): prove unavailable and stale consumer refusal
- 0dd51ec76f5f6975cdb9a2a213bee54483857452 — Merge commit 'd8c6b90860c27335d474922165e478d844a5f61e'
- cd41f8737bbc3c44c4587e7b3eab01526f05002b — Implement bounded guest account linking and recovery
- 4b8122ebed5ae5e9168648d83f2cc1b8b0cc8c0a — Integrate verified guest account recovery
- 12ddd4a44b7bd878cec6d15d0997196f37001998 — Cancel connection-owned uploads on browser close
- d12ee8a2f5a290a3bcc6a53d9104303719a5e42e — Exercise retained uploads on owner drop and peer termination
- ff6f2cc4f700d869095beb048ca24e98aa2b73b6 — Integrate owned RPC cancellation and close signaling

Next block baseline: `ff6f2cc4f700d869095beb048ca24e98aa2b73b6`.

## 2026-10-08T16:38:04-04:00 to 2026-10-08T22:38:04-04:00

Range: 93691ba198734ab834dc4758d85fb9e4123b6aef..9079d004d6bda65b058fbf52d9b09c89798440de

- 7d8b0adbd04e0d302b56dc32852e7f350ba7740e — Define audience-safe tempo boundary contract example
- e4b771486891f40b09e95b3721cfcd1607e79172 — Correct inclusive deadband expectation in tempo contract example
- a62b4f7f19c37de23aa877cf2c4e086b31bf27c6 — Verify audience-safe tempo profiles
- 06d62a50c872be044a3b0846965ad394e5b0686d — test(rpc-bridge): verify aggregate receive credit lifecycle
- 591235064ac3696c717fd7a082f6a8b322b95422 — Verify HTTP2 connection credit follows released bytes
- 692bbae299ce26f18a1b57086527bd91d02ab8ef — Exercise RPC byte shapes through generated browser fixture
- 74eb2aea94c0edbaa9b6641b7d8b9c7cb514749a — test(commerce): prove cancellation and private access boundaries
- 4c75163bbbb934892856bea3c836f97d1da2a61d — Merge commit '74eb2aea94c0edbaa9b6641b7d8b9c7cb514749a'
- fae3c0beb8760de4c51b50db2334cc0d1c370de7 — test: enforce prepared and replay execution modes
- 227d8167d3c819ff688ad25ee6619b0d728b286f — test: derive provider execution mode witness from consumers
- d6830aaec7ec629890de9070654daaa7896ff289 — Merge commit '227d8167d3c819ff688ad25ee6619b0d728b286f'
- 995c6d8a0fcebc08736524509c4e01f69b50e1bc — Exercise RPC close and reconnect outcome boundaries
- 42368c524324c133ac5e12287a308641097a1dbe — Prove pending tunnel IO closure with consumed native golden
- 8a83d1e9a87da79e16176e8614f187c3becd768b — Merge commit '42368c524324c133ac5e12287a308641097a1dbe'
- e7647bb57e51e57ab04dbd47c64b1c1ab6f93c23 — Define session effect dispatch claim port and fault contract
- 91476b32a52a9f5716b07d5da16b6a67cb6ec9c6 — Define commerce-backed BudgetStore consumer port
- 85f284d04ac7399aa4f42eec27b4caf6bb2b4ea7 — Merge commit '91476b32a52a9f5716b07d5da16b6a67cb6ec9c6'
- f3662d3f03645b030990bdfe2e9850715a770123 — Define private customer export contract boundary
- 26b7d50ecaac56c5950730737af3be792aa17f22 — Merge commit 'f3662d3f03645b030990bdfe2e9850715a770123'
- 359397d5b391c32873eb6874eb8eac1d242283af — feat(provider-api): add checked modality request envelope
- fa83804e640a13584f698a54393e7f43ea7a5831 — Add bounded predictive media planning over existing scheduler
- 6056201356bb1eec02401190482b9eb1e550c5e5 — Preflight all predictive dependency bounds before traversal
- 6f2a7d9b19b7e154fbc415913b96936b1d78cae7 — Merge reviewed media predictive planner and dependency preflight repair
- ec165aff9c3470f2caebe0bc8dc7db1fcafefcb7 — Define G02 qualification verdict and resource stop
- 49fc8a5516c2d2749f835a253f56395b882dfe91 — Preserve typed G02 terminal verdicts
- 95eb6e55852dd092e187350739d87875a64d0978 — Merge commit '26b7d50ecaac56c5950730737af3be792aa17f22' into g02-verdict-resource-stop-d04-a1
- 4fb61b80d1531ef26d3f331cae487f3044543079 — Preserve first measured G02 failure through terminal cleanup
- a4e06b39238bbe8d4139117acfcf84613338df65 — Merge reviewed monotonic transport qualification verdict and terminal evidence
- 9079d004d6bda65b058fbf52d9b09c89798440de — Qualify bidi response during mixed browser pressure

Next block baseline: `9079d004d6bda65b058fbf52d9b09c89798440de`.

## 2026-10-08T10:38:04-04:00 to 2026-10-08T16:38:04-04:00

Range: 23e50c18f9a98e1432dff6c7d0baf03019c1b723..93691ba198734ab834dc4758d85fb9e4123b6aef

- 3725e297f875ee8b46ec7e4d63a9a3f88ed00988 — Qualify retained artifact ownership and evidence boundaries
- ccbc82d604d88ff4f0d4d8429a2324f226485566 — Merge retained-artifact ownership qualification
- f98cda22601b11d35df28153f30fac89e19672f5 — Test persistent source-pinned narrative history and recovery
- a574710a8399a7d84c0fabbb5982ce6893d89b0a — Fix current codec headers in owned PostgreSQL fixtures
- 17af0847be5fd7579bdb355ab9ca2bdf3bd689b5 — Correct poisoned persistence fixture refusal
- a676a198d184306e0ba60d3cdb8345f1883ff923 — Merge qualified narrative history persistence
- caa7ff1d36a4a2d60623ced8fb4a7d212677966a — Qualify Testkit source-linked corpus identity with current owner gates
- 946fef30c17947016460a1859d10cc4a8b66c841 — Merge qualified Testkit corpus identity contract
- 4881e416d9cdd933559a154eb9541902ca4b06df — Qualify encounter consequence basis without optional media
- 2ecf80a9c514b930ec52c2ea9b38108a0fc7a38f — Merge qualified encounter reward basis contract
- 38662433f6d4d9a83425cc1c8a3a16d7a8876099 — test(interaction): freeze novice ruling design contract
- cac0cb1b0c458b7b01e16876e772626514c0143e — Merge qualified novice ruling contract
- dd1127598ed57929a7445c7601b7838930b079b5 — test(knowledge): freeze summary and index generation lifetimes
- 1c6ee9ae2f42ce146d3b94b06f0836abdcae7dbf — Merge qualified summary index lifetime contract
- 943d7847564f92012df77e68d241c373dae47457 — test(rpc): qualify retained receive allocation and credit
- ae16fb3a667d96f2e2d9dc56e7a9600de3f16769 — Merge receive allocation and passive credit qualification
- 39f7c9a90318b64206adc2b1620793dafb7b45f3 — testkit: add faithful scripted provider fixture
- f78dba8cef6fd42468702ee877d1102759a6e7b3 — Merge faithful bounded provider fake contracts
- edde31bbf5cecdc08fd1c9a45089aaa950370906 — Define bookend committed and speculative evidence boundary
- 6ccd6ce556c8c20ceb51b7f23755ea55673dd349 — Merge observer-safe recap and trailer evidence selection
- 10d5b2cd8cf7a2ee371b2b476223e0d31cdc90d8 — Reject nonregular durable asset backing on resolution and publication
- 6c796ed8751f6cca4f4ec5cf611d86986c76d5e6 — Merge durable asset resolution hardening
- 0caefd7a855b181d4703cec0c263952dda7f0bb3 — Define protected recovery overlay design contract
- 7c278c0c191009c4cbd7577a78f75d5e034e9890 — Merge bounded irreversible recovery design contract
- 95b6f9f0e52ba2dd4fe73d092f4589680eca2ce4 — fix(model): bind accepted draw facts to their operation
- e86cc0569df2b7ed2e04044d0f37fc8c9299c3c1 — Merge accepted draw operation provenance validation
- e0d61441f58ef9f11dc919ccde2b8562f6d7a22c — Qualify browser RPC owner lifetime in fixture
- 93691ba198734ab834dc4758d85fb9e4123b6aef — Verify browser RPC ownership lifetimes

Next block baseline: `93691ba198734ab834dc4758d85fb9e4123b6aef`.

## 2026-10-08T04:38:04-04:00 to 2026-10-08T10:38:04-04:00

Range: c3d7643ac1c75d0d44d70d656ab66413197ae292..23e50c18f9a98e1432dff6c7d0baf03019c1b723

- e4a0818b30e228f43441e50116e0a2ee9eb4a7ba — Add independent output evidence fixture format
- 997dc02051a77a8218f37cfa6e13f4d3399fba48 — Merge commit 'e4a0818b30e228f43441e50116e0a2ee9eb4a7ba'
- 784cc927ef5a4839957def3b6887f7df4c459a9c — Define bounded tempo fatigue recovery and rotation policy
- ece48170e4b0008ca3b9a8487e1b2ee864c22779 — Merge commit '784cc927ef5a4839957def3b6887f7df4c459a9c'
- 6af2bc84c772ce73808398fce0a80e81c3264c15 — Add bounded knowledge retrieval input fixtures
- f42c18e70503a37635102f2b470b151984cd6b82 — Merge commit '6af2bc84c772ce73808398fce0a80e81c3264c15'
- ac32d405082e9849f2bb767854c679c2fbc6e571 — Add source-bound grounded conversation claim contract
- 08e41fa0bb1ca9098ada92e4fac62605260a9f83 — test(intent): qualify text questions and jokes without resource spend
- 3daf8a6daf48c4b34cbbfe1d3842ee0b50e96c06 — Merge F08 D01 text question contract
- 0813b272a679e73e3767bf40c65672ef4f295609 — Move operator scope decision into private Rust contract
- 4f4fa8cd40e26ea86a947ece1dbbd6976c95ac94 — Merge Tools D01 operator scope contract
- 9c877fd7ab7471b541c5b474c284fe0c720583e1 — Add partial progress admission contract tests
- 852175db01c2778ef51e4334a0e7c50cdbb70c7c — Merge Intent D04 partial progress contract
- bc481779ec41490baa37f88288272435e014718d — Define committed continuity and critical eligibility contract example
- d0769d1680f67cfe60517759650056c8a1f437aa — Merge G12 committed eligibility contract
- d34c169204ca59d61835999c8ff164a257073b76 — Specify current offer expiry and revision binding
- 3d0604735889b83a700c1cdc3845e58511ff2806 — Match canonical unknown-member offer enumeration refusal
- 0ec13534a9d60a8309c9794ff555e8f57c6e2dd0 — Merge F07 offer expiry contract
- 157e9623beb7b7c0d4fd7bccd8c9cea87cd7eacc — Specify controlled roll boundary through canonical draw guards
- e66c1dc2228c6152fbc88ad6d1cda6229285172f — Count source binding in controlled roll fixture dependency budget
- 23e50c18f9a98e1432dff6c7d0baf03019c1b723 — Merge F10 controlled draw qualification

Next block baseline: `23e50c18f9a98e1432dff6c7d0baf03019c1b723`.

## 2026-10-07T22:38:04-04:00 to 2026-10-08T04:38:04-04:00

Range: 363c1745eb4dfe4e2bb6a60688dc398d19666e3f..c3d7643ac1c75d0d44d70d656ab66413197ae292

- 0d3e90157a083ca60f3738f183a252f8f372eb7b — Define exact cache replay identity and current retrieval authority
- 542d2901a57a9c486154e2b0051470f57116eafe — Verify cache withdrawal authority with intact public wire receipts
- 964adbf6d3a95234d736c400d7ac71b9d2ce9a6d — Merge commit '542d2901a57a9c486154e2b0051470f57116eafe'
- 3c1102e95f7c4bb90f35c06307a0e3305a39eda6 — Implement separate operator and consented export permissions
- 48f68156daacb0fec89a425e576411fb1f0c65fa — Merge commit '3c1102e95f7c4bb90f35c06307a0e3305a39eda6'
- 81af75740cc863a5c99e0ca1600361972a0d784b — Define listener-safe speech projection and expression requests
- 3bef91a610805fd46275be9e4581b227704a7a00 — Merge commit '81af75740cc863a5c99e0ca1600361972a0d784b'
- c6e9a2bcab16bba609dddc09babb6298d7a38bad — Define client reconnect identity and freshness contract
- 8eafe326e86c845498856be44696ce9117a31082 — test(tempo): qualify logical presentation and wall clock separation
- d2d28ec8fe046aed5cfea3840eb897bc586f33d7 — Integrate tempo clock separation contract
- 170dd5771b475f90a212eefcdf3aa42114cbee5f — Admit explicit current conversation topic and disclosure scope
- 0d281d8a0dbb7a72a50f93f4c8b5254712a420f7 — Integrate explicit conversation permissions
- 79f78928851c4881bf782559ac397cb7858f037b — Add source-bound media continuity and prepared tier selection
- 5f35224b1d0c3c0cb41b874b88a3fa52677a5961 — Integrate versioned scene item and NPC continuity
- aa86d548c492903e048507c759af6b6cf6568678 — feat(tempo): stage bounded impulse curves and decaying inertia
- eeedf3f9226d00a9326c26df8653cee9387e24ee — Integrate bounded tempo impulse and inertia limits
- 2488581456a9d5d86b55e50d1730f1999fcb2494 — Add bounded uploaded content candidate validation
- 6f0c8c526355123677d0e4b6f1bc44c49894affb — Implement bounded content import validation
- 831e7d240371b67ecb696e750e32f5d482e829ad — Add source-grounded encounter escalation and reuse selection
- 0af49b226c7bfbb804570a474c77508083b89a27 — Merge encounter escalation subsystem
- b645d1c20ee527f55efeca58755b21c3125c574f — Gate critical celebrations on admitted committed outcomes
- 27f7b4fddb5e3e4b6f2c10f686e114e80491b9ad — Merge committed critical celebration admission
- cd948d0a36786289e5fe7bcfedcc6d1f23ea7614 — Define safe dialogue dispositions and explicit native confirmation
- 67c185e7732094aa92d8c5eede1e67f7122a8c83 — Prevent dialogue requests from dispatching pending jobs
- cf94ffd9ada6609bf808feaca2d4eabd6ce19dbc — Fix mounted dialogue fixture database scope
- 6914cbdfc9ad138164dfb147a724a71873900577 — Fix dialogue mounted controls for current journey offers
- c3d7643ac1c75d0d44d70d656ab66413197ae292 — Merge mounted dialogue confirmation qualification

Next block baseline: `c3d7643ac1c75d0d44d70d656ab66413197ae292`.

## 2026-10-07T16:38:04-04:00 to 2026-10-07T22:38:04-04:00

Range: dd62a0330ba21796c2b33e461acb61bee5b246ba..363c1745eb4dfe4e2bb6a60688dc398d19666e3f

- ffdd8b63f4c819a9311f2054386c6c133f089fc4 — Define source-bound battlefield objective and escalation policy
- 95836ca46743348f86035f0a9566e5fc778d94f0 — Define source-bound encounter admission and failure exits
- 570600858551de5defbc421f0e27a083ea4850e2 — Define bounded Intent candidate and step execution contract
- f228d16192ee8e9fdf003dee9ca873199eddebbd — Merge bounded intent step schema contract
- f7321cd4b65f342c9b7ec5715bb0df178d9cc9ec — Implement canonical affordance lookup and target admission
- 1e2c0f248c42da8f711fb07d2d2b4b106ceb086f — Merge commit 'f7321cd4b65f342c9b7ec5715bb0df178d9cc9ec'
- 8eb8fb12df3e8a63b4494fb079457e8f9d2680cb — Define Combat cancellation and basis revalidation contract
- 9505972339cc4bf90bc9e040d56f6599566df9c7 — Merge commit '8eb8fb12df3e8a63b4494fb079457e8f9d2680cb'
- 16bb6d47670920ad6186c064324baeaa6c076adf — C-df-types-BOUNDARY: freeze candidate contracts and consumer fixtures
- 363c1745eb4dfe4e2bb6a60688dc398d19666e3f — Merge commit '16bb6d47670920ad6186c064324baeaa6c076adf'

Next block baseline: `363c1745eb4dfe4e2bb6a60688dc398d19666e3f`.

## 2026-10-07T10:38:04-04:00 to 2026-10-07T16:38:04-04:00

Range: c0c0425a6fd28c4bc411569d636bdcdaa5804c93..dd62a0330ba21796c2b33e461acb61bee5b246ba

- 9b08bc073b65feb85a4c696653d3c8e13161d6a5 — test(knowledge): preserve canonical truth across provenance recovery
- a1a721cc9a5305f03b7acdeebbac639b176a90f2 — Expose rules explanations from selected handler provenance
- d1848c0c9c75b1e1d27b12467a5dcb22cfab29b2 — Define explicit claim ownership and witness disclosure contract
- 2a097c3cbf3fb29a56110b568bde773d319626c5 — Validate narrative convergence and persist intervention budgets
- dd62a0330ba21796c2b33e461acb61bee5b246ba — Merge commit '2a097c3cbf3fb29a56110b568bde773d319626c5'

Next block baseline: `dd62a0330ba21796c2b33e461acb61bee5b246ba`.

## 2026-10-07T04:38:04-04:00 to 2026-10-07T10:38:04-04:00

Range: d6648ba329750c0826b369b36879962cc7d9d128..c0c0425a6fd28c4bc411569d636bdcdaa5804c93

- f65d963b7f217be6d3f1c77d0487dd93a1f4fd1a — Define source-pinned replay inputs and canonical checkpoint hashes
- 12d96e97625e2d9517f7da0fe3dc0d42d3f14d7f — Define bounded source-addressed encounter challenge contracts
- 5ded937597c95f3efbeab56c8039acfe6c085f33 — Merge commit '12d96e97625e2d9517f7da0fe3dc0d42d3f14d7f'
- e4d5a66c19ec3612596a7deb5348e9fe82996875 — Persist NPC identity, relationship axes and obligation lifecycle
- c0c0425a6fd28c4bc411569d636bdcdaa5804c93 — test(narrative): verify alternative route and refusal contracts

Next block baseline: `c0c0425a6fd28c4bc411569d636bdcdaa5804c93`.

## 2026-10-06T22:38:04-04:00 to 2026-10-07T04:38:04-04:00

Range: b1ebc9e71e1897fd7fda14fdbfbecb527e3345c9..d6648ba329750c0826b369b36879962cc7d9d128

- 894563102e8e616acd47e43fbc6bba21b7619dc9 — Verify session logical-time and wake contract
- 9dda173b50432b2bf2448ca1fd91e24a2f77e751 — Merge verified session timer clock generation contract
- d6648ba329750c0826b369b36879962cc7d9d128 — test(engine): enforce dev-only test control authority boundaries

Next block baseline: `d6648ba329750c0826b369b36879962cc7d9d128`.

## 2026-10-06T16:38:04-04:00 to 2026-10-06T22:38:04-04:00

Range: 2d2cbae3b11383e726e46187e504638196403a57..b1ebc9e71e1897fd7fda14fdbfbecb527e3345c9

- 8ab9d6aa66af8537edbce20ce31d22e42f31fa88 — Define perception limits for tactical ranking
- c3396e81d031d0633c514d27b36fb730cdd0f87a — Define rules-constrained tactical intent contract
- cad5f8af5becf6a0a44b99e1665f7e0e89cc8b64 — Test planned RPC inventory and generated descriptor ownership
- 6b44997a7dc70843bca299bc10e005889b7ad048 — Test generated catalog claims against compiled dispatch policy
- 8d18fb25575cd305c3ba4f32e0e20ec858013e75 — Verify paused-world clock admission and schedule replay
- b1ebc9e71e1897fd7fda14fdbfbecb527e3345c9 — Verify typed rules resolution envelope and staged invocation

Next block baseline: `b1ebc9e71e1897fd7fda14fdbfbecb527e3345c9`.

## 2026-10-06T10:38:04-04:00 to 2026-10-06T16:38:04-04:00

Range: 6566ef1e2e295a695dcf61a797a988659099d141..2d2cbae3b11383e726e46187e504638196403a57

- 63ee4132a1abccfc944d5a3ea6579dac04dd3959 — Bind registered movement handlers to their fixed source
- b0418e86ae0b2104f1ddcee3bb9de71464ccee00 — Refresh recovery owner lease after grant revalidation
- 05484eff8f754934c13ee0895878273bb2c51cfb — Require canonical accepted reaction fact ordinal
- aef09f9fb3bd7b4854c20c65c8d686bf235d46fc — Integrate movement and recovery reliability with retained checkpoints
- 884f361b20aac49661c1ca560ef330d812d5feba — fix(session): retain authorized operation lookups during uncertainty
- 2d2cbae3b11383e726e46187e504638196403a57 — test(model): preserve canonical facts across observer updates

Next block baseline: `2d2cbae3b11383e726e46187e504638196403a57`.

## 2026-10-06T04:38:04-04:00 to 2026-10-06T10:38:04-04:00

Range: c038b2cb1328deb77ce7cdd1611cf9d1d0e0e4de..6566ef1e2e295a695dcf61a797a988659099d141

- b66d25ea5eb980de996fd753389b2d7559d64d80 — Connect encounter objectives and fence restored gameplay state
- 89b39b790891a6e20d4a543c803de5550f4648cf — Connect campaign scene image resource ownership
- 49ba8fab6ba9c041830f0b29e41241ed81891397 — Preserve operation facts and pending intent identity
- be5c73f6bb21a64eab643f06243bf2614b0630a0 — Connect native story and combat selection
- aa19994ae9765def66aed1c14c3c15d394b4247a — Connect public scene rendering and continuous gameplay
- 0a595132b59a82dec1325bd2c3f49f29147c8442 — Reject changing asset bytes during publication retry
- b74a68a7ef2cf09e70d968fbe0dbdcf35413444d — Prevent queued inputs from starving due session work
- 3e4484af3adcd5a88516396f5b51098015612656 — Preserve admitted character-reference mode across default changes
- 7593e90c39b3011bf1530ddf93523740a9be684f — Merge verified asset, session, and reference-job reliability fixes
- 6566ef1e2e295a695dcf61a797a988659099d141 — Preserve retained bookend mode across configured defaults

Next block baseline: `6566ef1e2e295a695dcf61a797a988659099d141`.

## 2026-10-05T22:38:04-04:00 to 2026-10-06T04:38:04-04:00

Range: c165da58ff8d656e820436c123151e03e7872e26..c038b2cb1328deb77ce7cdd1611cf9d1d0e0e4de

- f29eec9720f48943579e262b7ad2cddc95b1509b — Add audio and scene rendering with narrative and client recovery
- ec67c03385fbd3059423bf3a9cb1e151a2f82bad — Connect engine staging and browser media lifetimes with commerce validation
- 77fbefe1d07ec01a995e25a8e26908eade794c73 — Repair causal staging, media lifetimes and public asset delivery
- c038b2cb1328deb77ce7cdd1611cf9d1d0e0e4de — Guard durable projections, causal intent bindings and media replacement

Next block baseline: `c038b2cb1328deb77ce7cdd1611cf9d1d0e0e4de`.

## 2026-10-05T16:38:04-04:00 to 2026-10-05T22:38:04-04:00

Range: 1301baefbae2f75751eab875064c5894d00ec79f..c165da58ff8d656e820436c123151e03e7872e26

- 2505763463e74b15405786e1c21492d5b309aa0f — Wire native gameplay and harden subsystem lifetimes
- 3717cc044896694997ff64672735582a1128a38e — Connect courier reactions and harden transport and rules
- c084eb47b4a4d474a80026ec405802804ef05093 — Add tempo elapsed staging and harden gameplay and asset boundaries
- 0922abae3f25e2c90fb4948a1a7302ab9fe36ed4 — Add spotlight and pacing selection with tactical and locale consumers
- c165da58ff8d656e820436c123151e03e7872e26 — Add fatigue staging and committed presentation with grant recovery

Next block baseline: `c165da58ff8d656e820436c123151e03e7872e26`.

## 2026-10-05T10:38:04-04:00 to 2026-10-05T16:38:04-04:00

Range: 5f9b9fbdb46f56c5139c9df53cea6148bc9c3424..1301baefbae2f75751eab875064c5894d00ec79f

- 078bafc905acf0454b6d01ca1dfcf8bf97f5d827 — Recover narrative completion while players are idle
- 5d0eb744caf4ba9a4ae0d154d9c8d0c797c67929 — Validate encounter participants before initiative
- 302fc910151670be5969a6136b30f1c15347853e — Inspect registered courier effects before persistence
- 7a068a90edbf60c30dec3231a0127c3c0ddd0bce — Stage obligation fulfillment from committed canonical causes
- 43ad22429f36a976081eb6e189fdefa824d66b88 — Filter canonical facts through current observer scope
- 051c64419f2cac81164e3a918e08ae09e533d1cc — Route offered semantic responses through engine admission
- 1301baefbae2f75751eab875064c5894d00ec79f — Add eight validated engine subsystem boundaries

Next block baseline: `1301baefbae2f75751eab875064c5894d00ec79f`.

## 2026-10-05T04:38:04-04:00 to 2026-10-05T10:38:04-04:00

Range: 7d6e79c7a99005b6a4806e5b0e4a8062de813c60..5f9b9fbdb46f56c5139c9df53cea6148bc9c3424

- 5f9b9fbdb46f56c5139c9df53cea6148bc9c3424 — Keep combat rounds independent of rest and travel time

Next block baseline: `5f9b9fbdb46f56c5139c9df53cea6148bc9c3424`.

## 2026-10-04T22:38:04-04:00 to 2026-10-05T04:38:04-04:00

Range: 546c4af886faca68765953343220eec9dc72a813..7d6e79c7a99005b6a4806e5b0e4a8062de813c60

- fbc4cdd8a9fdac1a5c571177925ad7a1ab383f5f — Restore saved gameplay across native server restarts
- bc4d315eba5a20a87469b2a7b0316eb3e3734821 — Connect source-qualified short rests to durable gameplay
- dd8be6153ea8480e2a22b6ab8361345780bfa8d2 — Execute durable prepared courier narrative jobs
- 7d6e79c7a99005b6a4806e5b0e4a8062de813c60 — Continue victorious parties into the durable Harbor Inn scene

Next block baseline: `7d6e79c7a99005b6a4806e5b0e4a8062de813c60`.

## 2026-10-04T16:38:04-04:00 to 2026-10-04T22:38:04-04:00

Range: dfa5451fb80d07ae77e30dc047d6213549f68e9d..546c4af886faca68765953343220eec9dc72a813

- 304ea7c02af56e031ac86502742dfa52a468e3be — Handle missing and stale asset delivery outcomes
- 5de82abe8007058e8530e0a4bda325f5e9d61db9 — Add complete two-player gameplay journey and demo
- c914302feef64861aaf8a7eae79f06e7e2797255 — Wire narrative progression into accepted gameplay decisions
- 546c4af886faca68765953343220eec9dc72a813 — Recover durable gameplay after lost database acknowledgements

Next block baseline: `546c4af886faca68765953343220eec9dc72a813`.

## 2026-10-04T10:38:04-04:00 to 2026-10-04T16:38:04-04:00

Range: 723b29a60de1e8e0426ba41e6e42264202c46d54..dfa5451fb80d07ae77e30dc047d6213549f68e9d

- 57f92eae83a9ee60f85e3efa62eb6bd5927ec396 — Integrate API admission, client connections and exact asset resolution
- 13a5152b42ecc2dc6fa6714eaa0077c141068e06 — Test actor fencing after an unknown PostgreSQL commit
- 15b2595a310d741e2c07ef10e1a5b41f807fc645 — Merge verified PostgreSQL actor uncertainty tests
- d9591a232e0462f2f225833312e0d464d8ab82f3 — Integrate character appearance and verified scene images
- 1622d598ccdce15529337c25685860fa145f40fb — Capture cosmetic character appearance and public summaries
- d7a64f1270270dc8102eeda7e53ad636a6604f7a — Synchronize synthetic character confirmation across both client projections
- ee6432a9cc4508726caa921409846f30f5fb4f28 — Mount verified scene images in both Rust browser clients
- 4c2ac625341f084436a09a195a2375d08892c93a — Reconcile verified character and scene-image branch histories
- d5f97e7b8cc069712d886f47cc46b3fe338bb459 — Add an interactive harbor-to-combat gameplay preview
- c1393d9103dd9370bbc936e58c0020ad49947537 — Improve exploration framing and handle missing character artwork
- 478abc2820275ee104a198328c8fda1bf56cd991 — Keep narration before phone scene choices
- d84d1ff700b4bab6e836e65e95eca09747de78eb — Extend scene preview through aftermath, campfire and journey
- dfa5451fb80d07ae77e30dc047d6213549f68e9d — Add real harbor investigation and e2e gameplay recording

Next block baseline: `dfa5451fb80d07ae77e30dc047d6213549f68e9d`.

## 2026-10-04T04:38:04-04:00 to 2026-10-04T10:38:04-04:00

Range: af19148dee48df78da1c2acddd77b100ba93458b..723b29a60de1e8e0426ba41e6e42264202c46d54

- 6a6b18ecde8ed76bd3ab2158e0d1846189b8cbcc — Add bounded verified client asset cache and revocable leases
- 00c4611e25d31eefb871d14243f5dab46dcb2b29 — Connect durable native session submission to registered engine boundary
- 058f2000fe96f736d6be80a49f852b46d82498cc — Integrate qualified native session submission with verified client assets
- a32b35e8d7e719dad91cbb0d7474ca48b29ced1c — Add reviewed Mara four-view character reference sheet
- 4b28994b1c9d07080c34771e33e9c439b1041112 — Add reviewed Mara harbor scene using saved character reference
- 78d2cd6d585ca2d95cc9cb1da89c66e9c229d97d — Add reviewed Mara portrait derived from saved reference sheet
- 374ff1e3a312764f785bfb2addbc972df95d072d — Integrate native PostgreSQL session persistence adapter
- 99b961d2f60464e2e4e47b095c759f8e6d36c02e — Fix Join fixture input lookup against the mounted controls
- c21f6cf065d9475afd9c2518df40a391ed8fceb6 — Redesign combat clients around cinematic scenes and compact actions
- 9468a69b496d39142a37828878504634d3f7e224 — Polish both character creation clients with cinematic campfire themes
- 9cc9f1ea1baef4aa7617e19040b1b70728c6a484 — Restore scoped reconnect focus and wrap narrow Join narration
- 18bd83fe5cd3ca488a53e64ff061692c4f9a340a — Add versioned generated lantern-keeper portrait to existing character cards
- fc2b623ef096dc0b0e5b6ae8583517c8c82cb7b6 — Polish character sheet campfire presentation
- 3c4f6ea7be82762ed9c35606c284add6173ac7dd — Document pinned Rust toolchain targets
- 723b29a60de1e8e0426ba41e6e42264202c46d54 — Reconcile superseded UI and toolchain branch histories

Next block baseline: `723b29a60de1e8e0426ba41e6e42264202c46d54`.

## 2026-10-03T22:38:04-04:00 to 2026-10-04T04:38:04-04:00

Range: b65587e6c446eb8f5f9a1b2a2501cd0de939581c..af19148dee48df78da1c2acddd77b100ba93458b

- da1811981435cc7ac173b1417b52dbffe8be529c — Redesign combat clients around cinematic scenes and compact actions
- ff4df77ccc4a0c0ea30909f849e70b06591430a1 — Exercise persistent role shell inputs, pending states and ownership replacement
- 124a60180e1083c078e0777ba66415f6d81f5331 — Polish both character creation clients with cinematic campfire themes
- 28e889ae91f59ccc50bfecc53d4153b3bb03119c — Polish Exploration clients with warmer scene-led presentation
- 76c2015cdee1667c1ce6a1396269111bd4147f4b — Refine Exploration scene hierarchy and phone action layout
- e8633c6ce034cc6421a757b6328541323bb9da2d — Improve Exploration accessibility and narrative composition
- 8e940e66a340822da53c8db0b84d345886936d0f — Combine polished client phases, portraits and reconnect focus repairs
- e309c9c0dded2d9b613f370ff39b13a006509a33 — Checkpoint staged gameplay rules, combat and engine pipeline
- a1e420d3b8e53b2e151a98005753574f82026bbd — Stage reusable character references after accepted creation
- d7900d137a988bc692d8418223abf6e5ff984bc8 — Integrate gameplay engine and reusable character references with client UI
- 693248336f397264d25bede3b432936b5e7c5baf — Fix client status contrast, narrow Join wrapping and narrator fallback
- 323637055e3184f943d2537aa0bda3580ec61985 — Merge verified client polish with gameplay and character references
- ca31e72bf956d42bc5b9b8f125e395a19fe47de5 — Preserve character reference reuse after mechanical choices change
- af19148dee48df78da1c2acddd77b100ba93458b — Merge tested character reference continuity repair

Next block baseline: `af19148dee48df78da1c2acddd77b100ba93458b`.

## 2026-10-03T16:38:04-04:00 to 2026-10-03T22:38:04-04:00

Range: 1ff6e69e582a494980575bd161ae3be5dbee101b..b65587e6c446eb8f5f9a1b2a2501cd0de939581c

- 6571ff79854043c1413a2a7c99c9e01374c6332a — Add interactive campaign UI demo video
- 3a92bb234866008043936221fff0bc8041b2a65f — Add DungeonFlux phase UI asset kit
- 3dda8846a51ba4d32478d2a5767104bda9d352b9 — Checkpoint interactive UI phases and dual-client character creation
- 927a53ac505fe0e6f4fd643bbebce74f51d02cab — Add persistent character creation mounts for both browser clients
- c1f5bb632ad5efd5fd5c8a73185bda933964a9a3 — Mount player sheet navigation and exploration for both clients
- 8e0715b8b7098b7b2a809945caaa0e3ff4642e47 — Fix browser-target UI fixture and draft cleanup checks
- f03e965a9757eea62dbc1bf82f5e1b33a960dd98 — Mount combat and session overlays in both browser clients
- 875b6a5a99b521614a88553c7363b645df6f2223 — Mount persistent join and lobby UI in both browser clients
- aabb6c2cdc2dca493de57e9ddea94b108e263631 — Fit combat and overlay panels and handle missing scene art
- b65587e6c446eb8f5f9a1b2a2501cd0de939581c — Connect player and display UI phases in a persistent Rust shell

Next block baseline: `b65587e6c446eb8f5f9a1b2a2501cd0de939581c`.

## 2026-10-03T10:38:04-04:00 to 2026-10-03T16:38:04-04:00

Range: 6ba2d9cc5bf4bb4eababe4028e95a9cf76d731fc..1ff6e69e582a494980575bd161ae3be5dbee101b

- 1ff6e69e582a494980575bd161ae3be5dbee101b — Import ShellHacks concept art and build cinematic campaign UI

Next block baseline: `1ff6e69e582a494980575bd161ae3be5dbee101b`.

## 2026-10-02T22:38:04-04:00 to 2026-10-03T04:38:04-04:00

Range: eaf35af4273cc714c3c01d3a939af18237458113..6ba2d9cc5bf4bb4eababe4028e95a9cf76d731fc

- 101521e643884e8be935bd4759622deafdeaa870 — Implement shared browser token and layout primitives
- c9f046c39d59dd62844994da25bb9b89f52355dd — Keep native UI action buttons in keyboard tab order
- 420ede825ccc04d4cb30c694999c523a54181eb4 — Integrate bounded Rust source B-C-df-ui-I01
- 9778b8e552b0104379d7af7e23cb796b5e20ba1f — Implement source-pinned rules handler dispatch
- b9d82c32487898766c6a4bed13646a99779bffdc — Integrate bounded Rust source B-C-df-rules-I01
- ad20ca9fa8e01b8a428c933c2fdeb2b0dd6d3e35 — Add accessible controlled input and action widgets
- 6ba2d9cc5bf4bb4eababe4028e95a9cf76d731fc — Integrate reviewed Protocol WASM bindings and upload status propagation

Next block baseline: `6ba2d9cc5bf4bb4eababe4028e95a9cf76d731fc`.

## 2026-10-02T16:38:04-04:00 to 2026-10-02T22:38:04-04:00

Range: 45aac097ea2317d93b2d8f3eeeb2a9eff2babd9c..eaf35af4273cc714c3c01d3a939af18237458113

- abf20ca6bcc51994c32ed5a250956bd816832447 — Implement bounded complete recording admission
- be96dcc4ca0d6da456fe8a75bb7f2257e5e184d2 — Integrate bounded Rust source B-C-df-ai-I03
- da73000c4a68b7d9727efef129f4b77fe4158fe9 — Implement prepared and replay artifact lookup
- 279fac6603a404e0b1db38ce3a2a899f42532899 — Integrate bounded Rust source B-C-df-ai-I04
- fa1b6b0672487970fef8456452e45b4abac3c931 — Implement bounded encounter participant admissibility
- 3cf793bb5e3a702ce187dd0c04dce52d6c61d036 — Integrate bounded Rust source B-C-df-encounter-I03
- 82f02aa8fb350fba6bcfe31ea79dcba7c7e09a2a — Implement bounded guest bootstrap invocation policy
- e30f0a7602d486f72ca1b23e3efa3bb8e54cacb7 — Integrate bounded Rust source B-C-df-auth-I01
- d9e21837dfb06be52ffb0b5dcb34355c18d53a6d — Implement current membership scope and role validation
- 0bc409141459df2bee20e8245181c3e981cca6be — Integrate bounded Rust source B-C-df-auth-I02
- e02ffd24cd1738cf0b0b29c005253f9b0447f427 — Fence credential rotation and publication against current authority
- 9c5c7dfea68ebfcdc3bee0da87402b5fd0a58aaa — Integrate bounded Rust source B-C-df-auth-I04
- a998a1127f6c173e5471afcb355e7262af5e1158 — Implement bounded content graph validation
- 4b5e385e78bf230a549c0024d4a446723c02ac5c — Integrate bounded Rust source B-C-df-content-I02
- 7e960c8d9cd0423d94ca05803d979d8833d1764f — Implement bounded immutable content catalog snapshots
- 443121d420ebc3efbef93a8d1b9dd3245bebab53 — Integrate bounded Rust source B-C-df-content-I04
- af651d285e6e029d284b158d9a0b4985d72536e7 — Implement atomic locale catalog version loading
- 963c59bb746e9fbbbd59bb62223342a65ca8e808 — Integrate bounded Rust source B-C-df-locale-I04
- 44e76ed87c5a75ee65f7b993dbf2b20ab30747b2 — Implement explicit voluntary refusal cooldown
- 0379d60e61cbe29a1e404dbd63fc8e84a5ecb497 — Preserve bounded refusal storage when staging cloned candidates
- 8c8a4b241bef29816f4d8de9934f94a7c99e7ca9 — Integrate bounded Rust source B-C-df-experience-I03
- dfb7dca14b57d5bd468adc554cf23a736c1c21c2 — Implement independent virtual logical and presentation clocks
- 8826822c133ef99bcb712b8e30e440091d168309 — Integrate bounded Rust source B-C-df-testkit-I01
- b3a698e3a49cb8fb57e6ddd518c2d0f1ff111e61 — Implement native tonic incoming admission and generated loopback proof
- 6bbac4fa5e54b8a788613864a16f9b8d83ba1410 — Integrate bounded Rust source B-C-df-rpc-bridge-I03
- 280fcf267abfe9f09a956714bcb2e974f39ff4a3 — Implement canonical immutable model checkpoints
- fbc921539e998bb210b807fd5a14e4eab96d0aab — Validate canonical checkpoint record ownership and request bounds
- 6891ca5c61c9c98204f08452397f9c8381e324c1 — Reject duplicate canonical graph and record identities
- 018a70a6728b99b266037b02777a80d1cc1430bf — Integrate bounded Rust source B-C-df-model-I01
- 86aceddfdcf429807fc9c92a870fcde6c81437ff — Capture dispatch context and retain asynchronous child span links
- a8dcdd34c619868f6539eb84c4cd58c615a02847 — Integrate bounded Rust source B-C-df-observe-I02
- 22330be553973fc55d97ae5aed769f240d2e1ad8 — Test frozen generated native RPC bindings
- d5650e403c72eeaef65cfada509959cf27eb7748 — Integrate bounded Rust source B-C-df-protocol-I01
- 8f067383eafffbcebfa94ef7c9d59aee6063780c — Implement bounded safe browser diagnostic capture and upload lifetime
- 53c0daef70cd321a966f2ef620e51745cebfc604 — Integrate bounded Rust source B-C-df-observe-I03
- c9cb6544481e668f92e328a32bf8df667a46e702 — Implement verified immutable asset byte publication
- b29e00e6a499c2a4b6ee8465df2ba9aec2ff3402 — Preserve owned asset promotion temporary on collision
- 0f3e151e14e9f44d958c60992893a812ca2066fb — Integrate bounded Rust source B-C-df-assets-I01
- 30f7ad957d82edc2bea46a41b49671973f25cf17 — Implement bounded session actor inbox and reducer loop
- c88758991b8d6db4e098dfd1385055b5db220f21 — Integrate bounded Rust source B-C-df-session-I01
- ba42c8aa24fbd9fb00bd94a655cb3f623ec7b799 — Implement bounded canonical world due-event selection
- 99f482ac4126c4f1c2acd06e1af9cb9992727b2e — Integrate bounded Rust source B-C-df-world-I01
- db7e8bec9b17a0ad63cf6c54ea9f57a767785f40 — Implement closed model command payload validation
- 0d6352dad9ad352876160ca4e977b530d76b8298 — Integrate bounded Rust source B-C-df-model-I02
- d0eff45274fa9e28d731508ab675804fe9db4c1f — feat(testkit): preserve supplied dice and semantic replay records
- 2dd4b36fd56fc58fd97fb4606d0ab0fb318295f7 — Integrate bounded Rust source B-C-df-testkit-I02
- 91fe53c1ba6880a4ea0be1aefece67831c2d72fd — Stage schedule destinations through accepted logical time
- ce7766028ff984297c511fe2594767cbd27a81a7 — Integrate bounded Rust source B-C-df-world-I02
- f8f4f414e9e119dc227c53ddaca38905ae78dc0c — Implement bounded current-authorized asset range reads
- eaf35af4273cc714c3c01d3a939af18237458113 — Integrate bounded Rust source B-C-df-assets-I02

Next block baseline: `eaf35af4273cc714c3c01d3a939af18237458113`.

## 2026-10-02T10:38:04-04:00 to 2026-10-02T16:38:04-04:00

Range: e01d7ba0ae36d34d03b4ddaf2a0647e552bc4c82..45aac097ea2317d93b2d8f3eeeb2a9eff2babd9c

- 41176ab40995c13e2d990b140870764f2d1b8490 — Implement typed safe catalog placeholder formatting
- 1cd914d1bd68165d6e05fccb48b1fa8ba460cb60 — Keep typed Locale format data separate from markup and commands
- fb09789ed1bfc16540e8426f8539a17e4d336904 — Add shared crate composition for parallel Rust implementation
- b2ed690a6ab9a38e561087d072c9acd6e4306751 — test(rpc-bridge): verify native tunnel partial byte ordering
- 354614e32f4861d38e1618f8f503932b13562eb5 — Exercise bounded RPC native partial byte-order and lifecycle regressions
- 1baacd03a6b769f1563db66c800c4723f7749030 — Implement monotonic client complete-view admission
- edc9cb668f3dac7a8fdebcc12cb6cc3cb0824eed — Integrate bounded Rust source B-C-df-client-I02
- bee0d2eb7f1c6197134516948a8e58ce8bab728b — Implement uncertainty-safe operation receipt lookup
- 83805a77fb647993f99d84e918d381f860750546 — Integrate bounded Rust source B-C-df-client-I03
- 130eda00148e56327b6589be3163260a57924a90 — Protect speech capacity with bounded ranked media scheduling
- 6bf319870e3e1ca0d15c3f667c6e319d875c27db — Integrate bounded Rust source B-C-df-media-I01
- 02a6c37048b0c5551763a831db55986407bf7d6d — Implement bounded media cancellation and stale completion fences
- c46cfef536c7a27f89a7dd18979bfb5173799e65 — Integrate bounded Rust source B-C-df-media-I02
- 2c01757cfea5ad49676dca6159dc176074aa3d20 — Implement attributed belief update validation
- 7919b3c5526a6196acd9565a3613868b67260b75 — Integrate bounded Rust source B-C-df-knowledge-I01
- c92611ba5419497d8c02c78a1eaad047825aeccf — Implement bounded owner-authorized knowledge candidate ranking
- 299eb8ae9006cfe99fed46532a0486a889a9ce31 — Integrate bounded Rust source B-C-df-knowledge-I02
- a47a5d0e07a71b61e1d062e00094cce26e6b2788 — Implement independent relationship axis proposals
- ee487b8c10b749b707a524949f0fd711a4e2097f — Integrate bounded Rust source B-C-df-interaction-I02
- 370b2043735994953dde9a6b70f1b35ad2dea70f — Implement provenance-preserving obligation lifecycle proposals
- 19693bf6d2bb627d5d0d887edaa61a0f755e99e5 — Integrate bounded Rust source B-C-df-interaction-I03
- 9362e2444115859f0ba7f5036a92f65b8f73b7cc — Implement bounded accepted activity window aggregation
- d00c7f1ea6be6eef4ecb143ab0b36c5424a7e298 — Integrate bounded Rust source B-C-df-experience-I01
- 7d568f476bced6d84cf31a89a3eb6a9cfba30bd9 — test(telemetry): distinguish exporter failures from caller results
- 9cc8473156e87626c6114647933b7e7d610d55ac — Integrate bounded Rust source B-C-df-observe-I04
- caac22da13cfa98ea1a2f0961b1ec74df058c040 — test(telemetry): verify disk receipts and crash recovery
- 2017dcb8793e7c097895e69f3a9d1622676041be — Integrate bounded Rust source B-C-df-telemetry-I02
- 65b9ca7e5b35d4b097b21bf0dc30ab36a0411a61 — test(telemetry): qualify quota refusal and accepted-work recovery
- 45aac097ea2317d93b2d8f3eeeb2a9eff2babd9c — Integrate bounded Rust source B-C-df-telemetry-F02

Next block baseline: `45aac097ea2317d93b2d8f3eeeb2a9eff2babd9c`.

## 2026-10-02T04:38:04-04:00 to 2026-10-02T10:38:04-04:00

Range: 38f6983c8f9df39610c6a67acc00bc29084e5f30..e01d7ba0ae36d34d03b4ddaf2a0647e552bc4c82

- d31d2828402ee930865f3cb4c44a234afc96954c — Record six-hour parallel integration work block
- cbc4d8eb28d74539ac670b390030caeae0280c18 — Reserve public and operator protobuf namespace boundaries
- f4da18692d0a5c49d867a429c35ce120ebfd5e6b — Integrate B-C-df-protocol-D01 bounded decision
- 34a3745e89814f82c49ca71cfbab205c9b0993a6 — Specify enum unknown values and explicit field presence
- 9c9110c283cf6f596c4a3b15ae53ad0620cb87a9 — Integrate B-C-df-protocol-D02 bounded decision
- 37c50713b1625cd9cdcfb05b78fbe0cc7f933ab6 — Document recovery revision and strict write expiry decision
- 49efeaf5b4f3aefdbf970606a608f2a6270e60f2 — Integrate B-C-df-protocol-D03 bounded decision
- ad49697b8f3668199ab372ddfd18f4ecbdf4dc67 — docs(df-web): define persistent route disposal contract
- 0bb297f340f081d31bad74b298a839d7e20dc018 — Integrate B-C-df-web-D03 bounded decision
- 992bfd01195f2dda28cac0c5e1edf40ea0f2d768 — Define bounded effect and trigger composition decision
- 1319e091270dafeedc3cb0712ad74ec4ce0f2ff2 — Integrate B-C-df-rules-D02 bounded decision
- 0848b954dda04462c25524f94d67064dc6d572a3 — Define server geometry and legal-choice query authority
- 60df2ca35c93d97195e3583f82054d8a81156745 — Integrate B-C-df-rules-D03 bounded decision
- 8f0f3a83099ef2ec44684e2ff6a080f45c675a2e — Define deterministic bounded world catch-up decision
- 73264ca84a78ae6d8cb7b7bb59f204236a5202de — Integrate B-C-df-world-D02 bounded decision
- d3c9e03ebfc79055893502b976a87b6a11e83434 — Define session operation and job identity retention decision
- 7cac2ce112c88f960566567b103b290b92f5497e — Integrate B-C-df-session-D02 bounded decision
- bebf221c240a76a9d7c42a55e0f7064047f397fe — Implement explicit locale negotiation over admitted tags
- 3d37d4eddb2fe7e40905ee1da9b3b2cdd78a84a2 — Implement exact locale selection and disclosed fallback
- 73547435b5404c59bf5a755f7c8029c7807cea8f — Validate scoped span correlation and byte measurements
- e143b0bd5ec81bc7f31cf1a443265bcfda746070 — Attach admitted operation and build context to Observe native spans
- d83f83a5136cc29fc4c96a38359fbe059136d600 — test(protocol): exercise generated compatibility decoding boundaries
- 4dd3ba42a5a7f6eab7db26c145b1066584451226 — Test safe forward reading at generated Protocol compatibility boundary
- 70b34ef28d1bbc6d6e27f902e46f60cce349ca1c — Prepare bounded telemetry ingress accounting and boundary fixtures
- 93a17518974f8b2bc45509bf6a60b05b57fb6cd7 — Format verified telemetry ingress accounting and boundary fixtures
- 7e5e80cee10769f905fb218f13fa3cd78492c7d4 — Enforce Telemetry ingress admission and owned queue capacity
- 3790bb1871c3c3e167b10292493a11a43fa3e824 — Add checked locale catalog lookup
- 7ad3449df8a0ad9f323c38752e56b2c31a16a67f — Format locale catalog pointer assertion
- e01d7ba0ae36d34d03b4ddaf2a0647e552bc4c82 — Retain visible TextKey diagnostic identity in checked Locale catalogs

Next block baseline: `e01d7ba0ae36d34d03b4ddaf2a0647e552bc4c82`.

## 2026-10-01–2026-10-02 continued work — 22:38–04:38 EDT

Six-hour boundaries: 2026-10-01T22:38:04-04:00 through 2026-10-02T04:38:04-04:00.

Range: e8bbe087706d8f382ab92a96d11d4ee7be5cafa2..38f6983c8f9df39610c6a67acc00bc29084e5f30

- 8cb823eff46f81d0c3da37feedbbc9ce34ac3dc6 — Close continued six-hour integration block
- 4b38ab2fa35a8bad68ef4e9c09e5ec2696bfc5ca — Freeze exact money arithmetic and I03 consumer contract
- 6a3050df9c83a8582954ade5c56a19213ef1fbb9 — Integrate reviewed MONEY-I03-CONTRACT-001 contract
- 750e24ef5d6a7cc386f9650a628c626c48c73b4d — docs: specify remote mixed-room authority topology
- 0941a6bc507143a55c587b99f6b7dcc08b235ec6 — docs: repair remote topology retry and audio contract
- 1a535202640f382c29d800a8a0aca79f3f36af81 — Integrate reviewed B-F45-D01 contract
- 70f08ec656776b36c002fc43823d1b84f2f6dc75 — Document commerce transition policy boundary
- bd9a80c288198ca489132d44d1f9ba5edf846cf2 — Integrate reviewed B-C-df-commerce-D01 contract
- 7bb741f31e6dbe1019e311c35ada22b8ede028f1 — docs: define canonical commerce authority decision
- 0605dbbe214f114a7f99f104d9cd5c2aa31521ad — Integrate reviewed B-C-df-commerce-D02 contract
- 47715a3ac13d2bbed8b4a54bb7b64ece315546e0 — docs: define df-ai semantic and flavor contract
- 3c3ecd68cd1af8d390b12de8f4e0e098f564047c — Integrate reviewed B-C-df-ai-D01 contract
- ec9c05f83074e62a1f2bea2c85f88b092aced2ea — docs: define listener-safe expression context
- 479845c599ac12e62d089d85acd88dbd7f3324b3 — docs: format expression context example
- 924af3ec6cbfd08f62259720299e6d335af986ef — Integrate reviewed B-C-df-ai-D02 contract
- 93cfc93d9201999390fda59b55c2ed1d63ffb877 — docs: define immutable asset publication contract
- 7acdc275cadd818680b19a5b67b14cf7cc148770 — Clarify immutable asset version refusal ordering
- 989232793db3c0be35092837e998b442a6c75275 — Honor hosted object media selection for assets
- 0bbcc1fc73e59d35e320795ccef7422c3879a106 — Integrate reviewed B-C-df-assets-D01 contract
- 62f95adc39afeba2d903e71354bdd7eb67fa3d90 — docs: specify AFK and resume admission policy
- 778436d1522c7404e7dfb8f812d2c44c3acc9c99 — docs: refuse exhausted resume generation
- 8b6b3cdfe13f39dff2ce738e6179697aaefefe66 — Integrate reviewed B-F45-D02 contract
- d8380082fd8129b67e470e4c877ee8f199e7c41e — docs: specify F46 recap fact selection
- 9fa8243bea585c32ab93e94ec26497b9712c6c15 — Integrate reviewed B-F46-D01 contract
- 545801e7ff152d8637cc7c668cf3b74c263e0a20 — docs: specify speculative trailer label contract
- e24157ca71ca8bbca1ed4011114c8faa526e0a7c — Integrate reviewed B-F46-D02 contract
- 5630eeb3c79ceaed3ce2ac9d0832462b2426455d — docs: define AI claim outcome slot qualification
- 2718f594595319dff7798b0522775c71895d8cf7 — Integrate reviewed B-C-df-ai-D03 contract
- 2c05f2571902a1d23605587afee64e089d4a53d3 — docs: define authorized df-api projection policy
- 748508e4893fc4bead8567f5c6686c6233d0407e — Integrate reviewed B-C-df-api-D01 contract
- 2badff8198f16322670084f58ee7b46484b28945 — docs: define public and operator API service sets
- 2b447fcb1df0b57d8d56032bde2ca40010830ade — Integrate reviewed B-C-df-api-D03 contract
- 0280118c5735b191d449bf607401b0176cd3ea01 — Implement exact money and conservative liability arithmetic
- f5f10c56543ea2ffbabbcbdce6d4dc00733113da — Implement exact tagged money and usage arithmetic
- bdade349f7ac97c7684e501a3cdddec05c586512 — docs(commerce): define hierarchical liability contract
- d100afb0badfa2da22bc8a19db53c263cd4c9585 — docs(commerce): bind spend hierarchy to currency
- 0e3bf37d363945be1fa5d843fa93ab18b5186fb6 — docs(commerce): cover currency-safe settlement probes
- d440691d6b947ecb5402bf0c8e1ff8b3c6a1406e — Integrate currency-safe hierarchical liability design
- 2ec637ef9a32f6e84e93d9d9d2218ad97809a6e3 — docs: define df-assets open authorization policy
- efea6bc97e578fa77d990a0c68f1161da4680383 — Integrate reviewed B-C-df-assets-D02 design
- 1491d5015cb2ee94cac489c8c35eecc7f9d1a54b — Define durable asset backing and cache boundary
- 9bebaa91fcdbef501ba3c62ab9bafb5af8f66a19 — Integrate reviewed B-C-df-assets-D03 design
- 8222e4030c6a336c76ab79ca8a4d68449cb788b5 — Define immutable ContentPack publication identity
- 68b07b7d3e83d9c319463805f5f3f7fceaf370c4 — Integrate reviewed B-C-df-content-D01 design
- d4a5b987fc00106fcc60f046e68df7a9fba38025 — docs: define df-session owner fencing decision
- 96c2e1b6994db88d40b139f4aec16a7ea9887eab — Integrate reviewed B-C-df-session-D01 design
- 3d07c2cc825c5f5a4acd8195aa58716bef810cf5 — docs: define staged director composition contract
- d6bf7dc1e0230d3f2cf65c88059ad08ced2d02ef — docs: preserve required engine outcomes on optional decline
- 575df9c3a619e8ee68eecd6bbf3d4c5ebdd00c35 — Integrate engine staged composition optional-capacity decision
- 44744aafeb13301578a10b52106c933b46de9252 — Add native origin CSRF recovery policy
- 9d7ebd7de32c36c3e140723d08a67ad0a09f3cf4 — Integrate B-X01-D02 reviewed security decision
- d0f78ab158e792d403f556b22d57c452978dd995 — docs: define X01 media decoder trust bounds
- 1a107c0ea2c688a975435d26939e05b1cc6257cc — Integrate B-X01-D03 reviewed security decision
- d92d1ca5d64e066a4380ecbad199b89f6ad3cc0b — Implement bounded RFC 5646 locale preference parsing
- a3c31a00e7406898b0a82617fa7626dc0a4e4c94 — Implement validated RFC5646 locale tags and actual consumer tests
- 105aec9813b26c92dfa3a0ca5302816444e54e56 — docs(observe): define operation context contract
- 88a5326aedff436a6baa4a4e58d0bab501e3787e — Integrate observable operation context decision
- 2be2e6ec9f45eece840b81cf0f4f04951ce97906 — docs: define owned df-tools preview lifecycle
- ba8bb287e1e3cfcdabc0bc5682b54d5d789084dd — docs: define owned df-tools preview lifecycle
- 2022c90fa8b4f53b99eaec43d5e32f3e3b7205cc — docs: define owned df-tools preview lifecycle
- 4745fdd11b8e8a2a7b787b1fc5f2746911701125 — Repair df-tools preview owner release contract literal
- f38d70a01b7cb8903ae58c211a6090ad90463def — Integrate held preview ownership decision repair
- b1beda21538c4e83bdf72ac93eed383010974fb6 — docs: define telemetry segment retention policy
- c8b4217e2790e11123e0231a9cd604f97a9755b5 — Integrate native telemetry segment ownership decision
- ca13cbf8cb36a6436f43da8904cbc650a9d87f16 — Define pure diagnostic instrumentation handoff
- 87cb4d114b12cfda9d3b27dd21d54e8c872fc22f — Integrate pure diagnostic handoff ownership decision
- e72b18a55ae18bde118e0120bec194fac912d51e — Define telemetry spool receipt semantics
- c8e0c62a7636a24c8b354131176e5376a2cd381e — Integrate B-C-df-telemetry-D02 diagnostic decision
- 92c94f4348e8d047a2cf446d10ec0119e87f877c — docs(observe): define diagnostic field catalog
- 0ffe02cf8bddfd83306b849e113c4eb0225da95b — Integrate B-C-df-observe-D02 diagnostic decision
- f8bf04fd8ee68ce319bd50409362f01a7d27f959 — Freeze telemetry SQLite mapping and query scope
- 4c0e050c2d30e632bd3b32c51a92cfda52479e6e — Integrate B-C-df-telemetry-D01 diagnostic decision
- c17a8fa27cf1108568dc0f9072a44daf381a196d — docs: define locale preference precedence
- bd92c3bd8484bf19abfa677046c9adfa3c9d6e31 — docs: fix locale example ownership
- 4eda2a7c00926bd15c730362f8dc36671b899726 — docs: exercise LocaleTag spelling accessor
- 2029cdff17f54ad8c870fc0737614125f2612b3c — Integrate B-C-df-locale-D02 bounded decision
- 42a929b3a2eef5e39bff984c100db08721d46c35 — docs(locale): define typed placeholder contract
- d17fb14284d902eb405a26b8f444cb4cdad54f30 — Integrate B-C-df-locale-D03 bounded decision
- 9d32ecbb8150d214d8dd97b2007d57da70b98fa3 — docs: define df-web Rust WASM boot contract
- 1effd3872ccac7c7da8184370266bfd5dd6bc764 — style: format df-web boot contract example
- c4dc4b7f08e5f274936456e022bf9fbd06ba37b1 — Integrate B-C-df-web-D01 bounded decision
- 4e64ad555a35068ebe9475071592944b885e10e0 — docs(engine): prepare pending continuation ownership decision
- f6a7d05517a19a4c2ee0c136742ed7c58ef9a213 — docs(engine): verify pending continuation boundary illustration
- a761d1dadd53c73b1fc32309244075ffb9c352dc — docs(engine): retain diagnosed formatter binding evidence
- 61829e8b43cb6338f9188efbb406b68b52662b7c — Integrate B-C-df-engine-D02 bounded decision
- 5b84413ae6023636eceadf7c3401180f5194b756 — Implement held preview registration in native fixture
- c59b5e9a3fcc0416e8b0baf1abde82fd74e08646 — Format owned preview lifecycle with pinned repository style
- d870db00dc349425e8f977bdfb20c6ffb24bfd3e — Integrate B-C-df-tools-I02 bounded decision
- e9d50fe8a388d8b127c81853c79ef6446e3253a1 — Define principal and campaign authority boundary
- 822a11af4105c5072e7d23ece5d21a5c65a9ada4 — Integrate B-C-df-auth-D01 bounded decision
- 9aaca00485233bb4114c51c3ba2d271a08102a3a — Define bootstrap credential and binding lifecycle
- f3e486a0b701e696f5fbd406a661727b4d1b44bd — Correct credential trace proof example
- 0750952025ca041eea2af0ba0c12c24bf338409a — Integrate B-C-df-auth-D02 bounded decision
- ede965b74d16fc9fd958c0db3f67108d268ffc3c — Define df-auth current entitlement reader boundary
- 491c9333493415533323abd8f2b79dd34caefbe0 — Integrate B-C-df-auth-D03 bounded decision
- 7a7cfd64e953d3dd89ee270c8ae8ef7d884e46a9 — Define immutable checkpoint envelope and exact pin refusals
- 076e5d13b12fc460a41ca97a998254bb91c83a26 — Integrate B-C-df-model-D01 bounded decision
- 47daeb632f25c65db8af63087d7b29d02488632d — Define closed model input and effect ownership decision
- 7d04a55c4fe62324adbaa6f1b02ca1d23a2dde54 — Integrate B-C-df-model-D02 bounded decision
- 4d33fd8f6e74a22c06fdf6413791cd231083737a — Define distinct pending resolution continuations and timing windows
- 1a172510fa7d7e27e2a3cd782e46c2b704a76dd0 — Integrate B-C-df-model-D03 bounded decision
- fb23e4a88dab6bff71edaed54853d1f8e0478e05 — Define consumer-owned PostgreSQL repository mappings
- d4785c08adce53bc81880a3b0a989cf7b8b3dbe4 — Integrate B-C-df-persistence-D01 bounded decision
- 8ded06467ba2fcce38082ca34b9b63895f735001 — Define durable decision transaction fence and unique intents
- 75a1a28ec0313184645e6538c2e6e47693be9b04 — Integrate B-C-df-persistence-D02 bounded decision
- 7cf0751f1d899cb73eb85c9f777d51df58c4dc5a — Define verified tenant RLS transaction scope and pool reset policy
- 41359242170a1f065ae67417e60df1ed4cdd0cc5 — Integrate B-C-df-persistence-D03 bounded decision
- 516ead38776d28f860cbde67c67085c04c437607 — Define exhaustive effect registration and terminal routing
- a901ea360786542496139d47bcd8081f57dd2caf — Integrate B-C-df-engine-D03 bounded decision
- e50e2e74e2583f6ed81517ce0eb0046671d2b371 — docs(content): define campaign graph validation
- 21363efa35cc8f39cad0fb08c8f6a439a082699e — docs(content): fix campaign fixture slice lifetimes
- 47c4e1f5fc15d12797006baea2e336e4fb2e4094 — Integrate B-C-df-content-D02 bounded decision
- a48788fe9729823d5a607fb91838f4950a713011 — Document asset revocation and replay suppression
- 118c87b006d0991057cc49141f6ad2ee3e3005aa — Integrate B-C-df-assets-D04 bounded decision
- 571f8aaa605825837597aa2b5c5d3bb91a409f10 — docs(locale): define stable catalog text keys
- 577099b8cf3c56f978f10c44954bb6aebe812cfb — docs(locale): prove stable catalog key behavior
- 5985c118233bd1008da01a44ca2d8faa2181ffaa — Integrate B-C-df-locale-D01 bounded decision
- 9d85e311b75c2dfdd6b72da9f617123011a76c2d — Define telemetry private-data lifecycle
- ebec071bea0f8f39a253ec6a35f079afb3edf3fe — Format telemetry lifecycle contract literal
- 08898e03b6cd66d81bfc94435d8c13eed9001ffd — Integrate B-C-df-telemetry-D04 bounded decision
- 67df32eff1a0650bf5a64e5ec4f86a1425158bcd — docs(observe): decide bounded browser export lifetime
- 85363e83e205eea86e3b86cceec584334ff4f139 — Repair browser export lease identity and bounded retained storage
- a055862bbe64fe8d310bfc92b2c31aa9570dac5a — Bind browser upload leases to their concrete buffer owner
- fe772c5cdfb98cc60eb914631b46c2edfa6a0a28 — Integrate B-C-df-observe-D04 bounded decision
- dd2ad30026f809b986b7fa16ced9b6daa547c766 — docs: define df-tools artifact provenance
- 7091c788cde1dace252fbf4e7f57a96d2add9cd9 — docs: format df-tools provenance contract
- c8237df681ed138fb83f29dde6750edbec44b613 — docs: repair artifact digest and rename boundaries
- 38f6983c8f9df39610c6a67acc00bc29084e5f30 — Integrate B-C-df-tools-D03 bounded decision

Includes integrated shared money/locale primitives, commerce policy, native observation ownership and preview lifecycle source, and independently reviewed bounded subsystem decisions. Preview lifecycle passed native/WASM gates, 30 native tests and 30 real owned process cases at d870db00. The final sixteen decision merges are awaiting independent acceptance at the fixed whole resulting revision; they are not yet marked DONE. Full game, standard-2024 coverage, production publication, database and browser feature qualification remain incomplete.

Next block baseline: `38f6983c8f9df39610c6a67acc00bc29084e5f30`. All branch commits and root bookkeeping in this range are retained in topological integration order; this closure commit is captured in the next block.

## 2026-10-01 continued work — 16:38–22:38 EDT

Six-hour boundaries: 2026-10-01T16:38:04-04:00 through 2026-10-01T22:38:04-04:00.

Range: 9def9b845531e5cc589a28343b822b4664bd03fd..e8bbe087706d8f382ab92a96d11d4ee7be5cafa2

- 827d947a8f1596eb9c7cb753d6a622b244184fb1 — docs: define private authoring upload boundary
- 89e56102552f1f0388479d3db219b4246a835aa0 — Integrate independently reviewed private authoring boundary
- fd55969483c6cad4b013c7b1d466f11fa1375e2e — docs: define G10 pack provenance validation contract
- e76009dc8a16e9ebe0e67e7c9757601c1b9f58e4 — docs: correct G10 pack identity and ownership policy
- aa52c1a83491da1f5d067607598e3098b2b2e314 — Integrate reviewed define graph and provenance validator
- ff5999e7658f71e2d680163f5be79716bb9158d8 — Define replay compatibility and logical-time policy
- 1458808da382db09d9614dd284fb7eb6adbb30da — Integrate reviewed define replay compatibility and pause policy
- 2b4aa9f7e5b1ee74c77a915fadee09207b76a14e — docs: define listener-safe claim envelope
- 23f1da942e113989fc948ab17f86b9fbf8a3d74f — Integrate reviewed define listener-safe claim envelope
- f85580a600a4292cbea879b68a4ed880ce5f73b2 — docs: define bounded native memory retrieval contract
- 72bbad19cfaa5f5690eef27b86cafef2ede63c4a — Integrate reviewed define bounded memory candidates
- 487535c54ee696bd96513a94d5e4d4212346d215 — docs: define consented observation and spotlight policy
- 0cb72148993754f5c28f60da705e99c5b8a19483 — docs: complete g12 d01 policy alternatives and evidence
- 9c2f04176538c8d5169b9047260dffdbc3c3ea49 — Integrate reviewed define opt-in observation and spotlight scope
- 376c5f7990164d80ce72557a673bf07851316e37 — Document heuristic predictive demand policy
- 23806e6165af305e87c9eedeaa30eb0f4868a56d — Require compatible recordings for replay demand
- 7178f630ee75b0b641a8032158e75634c6572348 — Integrate reviewed define heuristic predictive demand
- 301a37f850d79d80efbd4fd16fad6fda867cea9b — docs: define vendor qualification policy
- cd6f2cba6fdb1eda43f17628119e7f58088a8d22 — Integrate reviewed define future vendor qualification protocol
- 2a1adc3c8e65f393046c911cf572ca5f7d6d573b — docs: define G09 runner model routing policy
- cf0a15905359dbab3934bd18ab73f88f5a83e9d9 — docs: enforce frontier evaluator routing gates
- be829d9c96c5cd0eee24115292721098bf18ad4d — docs: record G09 A2 policy provenance
- a89b762108a4a2cfd208ef7ef077fa270384b740 — Integrate reviewed define runner model route policy
- 1c073661249c96c4091fff1d2cb0603b1563c26d — Define independent output review contract
- 308920e328ba6df85bea1e3d2abcbea5f1b796e6 — Integrate reviewed define independent output review contract
- a0575c15cdf5171c1a90d4aa36a385317747660b — docs: define G11 director ordering policy
- 8cdfce816581d043afad6083eed4514b1e15075d — docs: correct G11 composition ownership
- 11a7872cf6534aca0f1ca0ccd41bf6d91871e4f2 — Integrate reviewed B-G11-D01 decision
- 43a5f87c4235873f53a9cae60ab9d1b31df7c0ca — docs: define tempo accessibility policy
- 7d1ce0c133cc6367063e65f926611eb16f36691f — docs: format tempo contract example
- 5cc348c3ca44bf68f8e8ddd6520d13d679295a66 — docs: resolve tempo policy review defects
- df1f907c8ef01f5bffce901dd693f3a29dd7b438 — docs: clarify illustrative timing basis
- a3ce54a7d3eeba31e86be9fda5de540d650d1551 — Integrate reviewed B-G12-D02 decision
- f076b430e4f603cf1213747cec4e0c983a11e7cb — docs: decide prepared initial provider mode
- a215cc323244a681d6a98630a87ed9631ec0d261 — Integrate reviewed B-G08-D01 decision
- 050036f3267706c2c320209f0a4e55584da1df63 — docs: define G08 modality quote and fallback contract
- 4d565263046eed58830f2f231057393ea11ec1ac — Integrate reviewed B-G08-D02 decision
- f8612a1819ab016e5821597ecd795c8cf0c5c2ac — docs: define PostgreSQL durable receipt boundary
- 537efdd541e77d5445ae52eefe21530fd69074ba — Integrate reviewed B-F24-D01 decision
- 97366425af66098b6aba52096ac68a936f798357 — Record continued integration blocks and open design wave
- 6b029bb4ba587d13be5682f4913b96c086d729b2 — docs: decide initial payer identity recovery boundary
- e2007daba2d4eaa5a14da5cdd8ba8646217d7b85 — docs: harden payer recovery contract boundaries
- fa1316b2784608a1d8997969b36ca11a65971b3f — Integrate reviewed B-G04-D01 decision
- 92be5c33f0e94b27c8237ebbbb9085eef79ddbba — docs: decide initial device browser role matrix
- 4f80eb43ee75feffc198cc76b620e47cb2eab8ca — docs: correct G04 source availability status
- fd54807128380386738f67f83e53038ba6a7ac1a — Integrate reviewed B-G04-D02 decision
- b1981988681a3cc64f87c8c25303568878932abc — docs: decide initial G05 storage topology
- 5328816ae69a083cc4c119ef6fa643b35e508ac1 — docs: align G05 media root with storage decision
- 5d208d7ca953a8dd9a9360b6fb1a11a9ed20cc3d — docs: follow hosted G05 media deployment decision
- 4b19b971c2ec022107bc6ded14b7f0fc3d930d88 — Integrate reviewed B-G05-D01 decision
- 524fd9912b6e79fdb73a3faf0ede698dc56ca175 — docs: define bounded telemetry redaction contract
- 80200e01ee1e248a273b2b2861d53b7e93144527 — docs: tighten telemetry redaction fixture
- cf0c57891a5c068fd1376116e618637a6e10d053 — docs: format bounded telemetry example
- d9c9ee09511fc5f6cfdaf066dfc8d52a354b4389 — Integrate reviewed B-G06-D02 decision
- a774a5aac8b496a78d559280faa619ded89d953a — docs: define telemetry spool readiness policy
- 98ba784a44c5c454254743c7ad035d0ad79f98ce — docs: record verified spool contract example
- 5f3974212feff62a73589868b5bda15fd16ee306 — Integrate reviewed B-G06-D03 decision
- 8dacb9c7c1df3f96b2760fa711c59d6119769190 — Specify recovery journal watermark source
- 22b44d075762628970b1692d18012c0a3f0910b8 — Integrate reviewed B-F24-D02 decision
- 2b4933fb23341bc1ff8923e0e6385eec7ba926df — docs: define tenant repository scope
- af6d1bebbd702d1252cd091927e3fa1fad6a5aee — Integrate reviewed B-X01-D01 decision
- 5c7f9ad322c704105ffdf9253a9dd6ad6c2d4acc — docs: define command receipt outcome boundary
- 84f8294721bf3032ddfc4ba712423b618f1e4304 — docs: clarify receipt lookup refusal path
- e8bbe087706d8f382ab92a96d11d4ee7be5cafa2 — Integrate reviewed B-C-df-api-D02 decision

Reviewed design decisions cover authoring, provenance, replay, listener-safe claims, bounded memory candidates, consented observation, predictive demand, vendor qualification, evaluator routing, engine composition, legal tempo, provider modes/quotes, identity/device allocation, hosted PostgreSQL/object media, telemetry schema/admission, recovery watermarks, caller isolation, and RPC outcome classification. Exact policy fixtures and resulting-source checks support DESIGN completion; production gameplay, real services and durable enforcement, audio and device qualification remain pending.

Earlier rejected drafts reachable through accepted branch ancestry are listed as historical commits, not separately approved deliverables. Every newly reachable child, merge and bookkeeping commit is included once by revision range and integration order.

Next block baseline: `e8bbe087706d8f382ab92a96d11d4ee7be5cafa2`. The bookkeeping commit writing this closed block is carried into the open 22:38–04:38 block.

## 2026-10-01 continued work — 10:38–16:38 EDT

Six-hour boundaries: 2026-10-01T10:38:04-04:00 through 2026-10-01T16:38:04-04:00.

Range: 9def9b845531e5cc589a28343b822b4664bd03fd..9def9b845531e5cc589a28343b822b4664bd03fd (no new integrated commits).

No integration is recorded in this interval. A clock gap does not establish uninterrupted development or testing; no work or screenshot evidence is backfilled.

## 2026-10-01 continued work — 04:38–10:38 EDT

Six-hour boundaries: 2026-10-01T04:38:04-04:00 through 2026-10-01T10:38:04-04:00.

Range: deb0462a59b6346803e545ff3627c8b6b030423e..9def9b845531e5cc589a28343b822b4664bd03fd

- f7c8a7577385ecb04c2e3f3305e7fafa0c851119 — Record continued six-hour integration block
- a6b66e818331306fd59a846d65c8c3e5a91c7a2f — Define public RPC error taxonomy
- 2811f9d91667668f66717b1c5443552fbc06a74b — Integrate independently reviewed public RPC error taxonomy
- c435c2d12c1faa3da51142e6d8e1ca0b1ec82397 — Add bounded browser connection credit stall observation
- 51bf1d438294311c204380077ddb01fd043c5735 — Fence late connection credit report writes
- f1c2fc1965db37d47a539ffec68835f68422e469 — State hostile over-credit bursts as pending in credit reports
- 9def9b845531e5cc589a28343b822b4664bd03fd — Integrate independently reviewed browser connection credit observation

Public RPC error classification and actual browser connection-credit stall/resumption were independently reviewed and integrated. Full transport memory/control-queue/device qualification and production gameplay remain pending.

Next block baseline: `9def9b845531e5cc589a28343b822b4664bd03fd`. All newly reachable commits are recorded in topological integration order, including child corrections, merges, and prior bookkeeping.

## 2026-09-30–2026-10-01 continued work — 22:38–04:38 EDT

Six-hour boundaries: 2026-09-30T22:38:04-04:00 through 2026-10-01T04:38:04-04:00.

Range: 2e263fdc475f886555f301c47e3628fe43abc775..deb0462a59b6346803e545ff3627c8b6b030423e

- d2ae8f2 — Record completed ignore changes and reviewed development progress
- c4814eb — docs: freeze experimental protobuf policy
- ed55434 — Integrate reviewed B-G02-D02 policy decision
- 92bea4f — docs: define protobuf field allocation policy
- ebcebcf — Integrate reviewed B-G03-D02 policy decision
- c9ac120 — Document recovery revision policy
- 84b907e — Add executable recovery revision example
- 65d0913 — Integrate reviewed B-G03-D03 policy decision
- ec1bc08 — docs: define bootstrap cookie origin policy
- d1eb4c3 — Integrate reviewed B-G04-D03 policy decision
- 2c7c044 — Document protected journal durability boundary
- 083ccb2 — Integrate reviewed B-G05-D02 policy decision
- e4576ae — docs: define erasure retention overlay
- 3559d7c — Integrate reviewed B-G05-D03 policy decision
- be95c24 — Document per-instance telemetry topology
- aac3d48 — Integrate reviewed B-G06-D01 policy decision
- acbb2cf — audit: map native shared contract readiness gates
- 2db3035 — Keep native readiness evidence local and ignored
- fa22eea — Document task memory and lease admission policy
- 7e2aa44 — Correct combined lease admission reserve provenance
- c391041 — Integrate reviewed bounded task memory lease policy
- 14681cb — docs: freeze typed recovery revision policy
- 4bdc6ba — Integrate reviewed typed revision policy
- 9b6881e — docs: decide typed identity policy
- 26f1ea0 — Integrate reviewed typed identity policy
- 0b79048 — docs: specify money usage and duration units
- eef8bf5 — Integrate reviewed exact typed units policy
- b30fef0 — docs: define typed build provenance policy
- 5ebd415 — Integrate reviewed typed provenance policy
- a2ccefe — Propose exact I01 applicability correction and execution boundary
- 50fc0a3 — Repair I01 seed with mandatory accepted contract receipt
- 502f294 — Integrate independently reviewed I01 prerequisite applicability repair
- 8549d20 — docs: select dependency license admission policy
- e36ab46 — Integrate independently reviewed dependency license route policy
- cd06fcb — Add canonical hex constructors for identity types
- e6d6921 — Integrate independently verified canonical identity text constructors
- 1f80e77 — Propose exact I02 applicability and accepted source receipts
- a6ed525 — Integrate I02 prerequisite applicability and canonical arithmetic scope
- bab2bd1 — docs: select native HTTP2 transport fixture policy
- 62a10c8 — Merge independently reviewed B-G02-D01
- 998523d — test checked revision arithmetic boundaries
- aa4ff51 — format revision boundary fixture
- 1222e82 — Merge independently reviewed B-C-df-types-I02
- fa6d2b3 — Bound screenshot journey devlog entries
- da2f836 — Validate screenshot journey ingestion inputs
- d7d3de2 — Reject incomplete screenshot journal records
- 8321f08 — Fix bounded screenshot devlog ingestion with validated provenance
- ccb452c — Enforce incoming HTTP2 control frame rate at tunnel boundary
- 69dbde9 — Coalesce consumed browser h2 credit within fixed windows
- 3c9f795 — Keep browser credit coordinator Send and recover poisoned cleanup
- 921ae66 — Exercise poisoned credit cleanup against h2
- 003578c — Preserve browser pressure traffic under rolling control-frame cap
- c9a700b — Document reproducible fixture command registry
- 18bb52e — Repair fixture registry command contract
- 3531253 — Integrate reproducible command registry decision
- c566e0b — Measure retained browser callback Vec capacity
- deb0462 — Merge independently reviewed browser callback capacity observation

Canonical identity text and revision arithmetic, dependency licensing policy, typed unit/provenance decisions, native transport fixture policy, rolling control-frame admission with consumed-credit coalescing, screenshot ingestion and reproducible command registry received independent integrated checks. Full G01/G02/G03 qualification remains pending. The callback-capacity candidate is merged at deb0462 and undergoing fresh resulting-build browser evaluation at this boundary; it does not establish total heap or pre-callback browser engine allocation. Phone and audio qualification are deferred to later testing. The rejected control-rate and ingestion drafts appear in history through their repaired descendants; their failed evidence remains retained. Cleanup performed read-only inventories and zero deletions; the cleanup024 initial ownership-registration gap remains recorded separately from its approved fresh retry.

## 2026-09-30 resumed work — 16:38–22:38 EDT

First durable attempt: 2026-09-30T16:38:04-04:00; coordinator prerequisite reads preceded it.
Closed six-hour boundary: 2026-09-30T22:38:04-04:00. Bookkeeping finalized after the boundary; its commit carries into the next block.

Range: dd63d194a8e87ce575e861199abc28861ac2c28d..2e263fdc475f886555f301c47e3628fe43abc775

- 822ffc4 — Preserve reviewed development setup, source pins and pending transport evidence
- 2743c4a — Add experimental dependency closure audit evidence
- f79bebf — Harden dependency audit input paths
- 9898c70 — Freeze first shared identity recovery and protobuf contracts
- efc5617 — Record reviewed shared contracts and native foundation evidence
- 916298e — Qualify native PostgreSQL tenant policies and context reset
- a1b377e — Add bounded native SDK telemetry spool and diagnostic store
- 59201be — Bound producer structure and validate checkpoint commit coverage
- 421f783 — Integrate reviewed native durable telemetry foundation
- 01c2ccf — Correct toolchain target evidence and commands
- 0c395a3 — Keep fixture telemetry spans instance-owned
- 4ba3794 — Integrate per-instance fixture telemetry ownership
- 8e274ab — docs: select browser build pipeline
- 55f38cb — Integrate reviewed Rust browser build pipeline decision
- 5c43f2d — docs: define shared contract wave policy
- ba71f24 — docs: define shared contract wave policy
- d678211 — docs: require namespace for breaking wire changes
- fe5588c — Integrate reviewed initial shared-contract compatibility policy
- 9e5d2e0 — Ignore all development runtime output
- 9d38535 — Ignore reviewed generated artifacts and runtime output
- 7ccd9e5 — Ignore local development evidence
- 2e263fd — Ignore generated development evidence while preserving local files

G02 atomic pressure/reload acceptance completed on the retained integrated ba0 fixture.
The fresh current reload passed; retained-tab blank recovery remains a full G02 gap.
The recovery diagnosis remains inconclusive after earlier locked-screen observations;
the current UI state has not been revalidated for that diagnosis.

The current four-crate descriptive dependency/notice audit is approved. Its first
verifier submission failed ancestor-symlink confinement; the approved repair uses
anchored no-follow input reads. This audit is historical evidence and correctly
refuses the fifth-crate Cargo inputs; full G01/runtime/legal qualification stays open.

First shared contracts are independently approved on actual merged 9898 source.
Pure IDs, provenance and composite recovery revisions, generated common protobufs,
compatibility cases and actual descriptor ledger pass native and WASM compile gates.
Fresh isolated MAIN native/WASM builds and ten consuming/schema checks pass. WASM
execution and full G03 models, effects, audiences, authorization and services remain
pending. A reviewer Cargo rerun replaced three native candidate artifacts; original
and resulting identities are disclosed. Fresh merged builds used a separate root;
all live previews stayed fixed.

Cleanup inventories 007–011 are independently approved, with zero cleanup deletions.
Cleanup 011 retained 39 measured roots and explicitly excluded two live G06 roots. Inventory 010 explicitly left two live telemetry build/tmp
roots unmeasured; its measured totals do not represent all generated output.
One bounded passive screenshot recorder continues; its fixed preview is ba0 even
though workspace source is now 421f783 with the telemetry source merge. The live
preview remains fixed and does not expose the standalone telemetry executable.

The native durable telemetry foundation submitted clean a1b377e in its isolated
worktree. Frozen native/WASM gates pass; actual SDK capture, three process-crash
barriers and the final synthetic 1,000-record/s burst with a two-second sink lock
accepted and committed all 10,000 records, draining in 3.011 seconds. Earlier
failed paced runs and lifetime uncertainty remain disclosed. Independent frontier
execution found excessive nested SDK-value encoding allocation and silent skip of
a durably spooled record after checkpoint corruption. A separate Sol a2 repair resolved both defects under all six original criteria.
Independent candidate review approved 59201be; root merged it at 421f783. Fresh
resulting-MAIN build and runtime verification passed all six original criteria.
The native foundation task is closed: actual SDK/spool/SQLite/read-only/crash,
corruption, bounded allocation and concurrent query paths pass. All 10,000 records
committed in the measured two-second outage run with zero rejections.
Original worker and rejection evidence remain retained. Full G06 remains open.

A bounded three-file quality review found a low-severity experimental global tracer
provider ownership defect. A native probe reproduced it, and root intake created
a distinct pending repair after the current telemetry owner. After G06 closure,
root reproduced the original defect against the fresh verified MAIN421 artifact
using the explicitly cached compiler with installation disabled. One whole Luna
owner now repairs per-instance telemetry plus actual native transport consumers
in an isolated worktree; independent review remains pending. The probe bypassed
project cache settings and triggered a user-home pinned-toolchain install; retained
evidence discloses this deviation and the actual compiler hash match to the cached
compiler. Application source and protected native artifacts stayed unchanged.

Native PostgreSQL row-policy qualification submitted clean 916298 from its
separate isolated worktree. Forty worker observations cover direct non-privileged
roles, paired-tenant queries, transaction reset, concurrency and denials. Independent
frontier native execution approved the candidate, which was fast-forwarded to
MAIN916298. A fresh merged native run and all five original criteria passed; the finite
qualification task is closed with source/result/configuration hashes and independent
evidence. Full G05 remains open.
The recorder lease expired at 23:04 UTC; root verified its old process absent and
restarted the single recorder at 23:06 UTC. The missing 23:05 capture remains explicit.
Cleanup 012 coalesced the missed interval after temporary agent capacity refusal.
Its read-only inventory retained 40 measured roots with zero eligible/deleted bytes;
eight active/owned roots, including the new a2 targets, were excluded before lstat.
Independent review rejected the helper for directory-replacement symlink traversal
and hidden aggregate enumeration errors. A separate whole Sol repair retains the
original evidence and empty deletion allowlist; prior matching totals do not approve
the unsafe helper.

Cleanup 012 a2 independently passed descriptor-relative race, error, exclusion and
current-inventory checks and is closed. All output was retained; zero bytes were
deleted. Its two earlier broad filename-search deviations remain disclosed, so
the whole procedure is not claimed fully scope-conformant. The next periodic
cleanup interval was coalesced after an actual agent-capacity refusal; tracked
cleanup013 independently passed its current report/21 exclusion and preservation
checks and is closed with zero eligible/deleted/reclaimed bytes.

Original B-G01-D01 submitted one planning document at 3c4f6ea, with actual cached
compiler identity and bounded native/WASM probe evidence. Independent review
rejected inaccurate wrapper job-count and stderr claims; a second whole economical
repair owns both corrections plus current six-crate/native-only-WASM applicability.
The repaired candidate 01c2ccf passed independent review and was fast-forwarded
to MAIN. A fresh resulting-source/compiler probe passed both original criteria and both
verification requirements; B-G01-D01 is closed. WASM compilation/module inspection
is verified; browser execution, framework and full G01 qualification remain open.

Writable execution resumed after the earlier permission interruption. The fixture
telemetry ownership repair is independently accepted and closed at 4ba3794: fresh
A/B/global routing, native WebSocket/HTTP2 modes, status/cancellation and all six
gates pass. The retained ba0 browser preview stayed fixed; no browser/device/audio
or full G06 completion is inferred.

Original browser pipeline decision B-G01-D02 is closed at 55f38cb after actual
fresh native/WASM checks, bindgen generation and refusal probes. Original contract
wave decision B-G03-D01 is closed at fe5588 after the namespace/window repair and
fresh native contracts/descriptor tests plus native/WASM gates. Its older draft
and rejected intermediate commit remain ancestry and are listed explicitly;
approval applies to the repaired final result. No production gameplay is implied.

Eight independent decision workers submitted bounded policy examples. Six current
candidates have independent approval but still await integration; recovery a2 has
independent approval after the missing literal example defect was repaired, and
the lease policy has a retained factual reserve-provenance rejection awaiting
repair. Queue completion requires resulting-source acceptance for each original
task. Current memory admission uses 3.2GiB system headroom and a combined 2GiB
build/review reserve; peak estimates do not prove additional separate reserves.

Cleanup014 and cleanup015 are independently accepted with no eligible, deleted
or reclaimed bytes. Current owned build/review roots are excluded before metadata.
The single official screenshot recorder was renewed on actual work only; expired
sessions and brief integration fencing remain explicit, without fabricated captures.

The artifact/runtime ignore task is closed at 9d38535 after independent real Git
boundary checks. Subsequent user-directed evidence ignoring was applied at 2e263fd:
2,219 evidence index entries were removed while all 2,219 current MAIN files and
351,395,266 bytes were preserved exactly. MAIN's newer local evidence was retained.
The directory remains durable local evidence protected from cleanup, despite its
Git exclusion. Independent resulting-MAIN acceptance passed all original criteria and the task is closed. New evidence is now ignored; none of the retained local files were deleted.
Source, schemas, plans and reusable environment templates remain trackable.

The next block starts at 2e263fdc475f886555f301c47e3628fe43abc775 and captures baseline..HEAD, including the separate bookkeeping commit writing this closed record.

## 2026-09-30T12:45:15-04:00 to 2026-09-30T16:17:22-04:00

Range: 4cac7647d4b2aa8e62f2c5c264d8561878383124..dd63d194a8e87ce575e861199abc28861ac2c28d

- f9343cb5ec17b33c76a03b20f6afec9168f9fb7a — Record verified S00 startup evidence and close implementation block
- 92460d4c58124bc713970da0d895fc59ffbfcb11 — Add owned local PostgreSQL 18.6 setup
- 25af2ecd54bfb2b918405d354972f7a652c05342 — Enforce PostgreSQL runtime ownership modes
- db403a4ca58ee5f221fb53e7fc61929345618cfe — Harden local PostgreSQL lifecycle ownership
- c68e168eeb2ab040ec1d6064765c647c4454a041 — Bind PostgreSQL lifecycle checks to exact server identity
- 51d59602c16a7a705c7c224fc03abd21b24828ac — Measure transport resources and exercise bounded desktop pressure
- ba0d3860fb27124be352b48766e99a955f746672 — Merge branch 'codex/qualify-g02'
- 0df797256ae35350521fa9dc0d4ece8d5b85b4a0 — Pin public SRD 5.2.1 source subset
- 6856a7457ad335028ccbe787ef01867f6e07c156 — Bound SRD source verification and attribution
- dd63d194a8e87ce575e861199abc28861ac2c28d — Merge branch 'codex/pin-srd-g07'

Owned PostgreSQL 18.6 development setup independently approved at `c68e168`; real
process identity, safe shutdown/refusal, transactions and restart persistence passed.
Public English SRD 5.2.1 source pin and bounded verifier independently approved against
merged `dd63d194`; full-book source, catalog and commercial rights gates remain open.

Transport candidate `51d5960` passed its five bounded desktop criteria and was merged
at `ba0d386`. Current native/WASM gates, nine native tests, browser semantics, pressure
and desktop/narrow visual checks pass. Final integrated reload is unperformed because
the Mac locked; the task remains in review. Full G02 device, complete memory, audio,
network and recovery qualification remains inconclusive. No gameplay completion claim.

Cleanup inventories 004–006 retain all output and durable evidence; no deletions.
The work session ends pending manual Mac unlock. All three previews and the owned
PostgreSQL runtime remain available; the bounded screenshot recorder is stopped.

Next block baseline: `dd63d194a8e87ce575e861199abc28861ac2c28d`. The bookkeeping commit writing this
block is captured in the next block, including workflow history and retained evidence.

## 2026-09-30T11:50:05-04:00 to 2026-09-30T12:45:15-04:00

Range: 0a2c281a02402493636fde9d93873055bf8a3e28..4cac7647d4b2aa8e62f2c5c264d8561878383124

- 8032257 — Record reviewed backlog expansion commit block (carried prior bookkeeping commit)
- 4cac764 — Start Rust S00 native gRPC-over-WebSocket browser fixture

Experimental S00 native/Rust-WASM transport fixture independently approved and integrated.
All ten browser checks, three native behavior tests, native/WASM style and compile gates passed.
Physical/adversarial resource qualification (G02), durable telemetry (G06) and gameplay remain pending.

Next block baseline: `4cac7647d4b2aa8e62f2c5c264d8561878383124`. The bookkeeping/evidence commit writing this block
is captured in that next block. This closing active work-session block is shorter than six hours.

## 2026-09-30T08:22:00-04:00 to 2026-09-30T10:05:41-04:00

Range: b64e8990b62178751380d511ffbb76435b6122dc..0a2c281a02402493636fde9d93873055bf8a3e28

- 47fd0fa — Record next-proof audit commit block (carried prior bookkeeping commit)
- 0a2c281 — Expand reviewed backlog to 3145 scoped planning records

Next block baseline: `0a2c281a02402493636fde9d93873055bf8a3e28`. The bookkeeping commit writing this block
is captured in that next block. This closing active work-session block is shorter than six hours;
the prior bookkeeping commit is included by revision range rather than time filtering.

## 2026-09-30T03:18:40-04:00 to 2026-09-30T03:31:24-04:00

Range: fcbd3898cd8858945b3e6dbd3c8fda3f8b3f2b63..b64e8990b62178751380d511ffbb76435b6122dc

- 8ab1523 — Record reviewed commercial refinement commit block
- b64e899 — Record next executable proof decision and audit history

Next block baseline: `b64e8990b62178751380d511ffbb76435b6122dc`. The bookkeeping commit writing this block
is captured in that next block; this closing work-session block is shorter than six hours.

## 2026-09-30T02:58:24-04:00 to 2026-09-30T03:18:40-04:00

Range: 12dba460ba1a3d3a5566e1074938520041da2b34..fcbd3898cd8858945b3e6dbd3c8fda3f8b3f2b63

- aaf44f5 — Record initial six-hour development commit block
- fcbd389 — Qualify campaign offers against joint generation and support costs

Next block baseline: `fcbd3898cd8858945b3e6dbd3c8fda3f8b3f2b63`. The bookkeeping commit writing this block
is captured in that next block; this closing work-session block is shorter than six hours.

## 2026-09-30T01:13:16-04:00 to 2026-09-30T02:58:24-04:00

Range: empty baseline..12dba460ba1a3d3a5566e1074938520041da2b34

- cd0c8af — Init commit
- 12dba46 — Refine reviewed service contracts and preserve development evidence

Next block baseline: `12dba460ba1a3d3a5566e1074938520041da2b34`. The commit writing this block remains pending
for capture in that next block. This final work-session block is shorter than six hours.
