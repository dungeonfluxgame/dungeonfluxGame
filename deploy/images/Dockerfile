# DungeonFlux Linux images (master launch plan 12.5, 12.6; LAUNCH.md decisions 18 and 23).
# STATUS: UNVERIFIED. Written on a machine without a running Docker daemon; never built.
#
# Build (CI only, never on a production machine):  deploy/scripts/build-images.sh
#   docker buildx build --platform linux/amd64 -f deploy/images/Dockerfile --target control-plane ...
# The build context is the repository root; deploy/images/Dockerfile.dockerignore keeps secrets and
# unrelated trees (.env, config/dungeonflux.toml, artifacts, planning, development) out of it.
#
# Base images come in as build arguments that MUST carry a digest (deploy/images/images.lock). The
# arguments have no default on purpose: a build without a pinned digest fails at FROM.
#
# One binary, `df-server`, with subcommands (hosted-runtime-research.md 8.1): control-plane,
# campaign-runtime --descriptor, egress, migrate {status|plan|apply}, healthcheck, config check. It is
# configured by ONE TOML file passed with --config (the hosted binary reads no environment variables) and
# secrets are *_file paths inside the TOML. Separate image TARGETS exist so that each role ships only what it
# needs and has its own digest in the runner's allow-list (decision 18): control-plane (static client,
# content), campaign-runtime (content only), egress (nothing else: it is the only holder of provider keys).
# The trusted runner (`df-runner`) and the docker proxy are the last two targets (round 2).
#
# df-server exists (crates/df-server) but so far has only `config check`; the subcommands above are still O1's to add. It and df-runner are
# STANDALONE crates until the coordinator adds them to the workspace (each has its own [workspace] and Cargo.lock), so they are built by
# --manifest-path, which works the same once they are members. `--no-default-features` drops the `local-dev` feature ("off in production
# images", crates/df-server/Cargo.toml). TODO(O1): df-runner is a library today; the runner target needs a `df-runner` binary. TODO(O1): the media decode worker may need ffmpeg, which distroless lacks; if so it becomes
# another image (service-operations.md, "Additional security enforcement details").

ARG RUST_BASE
ARG RUNTIME_BASE
ARG EDGE_BUILDER_BASE
ARG EDGE_BASE
ARG BACKUP_BASE

# ---------------------------------------------------------------------------------------------
# toolchain: the pinned Rust (rust-toolchain.toml, 1.98.1) plus the one wasm-bindgen the client needs.
FROM ${RUST_BASE} AS toolchain
ARG WASM_BINDGEN_VERSION=0.2.129
ENV CARGO_INCREMENTAL=0 \
    CARGO_NET_RETRY=5 \
    CARGO_TERM_COLOR=never \
    RUST_BACKTRACE=1
WORKDIR /src
COPY rust-toolchain.toml ./
# rustup 1.28+ reads rust-toolchain.toml (components, the wasm32 target) exactly as ci.yml does.
RUN rustup toolchain install && rustup show active-toolchain
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install wasm-bindgen-cli --version "${WASM_BINDGEN_VERSION}" --locked

# ---------------------------------------------------------------------------------------------
# source: only what the workspace needs. --locked everywhere: Cargo.lock is the dependency pin.
FROM toolchain AS source
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY crates ./crates
COPY content ./content
COPY assets ./assets

# ---------------------------------------------------------------------------------------------
# build-native: release binary for linux/amd64 at the generic x86-64 baseline (no target-cpu=native).
FROM source AS build-native
ARG SERVER_PACKAGE=df-server
ARG SERVER_BIN=df-server
ENV RUSTFLAGS="--remap-path-prefix=/src=/build --remap-path-prefix=/usr/local/cargo=/cargo -C strip=symbols"
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release --manifest-path "crates/${SERVER_PACKAGE}/Cargo.toml" --target-dir /src/target \
        --no-default-features --bin "${SERVER_BIN}" \
    && install -D -m 0555 "target/release/${SERVER_BIN}" /out/server

# ---------------------------------------------------------------------------------------------
# build-web: the browser client (WASM) exactly as development/run-gameplay.sh builds it, in release mode.
FROM source AS build-web
ENV RUSTFLAGS="--remap-path-prefix=/src=/build --remap-path-prefix=/usr/local/cargo=/cargo"
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p df-tools --lib --target wasm32-unknown-unknown \
    && wasm-bindgen --target web --out-name df_tools --out-dir /out/web \
        target/wasm32-unknown-unknown/release/df_tools.wasm

