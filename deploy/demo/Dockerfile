# BuildKit builds this release for linux/amd64 in demo-release.yml.
FROM rust:1.98.1-bookworm AS build

WORKDIR /src
# The repository .cargo/config.toml points target-dir at artifacts/; release paths below expect /src/target.
ENV CARGO_TARGET_DIR=/src/target
RUN rustup target add wasm32-unknown-unknown

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/usr/local/cargo/wasm-bindgen-target \
    CARGO_TARGET_DIR=/usr/local/cargo/wasm-bindgen-target \
      cargo install --locked --version 0.2.129 wasm-bindgen-cli

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release --no-default-features -p df-tools --bin df-transport-fixture && \
    apt-get update && apt-get install -y --no-install-recommends binutils && \
    deploy/profiles/check-profile.sh demo deploy/profiles/demo.env /src/target/release/df-transport-fixture && \
    mkdir -p /opt/dungeonflux/bin && \
    cp /src/target/release/df-transport-fixture /opt/dungeonflux/bin/ && \
    cargo build --locked --release -p df-tools --lib --target wasm32-unknown-unknown && \
    mkdir -p /opt/dungeonflux/web && \
    wasm-bindgen --target web --out-name df_tools --out-dir /opt/dungeonflux/web \
      /src/target/wasm32-unknown-unknown/release/df_tools.wasm

ENV CARGO_PROFILE_RELEASE_OPT_LEVEL=z \
    CARGO_PROFILE_RELEASE_LTO=true \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
    CARGO_PROFILE_RELEASE_PANIC=abort
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p df-account-app --lib --target wasm32-unknown-unknown && \
    mkdir -p /opt/dungeonflux/app/assets && \
    wasm-bindgen --target web --no-typescript --out-name account_app \
      --out-dir /opt/dungeonflux/app/assets \
      /src/target/wasm32-unknown-unknown/release/df_account_app.wasm && \
    cat crates/df-account-app/web/css/base.css \
      crates/df-account-app/web/css/customer.css \
      crates/df-account-app/web/css/campaign.css \
      crates/df-account-app/web/css/admin.css > /opt/dungeonflux/app/assets/app.css && \
    printf 'import init from "/app/assets/account_app.js";\ninit({ module_or_path: "/app/assets/account_app_bg.wasm" });\n' \
      > /opt/dungeonflux/app/assets/boot.js && \
    cp crates/df-account-app/web/index.html /opt/dungeonflux/app/index.html && \
    cp docs/assets/establishing-harbor-canal-rowboat-bridge.webp /opt/dungeonflux/app/assets/landing.webp && \
    test "$(stat -c %s /opt/dungeonflux/app/assets/account_app_bg.wasm)" -le 716800

RUN mkdir -p /opt/dungeonflux/site /opt/dungeonflux/assets && \
    cp -a docs/. /opt/dungeonflux/site/ && \
    cp -a assets/ui /opt/dungeonflux/assets/ui && \
    if [ -d assets/music ]; then cp -a assets/music /opt/dungeonflux/assets/music; fi

FROM scratch AS web-export
COPY --from=build /opt/dungeonflux/site /site
COPY --from=build /opt/dungeonflux/app /app

FROM debian:bookworm-slim AS runtime
ENV MALLOC_ARENA_MAX=2 \
    DF_GAMEPLAY_LISTEN=0.0.0.0:63309 \
    DF_GAMEPLAY_PUBLIC_URL=https://dungeonfluxdnd.com \
    DF_GAMEPLAY_ALLOWED_ORIGINS=https://dungeonfluxdnd.com,https://www.dungeonfluxdnd.com \
    DF_GAMEPLAY_DATABASE_PORT=5432 \
    DF_GAMEPLAY_PACK_DIR=/opt/dungeonflux/content/harbor-journey \
    DF_GAMEPLAY_ADVENTURE_TABLES=/opt/dungeonflux/content/adventure-tables \
    DF_GAMEPLAY_VOICE_CASTING=/opt/dungeonflux/content/voice-casting.json \
    DF_GAMEPLAY_LOAD_DOTENV=off \
    DF_GAMEPLAY_HOME_VIDEO_CACHE=/tmp/home-video \
    DF_GAMEPLAY_MUSIC_CACHE=/tmp/music \
    DF_GAMEPLAY_VOICE_CACHE=/tmp/voice \
    DF_GAMEPLAY_TEXT_CACHE=/tmp/text

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd --system --gid 10001 dungeonflux && \
    useradd --system --uid 10001 --gid dungeonflux --no-create-home \
      --shell /usr/sbin/nologin dungeonflux && \
    install -d -o dungeonflux -g dungeonflux /opt/dungeonflux /tmp

COPY --from=build --chown=10001:10001 /opt/dungeonflux/bin/df-transport-fixture /usr/local/bin/df-transport-fixture
COPY --from=build /src/deploy/profiles/check-profile.sh /usr/local/bin/check-profile.sh
COPY --from=build --chown=10001:10001 /opt/dungeonflux/web /opt/dungeonflux/web
COPY --from=build --chown=10001:10001 /opt/dungeonflux/assets /opt/dungeonflux/assets
COPY --from=build --chown=10001:10001 /opt/dungeonflux/site /opt/dungeonflux/site
COPY --from=build --chown=10001:10001 /opt/dungeonflux/app /opt/dungeonflux/app
COPY --from=build --chown=10001:10001 /src/content /opt/dungeonflux/content

USER 10001:10001
WORKDIR /opt/dungeonflux
EXPOSE 63309
HEALTHCHECK --interval=30s --timeout=5s --start-period=90s --retries=3 \
  CMD ["/usr/bin/curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:63309/gameplay"]
ENTRYPOINT ["/usr/local/bin/df-transport-fixture", "--gameplay-demo", "/opt/dungeonflux/web", "/opt/dungeonflux/assets/ui"]
