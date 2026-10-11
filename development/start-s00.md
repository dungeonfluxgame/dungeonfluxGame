# Experimental S00 transport fixture

This document covers the S00 transport lab only; start the playable game with
`development/run-gameplay.sh`.

Task `START-S00-001`, attempt `START-S00-001-a1`, input
`8032257c9a3865f683f80e94134ab9fc6a404173`. This is a synthetic loopback experiment,
not gameplay, a production bootstrap service, or full G02/G06 approval.

## Delivered execution path

Rust browser UI → generated tonic protobuf client → custom `GrpcService` backed
by h2 → local single-threaded h2 driver → bounded web_sys binary WebSocket byte
stream → native bounded byte pump → tonic generated service. HTTP/2 and gRPC are
unchanged; there is no grpc-web translation or custom RPC envelope. The UI and
application logic are Rust. The only browser script is wasm-bindgen module boot.

Four crates exist because each has runnable responsibility: `df-protocol` owns the
experimental schema/generation; `df-rpc-bridge` owns native/browser byte adapters;
`df-observe` owns explicit OTEL correlation and bounded fixture export; `df-tools`
owns the synthetic native composition and Rust/WASM test interface. No game domain,
rules, database, provider or production API types are added.

The experimental namespace is `dungeonflux.experimental.transport.v1`. Sample
numbering and four service modes are frozen for this experiment in
`crates/df-protocol/proto/transport_fixture.proto`; they are not production G03 APIs.
A bidi reply arrives while the request remains open; the client then half-closes
and observes terminal trailers. Explicit fixture control behavior `REJECT` and
`WAIT` permits deterministic rejection and cancellation. Synthetic diagnostic fields
report native owners and observed grpc-timeout headers without payload logging.

## Local toolchain and reproducible commands

Stable Rust **1.98.1** was verified from the official stable manifest dated
2026-09-03 and pinned in `rust-toolchain.toml`, with Rust 2024 language/style,
rustfmt, Clippy and `wasm32-unknown-unknown`. Official installer SHA256 was verified
before execution. Cargo/protobuf/build output stays under `artifacts`; no home
Rust install, shell profile change, system package manager or paid call is needed.

For a fresh checkout without Rust, from the repository root:

Before each `build-fixture.sh build`, set these required inputs to their respective
files or directory:

- `DUNGEONFLUX_BUILD_EXPECTED_IDENTITY`: required public identity record.
- `DUNGEONFLUX_BUILD_OBSERVED_IDENTITY`: observed identity record.
- `DUNGEONFLUX_BUILD_CONFIGURATION`: public configuration bytes.
- `DUNGEONFLUX_BUILD_CONTENT_MANIFEST`: resolved content manifest.
- `DUNGEONFLUX_BUILD_ASSETS_ROOT`: resolved static asset directory.

```sh
mkdir -p artifacts/tmp/toolchain artifacts/cache/cargo artifacts/cache/rustup
curl --proto '=https' --tlsv1.2 -fsS \
  https://static.rust-lang.org/rustup/dist/aarch64-apple-darwin/rustup-init \
  -o artifacts/tmp/toolchain/rustup-init
curl --proto '=https' --tlsv1.2 -fsS \
  https://static.rust-lang.org/rustup/dist/aarch64-apple-darwin/rustup-init.sha256 \
  -o artifacts/tmp/toolchain/rustup-init.sha256
(cd artifacts/tmp/toolchain && shasum -a 256 -c rustup-init.sha256)
chmod +x artifacts/tmp/toolchain/rustup-init
RUSTUP_HOME="$PWD/artifacts/cache/rustup" \
CARGO_HOME="$PWD/artifacts/cache/cargo" \
TMPDIR="$PWD/artifacts/tmp/toolchain" \
  artifacts/tmp/toolchain/rustup-init -y --no-modify-path --profile minimal \
  --default-toolchain 1.98.1 --component rustfmt,clippy --target wasm32-unknown-unknown
./development/build-fixture.sh cargo install wasm-bindgen-cli --version 0.2.129 --locked
./development/build-fixture.sh build
./development/build-fixture.sh serve
```

The installer URL shown is for the evaluated Apple Silicon host. Other native hosts
need the matching official rustup target. Protoc is supplied through pinned
`protoc-bin-vendored`; it does not need a separate system install. The CLI generator
must match wasm-bindgen 0.2.129 from the tracked Cargo lockfile. Its installed tool
transitives carry upstream yanked/future-incompatibility warnings; the application
workspace native/WASM gates passed independently. Future tool upgrades are separate
qualification work.

Visit **http://127.0.0.1:43180** and click **Run transport checks**. The server accepts
only the preview's configured synthetic loopback WebSocket Origin and holds at most four admitted
connections. These bounds/admission checks do not implement production authentication
or WSS deployment. Ctrl-C stops the owned preview. Health is `/fixture-health`;
read-only safe OTEL spans are `/fixture-telemetry`.

During isolated-worktree builds, reuse the main repository's tool/cache/output:

```sh
export DUNGEONFLUX_ARTIFACT_ROOT=/Users/earlcameron/Desktop/dungeonflux/artifacts
./development/build-fixture.sh build
./development/build-fixture.sh serve
```

The wrapper injects the source revision into both builds and displays it in the
browser/native startup; edited source carries a `-dirty` suffix. Submit/measure a
clean commit, then rebuild before running it. Generated web files are published
as a complete set; previous sets stay in owned temporary artifacts. Record native
and WASM hashes with the tested revision. Do not rebuild a frozen preview during
an evaluator's interaction.