# ---------------------------------------------------------------------------------------------
# control-plane: authentication, memberships, billing, routing, webhooks, storage authorization, the media
# gateway, and the static client. Distroless: no shell, no package manager. The image default user is 65532, but the
# run definition ALWAYS overrides it with a distinct numeric uid per service (never 65532), the role's secrets
# group as primary gid, a read-only root filesystem, and tmpfs or an owned host directory for the few writable
# paths. Also runs `migrate` (same image, other subcommand).
FROM ${RUNTIME_BASE} AS control-plane
ARG VCS_REF=unknown
ARG SOURCE_URL=unknown
ARG VERSION=0.0.0
LABEL org.opencontainers.image.title="dungeonflux-control-plane" \
      org.opencontainers.image.source="${SOURCE_URL}" \
      org.opencontainers.image.revision="${VCS_REF}" \
      org.opencontainers.image.version="${VERSION}"
COPY --from=build-native /out/server /usr/local/bin/df-server
COPY --from=build-web /out/web /srv/web
COPY assets/ui /srv/ui
COPY content /srv/content
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/df-server"]
CMD ["control-plane", "--config", "/etc/dungeonflux/control-plane.toml"]

# ---------------------------------------------------------------------------------------------
# campaign-runtime: one isolated native process per active campaign (12.5). No web client. Started only by
# the trusted runner, from a digest in its allow-list, with a read-only descriptor file mounted. It holds no
# provider, payment, identity or object-store credential (decision 18); the descriptor and the database
# role password are its only secrets.
FROM ${RUNTIME_BASE} AS campaign-runtime
ARG VCS_REF=unknown
ARG SOURCE_URL=unknown
ARG VERSION=0.0.0
LABEL org.opencontainers.image.title="dungeonflux-campaign-runtime" \
      org.opencontainers.image.source="${SOURCE_URL}" \
      org.opencontainers.image.revision="${VCS_REF}" \
      org.opencontainers.image.version="${VERSION}"
COPY --from=build-native /out/server /usr/local/bin/df-server
COPY content /srv/content
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/df-server"]
CMD ["campaign-runtime", "--descriptor", "/etc/dungeonflux/runtime.toml"]

# ---------------------------------------------------------------------------------------------
# egress: the provider dispatcher, the ONLY process that holds provider credentials (decision 18). It
# carries no content, no web client and no object-store credential. Its service definition, hardening and
# signing are finished in round 2; the staging mirror needs the image now for the network split.
FROM ${RUNTIME_BASE} AS egress
ARG VCS_REF=unknown
ARG SOURCE_URL=unknown
ARG VERSION=0.0.0
LABEL org.opencontainers.image.title="dungeonflux-egress" \
      org.opencontainers.image.source="${SOURCE_URL}" \
      org.opencontainers.image.revision="${VCS_REF}" \
      org.opencontainers.image.version="${VERSION}"
COPY --from=build-native /out/server /usr/local/bin/df-server
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/df-server"]
CMD ["egress", "--config", "/etc/dungeonflux/egress.toml"]

# ---------------------------------------------------------------------------------------------
# build-runner: the trusted runner (round 2). TODO(O1): crates/df-runner is a library today and has no binary, so this stage fails until
# O1 adds one; the binary name df-runner is a proposal.
# It is a separate binary from df-server on purpose: it holds no game, payment or provider code and talks to the host only through
# the docker proxy's unix socket (no Docker socket of its own).
FROM source AS build-runner
ARG RUNNER_PACKAGE=df-runner
ARG RUNNER_BIN=df-runner
ENV RUSTFLAGS="--remap-path-prefix=/src=/build --remap-path-prefix=/usr/local/cargo=/cargo -C strip=symbols"
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release --manifest-path "crates/${RUNNER_PACKAGE}/Cargo.toml" --target-dir /src/target \
        --bin "${RUNNER_BIN}" \
    && install -D -m 0555 "target/release/${RUNNER_BIN}" /out/runner

# runner: places campaigns and starts runtimes through the docker proxy. The service definition (deploy/compose/production.compose.yml)
# runs it as uid 10004 with NO capability, a read-only root and no network but runner-net; a root helper (df-slot-dirs) owns the slot
# directories and the runner only writes file contents below them. O1 must confirm the design.
FROM ${RUNTIME_BASE} AS runner
ARG VCS_REF=unknown
ARG SOURCE_URL=unknown
ARG VERSION=0.0.0
LABEL org.opencontainers.image.title="dungeonflux-runner" \
      org.opencontainers.image.source="${SOURCE_URL}" \
      org.opencontainers.image.revision="${VCS_REF}" \
      org.opencontainers.image.version="${VERSION}"
