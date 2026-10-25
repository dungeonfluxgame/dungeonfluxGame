# gRPC-over-WebSocket compression and resilience research

Date: 2026-10-25
Status: research recommendation; no transport behavior change authorized here

## Scope and evidence

- Scenario: one television display and up to six phone controllers over home Wi-Fi
  or mobile data. Runtime RPC and image/audio transfers share the WebSocket tunnel;
  ordinary HTTP is for bootstrap only (owner brief; transport shape in
  [RPC transport](rpc-transport.md#direction), lines 8-22).
- The deployed design is binary WebSocket carrying the native HTTP/2 byte stream,
  then gRPC/protobuf (same document, lines 8-18). Bridge source confirms its
  purpose and current limits: 16 KiB frame, 256 KiB WebSocket message, 1 MiB
  receive budget, 64 KiB RPC message, eight concurrent streams
  (`crates/df-rpc-bridge/src/lib.rs:1-7`).
- The browser adapter uses `web_sys::WebSocket`, queues binary callback messages,
  and closes on message/receive/count overflow
  (`crates/df-rpc-bridge/src/browser.rs:67-77,95-152`).
- Browser connection open currently times out after 5 s
  (`crates/df-rpc-bridge/src/browser.rs:187-208`). That is a socket-open deadline,
  not a first usable RPC/view deadline.
- Native tunnel keepalive sends every 15 s and drops after 45 s idle
  (`crates/df-rpc-bridge/src/native.rs:20-33`). Ping/pong are handled in the
  WebSocket pump (`native.rs:347-348,361-369`). The browser WebSocket API does not
  expose control-frame ping; the current native pings can be answered by the
  browser network stack while the page is suspended, but cannot prove Rust/WASM
  application progress (`native.rs:20-24`).
- Existing transport research says WebSocket has no receive backpressure and
  receive memory must be bounded via HTTP/2 DATA credit; it sets an 8 MiB total
  outstanding envelope and prioritizes action/audio/control over assets
  (`planning/rpc-transport.md:121-143`).
- Client connection generations cancel old requests; current views retain the
  binding/revision watermark and reject stale generations
  (`crates/df-client/src/connection.rs:197-220`,
  `crates/df-client/src/connection_views.rs:14-52`).
- The existing close/reconnect bridge contract checks close wakeup, cleanup,
  retained buffered prefix and counters; it does not yet simulate delay, loss or
  reconnection replay (`crates/df-rpc-bridge/tests/close_reconnect_contract.rs:175-190`).
- The API design already names operation IDs/fingerprints, stable seat identity,
  binding generations, idempotency windows, operation lookup, revision-stamped
  snapshots, and bounded asset offsets (`planning/rpc-api.md:124-131,144-171,428-468,542-546`).
- The API also defines chunk sequence/offset, bounded active upload, acknowledgements,
  and explicit terminal outcomes for audio/capture (`planning/rpc-api.md:512-545`).
- Recovery design requires a new full snapshot on presentation epoch change and
  monotonic sequence in an epoch; old one-shot media is not replayed
  (`planning/rpc-api.md:455-468`).
- Reliability guidance explicitly requires bounded backoff, foreground resync,
  duplicate-tab ownership, current authorized initial view and reconnect separate
  from join (`planning/runtime-reliability.md:76-90`).
- Remote-play acceptance calls out packet delay/loss, reconnect mid-cue, sleep/AFK,
  revocation, action-to-view latency, bytes, CPU and memory
  (`planning/remote-play.md:67-81`).
- Caddy's current configured values are server idle 120 s, upstream dial 3 s,
  response header 60 s, flush immediately, and upgraded WebSocket close delay 5 min
  (`deploy/edge/Caddyfile.production:43-48,67-83`). The file labels itself
  unverified; run Caddy validation and qualify the optional load balancer before
  treating those settings as observed deployment behavior (`Caddyfile.production:1-6`).
- LAUNCH decision 32 establishes the single production origin and `/sessions/*`
  routing (`artifacts/tmp/agents/LAUNCH.md:139-144`); decision 33 establishes the
  internal PKI identities for the edge and placement (`LAUNCH.md:145-149`). Decision
  23 routes media bytes through the control-plane media gateway rather than object
  store URLs (`LAUNCH.md:105-107`). None of those decisions selects compression or
  fixes an external load-balancer idle timeout.
- Limits below are source facts. Size/rate scenarios are planning estimates from
  schema shape and explicit assumptions, not runtime measurements. No live-network
  call or platform qualification was performed.

## 1. Compression recommendation

### Decision

- Begin with no transport-wide compression. Preserve the current opaque h2 tunnel
  and its byte accounting until an end-to-end benchmark proves a useful saving.
- If measurements show view/control bandwidth is material, trial gRPC per-message
  gzip for selected protobuf messages above a size threshold; compression stays
  attached to each gRPC message, has no cross-message dictionary, and media bodies
  are explicitly excluded.
- Do not enable WebSocket `permessage-deflate` for the tunnel by default. It sits
  outside the gRPC framing and affects each WebSocket message after the adapter has
  split/reassembled h2 bytes. Its gain depends on WS message boundaries, so repeated
  h2 headers are not guaranteed to line up as useful compression units.
- Do not stack gRPC gzip and WebSocket deflate. Double compression spends CPU and
  adds memory/latency while gzip/deflate media or binary payload gains little.
- Do not select zstd until the chosen tonic/prost stack, generated client, browser
  WASM codec and peer negotiation all demonstrate interoperable support. This is
  **verify**; protobuf itself does not specify a compressor.

### Why and trade-offs

- gRPC compression applies to the protobuf message payload, not the five-byte
  gRPC message envelope or HTTP/2 framing. HTTP/2 DATA is not automatically
  compressed. The WebSocket extension compresses WebSocket messages, so in this
  bridge its layer sees opaque h2 frames; either layer may save bytes, but only the
  gRPC layer understands message boundaries and media type.
- `gzip` is the conservative interoperability candidate. Compression reduces
  cellular bytes for repetitive text/field names, at a CPU and allocation cost on
  the server and low-end phone. WASM CPU time can delay input/rendering on the main
  thread; measure encode/decode time and 95th/99th percentile action-to-view.
- `deflate` is an ambiguous name across HTTP implementations and has little reason
  to be selected over the already supported gRPC codec set; verify actual stack
  support and wire negotiation before considering it.
- `zstd` may offer a better speed/ratio point, but adds a dependency/codec and
  browser WASM cost. Verify binary size, license, build target, CPU and maximum
  decoded output before experiment.
- No compression gives predictable CPU and no decompression-bomb surface; cost is
  more bytes for repeated protobuf strings and captions. Current per-RPC limit is
  64 KiB (`df-rpc-bridge/src/lib.rs:3-7`). Keep a strict post-decompression cap no
  larger than the established message envelope, plus a separate ratio/time bound.
- One independent compressor per message has bounded state near one message and
  no dictionary carried across users/requests. If the chosen codec buffers, bound
  its input/output to one message and charge that memory against per-connection
  accounting. Benchmark actual heap peak; do not infer it from compressed size.
- `permessage-deflate` may retain a sliding-window context per direction and
  connection when context takeover is negotiated. That multiplies memory by all
  phone/display sockets and creates state whose reset points follow WS messages,
  not gRPC calls. With one TV plus six phones this means seven independent sessions
  at one table; measure at hosted connection scale too.
- Browser JS WebSocket APIs generally negotiate extensions with the browser, not
  with an application-provided compressor knob. Whether the deployed browser and
  Rust WebSocket server negotiate/disable context takeover in the exact required
  way is **verify** on supported iOS, Android and TV browsers. Do not infer uniform
  behavior from desktop Chrome.
- CRIME/BREACH-style compression oracles need a secret and attacker-influenced
  bytes to share observable compressed context. TLS compression is not proposed.
  gRPC per-message compression resets at message boundaries, lowering cross-message
  leakage; still avoid compressing any message that combines credentials/tokens or
  other secrets with attacker-controlled text. WS context takeover across arbitrary
  tunnel bytes is harder to reason about and could mix calls; disable it or avoid
  the extension. The actual secret/content co-location in each RPC is **verify**
  against generated messages before enabling any compression.
- Images encoded as WebP/JPEG/AVIF and audio encoded as Opus are already compressed:
  do not recompress them. Send bytes as-is and verify hashes on completion. If raw
  PCM or uncompressed structured metadata is ever sent, decide based on measured
  size and deadline.
- Image transfer currently shares the single RPC transport per the owner brief;
  application API design schedules assets below action/audio/control
  (`planning/rpc-api.md:15-17`; `planning/rpc-transport.md:136-143`). Compression is
  not a substitute for priority scheduling or congestion control.
- Revision/delta updates are the first optimization: avoid resending unchanged
  state by using the designed revision/sequence, coalescing presentation updates,
  and requesting a full snapshot only when required. Compression is a second
  optimization over the remaining bytes. The current proto `WatchViewRequest`
  carries `after_revision`, `ViewMessage` carries revision, and `JourneyView`
  contains audience-specific projection (`gameplay.proto:24-27,176`). The API's recovery design distinguishes durable revision from same-revision view sequence
  (`rpc-api.md:428-458`). Do not invent client-side game-state deltas that could
  lose authority or private-field filtering.

### Local size and bandwidth envelope

- Protocol definitions are under `crates/df-protocol/proto/`; relevant messages
  include `ViewMessage`, player/display views, `JourneyView`, `TableLine`, audio,
  asset and chunk messages (`gameplay.proto:24-27,176,193`; media contracts in
  `planning/rpc-api.md:512-570`). The current maximum RPC body is 64 KiB and the bridge's outer WS-message maximum is 256 KiB (`df-rpc-bridge/src/lib.rs:3-7`).
- Proto declarations bound some variable inputs but do not set a serialized-size
  budget for a normal view. `JourneyView` can carry repeated party/log/notebook and
  combat data (`gameplay.proto:176-193,243-248`); actual size depends on content.
- No representative serialized `ViewMessage`, image, audio, or upload fixtures
  are present in the designated production schema set. The transport fixture uses an opaque `bytes payload`, so it cannot stand in for gameplay media
  (`transport_fixture.proto:10-23`). Therefore the following are scenario estimates,
  not measurements. Add stable representative fixtures before choosing a codec.
- Estimate assumptions: six phones plus one display; each phone receives one
  8-24 KiB projected snapshot/update per second; display receives one 8-32 KiB
  public update per second. This models a busy visible scene, not guaranteed cadence.
- Phone view egress: 6 x 8-24 KiB/s = 48-144 KiB/s (0.39-1.18 Mbit/s).
- Display view egress: 8-32 KiB/s (0.07-0.26 Mbit/s).
- Combined view-only server egress: 56-176 KiB/s (0.46-1.44 Mbit/s) per table;
  a burst of six 64 KiB full snapshots plus one 64 KiB display snapshot is 448 KiB
  (3.67 Mbit) before framing/compression.
- Image traffic: metadata/URLs may be only hundreds of bytes per update; actual
  image bytes dominate and vary widely. Illustrative 0.5-3 MiB image delivered once
  to seven devices costs 3.5-21 MiB total; paced at 1 Mbit/s this is about 29-176 s
  aggregate server egress time, although recipients can share cached/CDN bytes only
  if a permitted architecture actually routes them there. Current launch decision
  routes media through the gateway (`LAUNCH.md:105-107`).
- Audio estimate: if one shared Opus stream averages 24-64 kbit/s, one listener
  receives 3-8 KiB/s; seven listeners receive 21-56 KiB/s total. Six distinct
  private streams at the same rates plus a shared display stream are 21-56 KiB/s
  combined. Codec and bitrate are assumptions; negotiated formats remain a browser
  test gate (`rpc-api.md:524`).
- Upload estimate: a 16 kHz, mono, 16-bit PCM microphone is 32 KiB/s before
  transport framing; compressed speech at 16-32 kbit/s is 2-4 KiB/s. The API
  requires declared codec/format and bounded chunks (`rpc-api.md:514-524`). Actual
  device format, preprocessing and concurrency are **verify**.
- Typical busy session estimate with view updates, one shared audio stream and one
  16-32 kbit/s upload is roughly 61-188 KiB/s egress plus 2-4 KiB/s ingress,
  excluding image transfer. Peak estimates must add image bursts and reconnect
  snapshots rather than pretending they are steady state.
- Measurement to run before selecting compression: serialize actual six-member
  player and display views at lobby, exploration, combat and journal-heavy phases;
  record payload bytes and compressed bytes/time at levels 0/1/3; measure browser
  main-thread encode/decode p50/p95/p99, WASM memory, server CPU, and first-byte,
  action-to-view, audio-start under 3G-like delay/loss. Report bytes and CPU per
  session, not just ratio.

## 2. Liveness on spotty links

- Keep one primary liveness mechanism. WebSocket Ping/Pong is currently initiated
  by native bridge every 15 s with 45 s idle timeout (`native.rs:20-33`). Browser
  cannot schedule a JS control ping; it may auto-answer the control frame. This
  tests the path to the socket stack, not whether the background tab's RPC consumer is making progress.
- H2 PING is separate from WebSocket Ping; vendored h2 has a separate ping/pong handler
  (`crates/df-rpc-bridge/vendor/h2/src/proto/connection.rs:48-49,214-216`).
  The current tunnel byte pump treats WS control frames separately from tunneled HTTP/2 bytes
  (`native.rs:347-348`); do not assume the vendored h2 PING crosses as a WS control
  frame. Verify vendored h2 configuration and actual emitted control frames before
  enabling a second cadence.
- Recommendation: retain native WS ping at 15 s / 45 s while measuring mobile
  radio wakes and edge behavior. For foreground app-health, add a low-frequency
  application heartbeat only if a real failure mode requires it; target 30 s send,
  90 s deadline as a starting **verify** value, suspended whenever the document is
  hidden. Do not send redundant WS, h2 and app pings every few seconds.
- A sub-30 s cadence may keep a cellular radio in a higher-power state or wake it
  more often; cost varies by modem/OS and is **verify** on devices. Background timer
  throttling means browser timers cannot enforce a 15 s application deadline.
- On `visibilitychange` to visible or `online`, check connection generation and
  immediately perform a bounded health/resume operation. On hidden/offline, stop
  user-visible retry loops and allow native heartbeat/edge timeout to clean up.
  Browser event behavior and online/offline accuracy are hints, not proof of reachability.
- Use staged deadlines: WebSocket handshake/open 5 s today (`browser.rs:187-208`);
  recommend bootstrap connect budget 8 s total with one retry only while visible;
  first authenticated RPC/initial view 10 s; foreground silent-connection deadline
  45-60 s; command deadline per method (short interactive commands 10 s, long
  provider-backed work uses accepted/pending receipt). Values need real remote
  p95/p99 qualification and are recommendations, not source settings.
- Track distinct timestamps and outcomes: DNS/TLS/upgrade, socket open, RPC headers,
  first payload, full snapshot, ping RTT, last server byte, last client byte, queue
  delay, and close reason. Existing bridge has byte and close metrics but not this
  complete stage timeline (bridge sources `resources.rs`; add shared OTEL stages).
- Half-open detection: use successful Pong/heartbeat response deadline on native;
  visible foreground RPC health on browser. A socket `readyState == OPEN` or
  `navigator.onLine` must not be treated as proof that a captive portal or dead
  route can reach the service.
- Align server and edge timers: app liveness timeout must be below edge idle 120 s;
  current WS ping interval is already well below it. Verify whether the Caddy
  `servers.timeouts.idle` applies to upgraded streams in the pinned Caddy release
  and whether the DigitalOcean LB has a shorter timeout. Keep a measured margin.
- TV browsers are often foreground/always-on but may sleep the display or suspend
  the app; phones lock and background tabs aggressively. Treat TV as the durable
  public projection but not a trusted clock. When TV reconnects, restore current
  public snapshot/timeline; when phone resumes, restore only its authorized player
  view and input lease.

## 3. Reconnection and resume

- Backoff: exponential with full jitter, e.g. random delay in `[0, min(30 s,
  500 ms x 2^attempt)]`, reset after a stable 30 s connection. Cap visible retries
  at 30 s; hidden pages wait for foreground; offline waits for `online`. Add a
  small initial random spread to avoid all phones reconnecting simultaneously.
- Retry only transport establishment and explicitly safe reads. For any command
  with an uncertain outcome, retain its original operation ID and canonical
  fingerprint, call operation lookup/resume, and never mint a new ID to replay the
  tap. RPC API already requires result lookup and says cancellation does not undo
  committed actions (`planning/rpc-api.md:144-171,230-236`).
- Resume request carries stable session/run identity, existing client binding,
  connection generation, last applied SessionRevision, presentation epoch and
  view sequence, plus pending operation IDs. Do not put credentials in the URL;
  transport plan explicitly forbids long-lived credentials in URLs
  (`rpc-transport.md:102-105`).
- Server keeps a small bounded replay tail of presentation events only when they
  are safe and useful. Proposed bound: 256 events and 1 MiB or 30 s, whichever
  comes first, per binding/session; if requested cursor predates retained tail,
  respond with typed resync-required and one full authorized snapshot. This is a
  starting design budget, not a current server implementation. Never replay a
  one-shot audio/cinema cue just because it was in the ring.
- Revisioned views may resume from latest applicable state, not necessarily replay
  every intermediate snapshot. Preserve `SessionRevision`, presentation epoch,
  and `view_sequence`; reject old generation messages. A new epoch/full snapshot
  cancels old audio buffers and cues (`rpc-api.md:455-468`).
- Commands are never dropped to recover from slow links. Keep a bounded client
  outbox only for commands with stable IDs, payload fingerprint and expiry; show
  each as queued/unknown/accepted/rejected. On reconnect, resolve unknown first.
  The server must reject same key/different payload and preserve compact dedupe
  evidence for the advertised retry window (`rpc-api.md:144-170,197-199`).
- Seat fencing: Resume preserves member/seat and does not allocate another one.
  Bind/takeover uses explicit intent, principal/session/binding namespace and
  connection generation; server atomically replaces the lease and fences old
  streams/commands. A stale reconnect may recover its own current binding only if
  still authorized; it cannot steal an active seat or revive a revoked lease
  (`rpc-api.md:126,152-163,448-462`; reliability `:83-87`).
- Phone UI during reconnect: preserve the current permitted view read-only, show
  a clear "Reconnecting" state and last confirmed revision/time, disable actions
  that cannot yet be submitted, preserve unsent text locally, and label submitted
  commands with unknown outcome until lookup resolves. Never show optimistic success
  for a command with no receipt.
- TV UI: keep the last safe public scene visible with a discreet stale/reconnecting
  indicator; do not animate timers forward or replay completed cues. On resume,
  request current display snapshot plus active timeline and transition to it once.
  TV is typically a public spectator, so private phone state must never leak into
  its recovery payload.
- Browser events: use `online`/`offline` and `visibilitychange` to accelerate or
  defer attempts, not to infer successful transport. Recreate the WS channel and
  increment client connection generation on each attempt. Close/cancel old stream
  readers before installing new generation; current client fences old scopes
  (`connection.rs:197-220`, `connection_views.rs:31-52`).
- Resume acceptance is same binding/member without new seat allocation, monotonic
  view revision/sequence, one result per operation ID, no duplicate audio/one-shot
  cue, active stream authorization rechecked, and full snapshot when replay cursor
  is too old (`rpc-api.md:448-468`).

## 4. Flow control and backpressure

- Maintain end-to-end byte bounds. Existing bridge constants cap frame 16 KiB,
  WS message 256 KiB, browser receive queue 1 MiB, RPC body 64 KiB, and concurrent
  h2 streams 8 (`df-rpc-bridge/src/lib.rs:3-7`; browser queue checks at
  `browser.rs:119-128`). Transport research specifies 8 MiB aggregate outstanding
  connection envelope (`rpc-transport.md:123-134`). Do not raise a bound based
  only on compression savings.
- Keep bounded per-connection and per-stream send queues. Reserve capacity for
  command receipts, revocation/resync control, and audio; bulk asset chunks use
  remaining weighted capacity. Expose current/peak queued bytes, oldest age,
  queue rejection/coalescing counts and stream credit in existing connection
  metrics (`df-rpc-bridge/src/resources.rs:211-320`).
- View updates are replaceable state: while one update is in-flight, coalesce
  pending updates to the newest safe snapshot; preserve required ordered control
  events separately. If a delta base revision is skipped, request/resend a full
  snapshot. Do not drop command receipts, audio terminal markers, lease revocations,
  upload acknowledgements, or durable journal events.
- A slow client should not block session owner or other participants. Bound each
  recipient queue; after a high-water mark, stop queuing obsolete views, reduce
  optional presentation frequency and eventually close/resync that stream. Record
  slow-consumer reason. Runtime reliability requires bounded queues and defined
  overflow (`runtime-reliability.md:48-60`).
- Large images over the same connection cause TCP head-of-line delay for later
  command/audio bytes even when h2 streams are prioritized. First use smaller
  chunks, weighted scheduling, per-RPC priority, h2 flow credit and concurrency
  limits. Keep same connection until measurements show p95/p99 interactive
  starvation despite those controls; then consider a second WSS connection through
  the same RPC bridge for bulk only, with separate admission and aggregate cap.
  Do not move media to an HTTP URL contrary to owner-specified gRPC-only runtime.
- Uploads: preserve the documented ordered chunks with explicit offsets/sequence,
  bounded bytes/in-flight chunks, per-chunk ack, resumable upload ID and durable
  offset/hash checkpoint. On reconnect, resume only after server returns accepted
  offset; repeated chunk ID+same hash is idempotent, same ID+different bytes is
  conflict. Existing `df-rpc-bridge/src/upload.rs` cancels pending client upload
  work when its connection closes (`upload.rs:16-40,48-80`); durable resume is a
  higher-level API responsibility, not a transport retry.
- For images, transfer metadata then bounded chunks and explicit completion; verify
  total size and hash before cache admission. This matches API design
  (`rpc-api.md:542-546`). Do not resume at an arbitrary byte boundary for codecs
  that require whole-object integrity unless chunk hashes are part of the manifest.
- Audio recovery uses new stream epoch/current timeline, not arbitrary byte skip;
  bounded jitter/decode buffers, explicit end/cancel and drop obsolete epoch
  (`rpc-api.md:526-538`; `runtime-reliability.md:96-105`).

## 5. Edge and protocol deployment

- Current Caddy source comments say upgraded WS stays open across reload for up to
  five minutes and app reconnect handles drain (`Caddyfile.production:33-38,75-83`).
  `stream_close_delay` is not a promise that the game session survives deploy.
  Send a drain notice, stop new admissions, keep sessions owned until bounded grace,
  then close with retryable reason; durable state and operation lookup govern truth.
- Caddy idle timeout is 120 s (`Caddyfile.production:43-48`) while WS keepalive
  is 15 s. Verify actual deployed Caddy version, reverse-proxy semantics, upstream
  LB/NAT/captive portal timeouts and route. Align heartbeats with the shortest
  intermediary timeout with margin; configuration currently says `UNVERIFIED`
  (`Caddyfile.production:1-6`).
- Current edge uses HTTP reverse proxy to control plane and the native tunnel is
  WebSocket upgrade; current config does not show RFC 8441 WebSocket over HTTP/2.
  Decision 32 uses one public origin (`LAUNCH.md:139-144`).
- Browser WebSocket over HTTP/1.1 upgrade is the compatible baseline. RFC 8441
  extended CONNECT permits WS over HTTP/2 only when client, edge, and upstream all
  negotiate it; browser and Caddy support in deployed chain is **verify**. Do not
  assume end-to-end h2 upgrade just because the inner tunnel is h2.
- WebTransport/HTTP/3 may reduce some TCP head-of-line blocking and support
  datagrams, but it changes transport semantics, browser support, edge operations
  and reliability tests. Keep as a future experiment only; do not change gRPC-only
  contract without owner decision and parity evidence.
- Captive portals may accept TLS/upgrade then stall or redirect pre-auth requests;
  authentication and origin checks must still happen at RPC boundary. A portal
  HTML response is a connect/protocol failure, not a new session or valid gRPC
  status. Avoid exposing credentials in URL/logs (transport `:102-105`; Caddy
  redaction `:21-31`).
- Deployment drain acceptance: readiness fails before shutdown, no new session
  allocation to draining instance, active operation receipt remains queryable,
  old streams close with retryable status inside configured delay, reconnect lands
  on healthy instance and preserves binding/session, observability has drain start,
  close and resume reason. OPS-01 is referenced by owner brief; confirm exact plan
  text and readiness contract before implementation (**verify**).
- Never let proxy buffering add latency. Caddy currently sets `flush_interval -1`
  (`Caddyfile.production:67-76`); verify this behavior for WS upgrade, not only
  HTTP response streaming.

## 6. Local and CI fault testing

- Windows developer path: run bridge server/client on loopback and insert a Rust
  fault proxy in the integration harness between the WebSocket client and server.
  Proxy owns both sockets and forwards binary frames with seeded configurable
  delay, jitter, loss, bandwidth limit, half-close, reset, pause/resume and blackhole.
  Control frames are forwarded consistently unless the scenario targets them.
- Keep deterministic seeded profiles: clean; 150 ms RTT + 30 ms jitter; 1% random
  frame drop/connection reset (WS is reliable so inject TCP reset/stall, not silent
  independent WS-message loss); 500 ms burst stall; 64 kbit/s uplink; reconnect
  during action receipt; reconnect during asset chunk; background/sleep simulation.
- Do not add an external dependency without checking existing Cargo dependencies
  and the workspace source-only policy. CI should use the same Rust proxy executable
  or test-only helper, no public internet and no real player identity.
- Existing bridge integration test already starts an in-process Axum WS server
  and exercises close/cleanup (`close_reconnect_contract.rs:37-83,175-190`); extend
  that harness at the stream boundary, or add a separate test helper in `tests/`.
- Local Windows traffic shaping alternatives (verify availability in CI): Windows
  QoS policies/NetQosPolicy shape traffic but are awkward for deterministic loss;
  third-party GUI shapers are not reproducible. Prefer the test proxy. Linux CI can
  additionally use `tc netem`, but make it optional and not the sole acceptance
  path.
- Log/test only synthetic identifiers and payloads. Preserve the project shared
  OpenTelemetry path; correlate attempt, connection generation, session revision,
  RPC operation, stage and fault profile without logging credentials or private
  speech (`rpc-transport.md:107-111`; Caddy redaction `:21-31`).
- Test current binary/build, pinned Caddy config and exact test proxy profile.
  Gather p50/p95/p99 handshake, first-view, action-to-view, audio-start, recovery;
  bytes in/out; CPU; WASM heap; per-connection queue peak; disconnect reason;
  duplicate/unknown operation count. Fix target directory and build revision for
  comparisons; no stale binary.

### Acceptance catalog

- NET-01 Clean reconnect: close socket while idle; client reconnects same binding,
  current authorized full view arrives, no new seat created.
- NET-02 Loss during mutation: server commits but client loses receipt; client uses
  same operation ID/GetOperation, receives one receipt, action applies once.
- NET-03 Unknown mutation: request never reached server; lookup returns not-recorded;
  new user intent gets a new ID only after explicit client action.
- NET-04 Revision resume: reconnect with retained view sequence; latest state is
  monotonic, no stale generation overwrites, cursor-too-old returns full snapshot.
- NET-05 Epoch reset: old audio/animation buffers stop; new epoch snapshot restores
  current timeline without replaying finished one-shots.
- NET-06 Seat fence: old and new phone connection race; explicit authorized takeover
  wins once, stale binding cannot submit or publish private view.
- NET-07 Slow consumer: saturate display receive/send path; view snapshots coalesce,
  receipts/control survive, queue stays under bytes and item cap, owner latency bounded.
- NET-08 Media contention: saturate asset stream during action and audio; measure
  p95/p99 action-to-view and audio start; asset traffic yields to control.
- NET-09 Upload resume: drop at chunk boundary, reconnect, continue from server
  acknowledged offset, hash matches, duplicate chunks do not duplicate commit.
- NET-10 Keepalive half-open: blackhole path; native detects within configured idle
  bound, foreground client resumes; hidden phone timer throttling does not cause
  false seat loss.
- NET-11 Captive portal/proxy timeout: redirect, close, stall and short idle timeout
  produce visible retry state, no new session, credentials absent from logs.
- NET-12 Drain: readiness flip and deploy close preserve queryable operation result;
  reconnect reaches healthy placement within drain policy.
- NET-13 Compression: representative protobuf view fixture shows wire bytes, codec
  time, memory and action/audio latency under both Wi-Fi-like and mobile profiles;
  media bytes are unchanged; decompression cap rejects oversized expansion.
- NET-14 Browser matrix: iOS Safari, Android Chrome, TV browser and desktop control
  actual foreground/background, online/offline events, WS extension negotiation,
  audio resume and all four RPC modes. Record unsupported behavior as a gate, not
  assumed pass (`rpc-transport.md:140-149`).

## 7. Prioritized build plan

The order is smallest-first. These are proposals only; do not implement under this
research task. Each step keeps contracts in planning/RPC API as the authority and
uses source changes only after the shared API numbering and browser gate are frozen.

| Rank | Scope and concrete paths | Acceptance | Before/after measures |
|---|---|---|---|
| 0 | Measurement fixture: `crates/df-protocol/tests/` and test-only fixture module. Serialize lobby, exploration, combat, long-log player/display views; include representative image/audio/upload chunks. | Stable encoded lengths by message type and phase; fixtures remain synthetic. | Payload p50/p95/max; image/audio bytes and cadence. |
| 1 | Transport timing instrumentation in `crates/df-rpc-bridge/src/resources.rs`, `native.rs`, `browser.rs`; shared OTEL through established observability path. Add stage timestamps without logging payloads. | Correlated connect/open/first-RPC/first-view/close timings and queue peaks. | Cold/warm connect, first view, bytes, memory, ping RTT. |
| 2 | NET proxy harness in `crates/df-rpc-bridge/tests/` and test-only Rust support; extend `close_reconnect_contract.rs`. | NET-01, NET-07, NET-10 deterministic under seeded profile. | Fault-injected result, queue bounds, cleanup, detection latency. |
| 3 | Client recovery controller in `crates/df-client/src/connection.rs`, `connection_views.rs`, and caller-owned UI state crate. Add jittered attempt state, online/visibility triggers, stale-generation rejection and reconnect UI contract. | NET-01, NET-04, NET-05; foreground/background browser smoke. | Recovery p50/p95, stale frame count, user action availability. |
| 4 | API resume/idempotency contracts in `planning/rpc-api.md`, then shared owner schema freeze and implementation in owning RPC/session crates. Add resume cursor, `GetOperation`, typed resync, replay bounds, binding fence. | NET-02..NET-06 with durable fixture/fake store and concurrent reconnect. | Duplicate apply count = 0; replay bytes/events; resume vs full snapshot bytes. |
| 5 | Backpressure scheduling in `df-rpc-bridge` plus server stream owner: coalesce views, reserve control/audio, cap per-client bytes and instrument drops. | NET-07, NET-08; full queue does not block owner. | Queue peak, action/audio p95/p99 with asset saturation, coalesced count. |
| 6 | Resumable media/upload contracts and owning media/API crates: upload ID, ack offset/hash, asset ranges, chunk replay policy. | NET-09; injected drop at every chunk boundary, same hash after resume. | Retry bytes, resumed completion time, duplicate commit count, memory peak. |
| 7 | Compression experiment only after rank 0-5 baseline: configure gRPC message codec at generated client/server boundaries; media explicitly identity. Do not edit tunnel adapter to compress arbitrary bytes. | NET-13 on native and browser WASM, actual browser matrix; no correctness or timing regression. | Compression ratio and CPU at sizes/levels; bytes/table; WASM binary/heap; p95/p99 action/audio. |
| 8 | Edge drain qualification in `deploy/edge/Caddyfile.production`, staging config and operations runbook; validate Caddy and optional LB. | NET-11, NET-12 with deploy simulation and short-timeout proxy. | Drain duration, close reason, reconnect success, operation lookup success. |
| 9 | Optional WS-over-h2 or WebTransport research spike only if rank 5 measurements demonstrate transport head-of-line limits that cannot be addressed by queue scheduling. | New compatibility decision, supported browser/edge matrix and same NET suite before any rollout. | Latency under loss, handshake, network cost, implementation and ops burden. |

### Required comparison gates

- Keep baseline build and fault profile fixed; run each codec against same serialized
  fixtures and deterministic 7-client traffic sequence.
- Compare identity/no compression, gRPC gzip at documented levels and any proposed
  zstd build. WebSocket extension test is separate and requires evidence of actual
  negotiation and context-takeover settings. Do not stack codecs in first trial.
- Reject a codec if it improves bytes but materially worsens interactive latency,
  CPU, WASM heap, battery/radio use, startup, or failure recovery. No arbitrary
  ratio improvement qualifies without action/audio measures.
- First deployment should be server-configurable and safe to disable, negotiated
  per RPC capability, with no silent fallback to incompatible compressed data.
  Message-size, decompressed-size and deadline caps apply before parse.
- Keep exact codec defaults and minimum client capability version in the frozen RPC
  API/compatibility fixtures. Protocol/body semantics remain unchanged; do not
  compress ciphertext or already-compressed media a second time.

## 8. Open verification gates

- **Verify** supported iOS, Android and TV browsers, background timer/WS behavior,
  extension negotiation and permessage-deflate context takeover with the real build.
- **Verify** vendored h2 PING configuration; it is not the same as WS ping/pong or
  an RPC application heartbeat.
- **Verify** deployed Caddy/LB/NAT/captive-portal timeout and five-minute drain
  behavior; Caddy source is marked unverified.
- **Verify** image-size distribution, six-client audio topology, microphone codec,
  gRPC codec negotiation and WASM compatibility; no live fixture supports estimates.
- **Verify** OPS-01 drain/readiness contract, replay-buffer owner and durable
  operation-result retention before fixing production bounds.
- Browser transport remains an explicit feasibility gate (`rpc-transport.md:43-62,140-149`).

