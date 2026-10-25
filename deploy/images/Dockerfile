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
# The trusted runner (`df-runner`, its own binary) is a later target (round 2).
#
# TODO(O1): the df-server crate and these subcommands do not exist yet; SERVER_PACKAGE/SERVER_BIN are the
# proposed names. TODO(O1): the media decode worker may need ffmpeg, which distroless lacks; if so it becomes
# another image (service-operations.md, "Additional security enforcement details").

ARG RUST_BASE
ARG RUNTIME_BASE

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
    cargo build --locked --release -p "${SERVER_PACKAGE}" --bin "${SERVER_BIN}" \
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
CMD ["campaign-runtime", "--descriptor", "/etc/dungeonflux/descriptor.toml"]

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