COPY --from=build-runner /out/runner /usr/local/bin/df-runner
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/df-runner"]
CMD ["--config", "/etc/dungeonflux/runner.toml"]

# ---------------------------------------------------------------------------------------------
# build-proxy: the body-checking Docker API proxy. Its own standalone crate (deploy/docker-proxy: std + serde_json only, its own
# Cargo.lock), NOT part of the game workspace, so a change to the game never changes what guards the Docker socket.
# The policy has unit tests: `cargo test --locked` in that directory runs them; the build runs them first.
FROM toolchain AS build-proxy
ENV RUSTFLAGS="--remap-path-prefix=/src=/build --remap-path-prefix=/usr/local/cargo=/cargo -C strip=symbols"
COPY deploy/docker-proxy ./deploy/docker-proxy
WORKDIR /src/deploy/docker-proxy
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/deploy/docker-proxy/target \
    cargo test --locked --release \
    && cargo build --locked --release \
    && install -D -m 0555 target/release/df-docker-proxy /out/docker-proxy

FROM ${RUNTIME_BASE} AS docker-proxy
ARG VCS_REF=unknown
ARG SOURCE_URL=unknown
ARG VERSION=0.0.0
LABEL org.opencontainers.image.title="dungeonflux-docker-proxy" \
      org.opencontainers.image.source="${SOURCE_URL}" \
      org.opencontainers.image.revision="${VCS_REF}" \
      org.opencontainers.image.version="${VERSION}"
COPY --from=build-proxy /out/docker-proxy /usr/local/bin/df-docker-proxy
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/df-docker-proxy"]

# ---------------------------------------------------------------------------------------------
# build-edge: Caddy with the rate-limit module, built by xcaddy at pinned versions (the module by COMMIT, never a moving tag). The
# stock image has no rate limiting; the pairing (/app/api/display/*) and join (/app/api/join/*) endpoints need a per-client limit
# at the edge. UNVERIFIED: never built; the module's directive syntax in the Caddyfiles is from its documentation, to be checked by
# `caddy validate` in CI (round 3).
FROM ${EDGE_BUILDER_BASE} AS build-edge
ARG CADDY_VERSION
ARG CADDY_RATELIMIT_REF
RUN xcaddy build "${CADDY_VERSION}" --with "github.com/mholt/caddy-ratelimit@${CADDY_RATELIMIT_REF}" --output /usr/bin/caddy

FROM ${EDGE_BASE} AS edge
ARG VCS_REF=unknown
ARG SOURCE_URL=unknown
ARG VERSION=0.0.0
LABEL org.opencontainers.image.title="dungeonflux-edge" \
      org.opencontainers.image.source="${SOURCE_URL}" \
      org.opencontainers.image.revision="${VCS_REF}" \
      org.opencontainers.image.version="${VERSION}"
COPY --from=build-edge /usr/bin/caddy /usr/bin/caddy
# COPY does not carry the file capability the stock image sets so that Caddy can bind ports 80 and 443 as a non-root user; set it again.
RUN setcap cap_net_bind_service=+ep /usr/bin/caddy

# ---------------------------------------------------------------------------------------------
# backup: the one-shot tools of the nightly backup (df-backup): pg_dump (the PostgreSQL image already has it, at the pinned version),
# age (encryption to the public recipient) and the aws CLI (S3-compatible upload). It holds no secret and runs only as a throwaway
# container started by df-backup with the credentials on stdin. UNVERIFIED (never built). The apt packages are not version-pinned:
# the base image is pinned by digest, the package versions are whatever that base's repositories serve at build time; a reproducible
# build would vendor them (recorded as a residual).
FROM ${BACKUP_BASE} AS backup
ARG VCS_REF=unknown
ARG SOURCE_URL=unknown
ARG VERSION=0.0.0
LABEL org.opencontainers.image.title="dungeonflux-backup" \
      org.opencontainers.image.source="${SOURCE_URL}" \
      org.opencontainers.image.revision="${VCS_REF}" \
      org.opencontainers.image.version="${VERSION}"
RUN apt-get update \
    && apt-get install -y --no-install-recommends age awscli ca-certificates \
    && rm -rf /var/lib/apt/lists/*