## Verification

```sh
./development/build-fixture.sh cargo fmt --all -- --check
./development/build-fixture.sh cargo clippy --locked --workspace --all-targets -- -D warnings
./development/build-fixture.sh cargo test --locked --workspace
./development/build-fixture.sh cargo clippy --locked -p df-rpc-bridge --target wasm32-unknown-unknown -- -D warnings
./development/build-fixture.sh cargo clippy --locked -p df-tools --lib --target wasm32-unknown-unknown -- -D warnings
./development/build-fixture.sh cargo build --locked -p df-rpc-bridge --target wasm32-unknown-unknown
./development/build-fixture.sh build
```

The original three native behavior tests cover explicit rejection trailers, cancellation-owned
stream disposal, and a **256KiB+1 binary WebSocket** rejected by the actual native
WebSocket gateway. Browser interaction is mandatory beyond those tests. Its ten
checks cover all four generated modes; native response metadata; ordered DATA/OK
trailers; client and bidi half-close; error statuses/custom trailers in all four
modes; server-owner release after stream cancellation; deadline/timeout; native
rejection of an oversized decoded protobuf; and multiplexed simultaneous streams.
Run it again to inspect cleanup and a fresh connection.

Deadline evidence distinguishes the local browser timer from native terminal
status: the browser sends `grpc-timeout`, enforces a finite local receipt wait,
and drops the call when that wait expires. Native tonic may terminate that wait
as CANCELLED; the UI records the actual source and status rather than claiming a
fabricated native DEADLINE_EXCEEDED trailer. Native diagnostic fields prove timeout
metadata arrived and that cancelled/expired owners returned to baseline (only the
stats RPC itself remains while observed). This fixture has no durable accepted
jobs; later gameplay cancellation must preserve their independent lifetime.

## Established-driver compatibility patch

Real browser interaction first completed all four modes, then unmodified **h2
0.4.19 panicked during cancellation**: `NextResetExpire::set_queued` invoked
`std::time::Instant::now`, unsupported on browser wasm32. Compile checks missed it.
The narrowly vendored h2 copy replaces only two Instant imports on wasm32 with
**web-time 1.1.0**, whose browser clock is Performance.now. Native clocks and all
HTTP/2 reset-retention behavior stay unchanged. No unsafe Send/Sync is added.
Upstream source, MIT license, published crate SHA256, upstream commit and exact
patch provenance are in `crates/df-rpc-bridge/vendor/h2/PATCH.md`. The upstream copy
is excluded from authored fmt/Clippy workspace membership; the adapted imports
remain part of bridge review. Do not disable retention to evade browser clocks.

## Bounds and remaining qualification

| Boundary | Experimental bound |
| --- | --- |
| Native admitted connections / pending ingress | 4 / 4 |
| Active streams (peer SETTINGS) | 8 per connection |
| HTTP/2 frame / decoded header list | 16KiB / 16KiB |
| Receive credit: connection / stream | 1MiB / 256KiB |
| WebSocket tunnel message | 256KiB |
| Browser callback queue | 1MiB and 256 messages, close visibly on overflow |
| Browser send backlog | writes fit remaining 256KiB bufferedAmount allowance; writes ≤16KiB |
| Native byte pipes | 1MiB per direction; outbound messages ≤16KiB |
| Decoded/encoded service protobuf | 64KiB |
| Client/bidi input items | 16 |
| Complete browser check run | 15 seconds |
| Native fixture OTEL export | 512 spans; old-span loss counter visible |

h2 releases DATA credit after the preceding response body chunk is consumed.
No application compression is enabled. Subsequent [desktop pressure/resource
qualification](qualify-g02.md) adds real queue/pipe/receipt/credit snapshots, bounded
native socket write buffers, time-sliced receive work, slow-consumer/paced-media/bulk
pressure, full small-RPC/frame-gap samples, and native/browser adversarial checks.
Those local observations do not prove hostile pre-callback browser allocations or
total process/engine memory under 8MiB, or approve physical-device/network budgets.
Physical iOS/Android/desktop matrix, suspend/foreground timing, engine/process CPU
and memory, actual audio playback and action-to-view latency remain unqualified.
In particular web-time documents browser sleep-clock differences.
**G02 remains INCONCLUSIVE** and cannot authorize dependent production transport work
without the required resource/device evidence.

The native SDK exports real correlated spans with parents, duration and safe typed
attributes through `df-observe`. The browser propagates W3C traceparent in generated
RPC metadata; it has a no-op local OTEL SDK in this fixture. Native span retention
is bounded and nondurable, with explicit loss counts. SQLite logs, durable spool,
full browser upload, exporter outage/recovery and authenticated review remain **G06
pending**. Neither telemetry nor fixture counters are gameplay state. Gameplay,
PostgreSQL, auth, catalogs, provider admission and physical resource qualification
are distinct future gates.

Primary references: [Rust 1.98.1 announcement](https://blog.rust-lang.org/2026/09/03/Rust-1.98.1/),
[rustup custom installation](https://rust-lang.github.io/rustup/installation/),
[tonic generic GrpcService](https://docs.rs/tonic/0.14.6/tonic/client/trait.GrpcService.html),
[h2 driver settings](https://docs.rs/h2/0.4.19/h2/client/struct.Builder.html),
[web-time browser clock and sleep limitation](https://docs.rs/web-time/1.1.0/).
