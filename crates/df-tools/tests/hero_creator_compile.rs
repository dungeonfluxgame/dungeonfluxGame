//! The browser surface compiles as part of the library; this checks wasm test linkage.
#![cfg(target_arch = "wasm32")]

const _: &str = df_tools::BUILD_ID;
