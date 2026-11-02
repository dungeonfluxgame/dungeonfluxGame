//! The native half of the machine round trip (plan O.16 W1): serves the page, the WebAssembly
//! client of the `machine_browser` example and the `/tunnel` WebSocket that feeds the sample
//! machine service. It is a loopback example, not a game server: it binds an ephemeral port on
//! 127.0.0.1, checks no origin and keeps nothing.
//!
//! Usage: `machine_bridge_server <directory>` where the directory holds `machine_browser.js` and
//! `machine_browser_bg.wasm` made by `wasm-bindgen` (see the `machine_browser` example). It prints
//! the address to open and runs until it is stopped.

#[cfg(not(target_arch = "wasm32"))]
#[path = "../tests/machine_support/server.rs"]
mod server;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::{error::Error, num::NonZeroUsize, path::PathBuf};

    use axum::{extract::Path, routing::get};
    use bytes::Bytes;
    use df_rpc_bridge::NativeIncoming;

    use super::server::{SampleMachineService, serve_grpc, tunnel_router};

    /// How many tunnels may wait to be taken by the gRPC server.
    const PENDING_TUNNELS: usize = 4;
    const PAGE: &str = "<!doctype html><meta charset=\"utf-8\">\
<title>Machine bridge round trip</title>\
<h1>Machine view and intent over the bridge</h1>\
<pre id=\"results\">loading</pre>\
<script type=\"module\">import init from '/pkg/machine_browser.js'; init();</script>";

    fn served(content_type: &'static str, bytes: Bytes) -> axum::routing::MethodRouter {
        get(move || {
            let bytes = bytes.clone();
            async move {
                (
                    [
                        ("content-type", content_type),
                        ("cache-control", "no-store"),
                    ],
                    bytes,
                )
            }
        })
    }

    pub async fn run() -> Result<(), Box<dyn Error>> {
        let directory: PathBuf = std::env::args_os()
            .nth(1)
            .ok_or("usage: machine_bridge_server <directory with the wasm-bindgen output>")?
            .into();
        let script = Bytes::from(std::fs::read(directory.join("machine_browser.js"))?);
        let module = Bytes::from(std::fs::read(directory.join("machine_browser_bg.wasm"))?);
        let service = SampleMachineService::new();
        let (admission, incoming) = NativeIncoming::bounded(
            NonZeroUsize::new(PENDING_TUNNELS).ok_or("no pending tunnels allowed")?,
        )?;
        let router = tunnel_router(admission)
            .route(
                "/report/{result}",
                get(|Path(result): Path<String>| async move {
                    match result.as_str() {
                        "pass" | "fail" => {
                            println!("machine bridge report: {result}");
                            axum::http::StatusCode::NO_CONTENT
                        }
                        _ => axum::http::StatusCode::BAD_REQUEST,
                    }
                }),
            )
            .route(
                "/",
                served("text/html", Bytes::from_static(PAGE.as_bytes())),
            )
            .route("/pkg/machine_browser.js", served("text/javascript", script))
            .route(
                "/pkg/machine_browser_bg.wasm",
                served("application/wasm", module),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        println!("machine bridge ready at http://{}", listener.local_addr()?);
        tokio::select! {
            result = serve_grpc(incoming, service) => result.map_err(Into::into),
            result = axum::serve(listener, router) => result.map_err(Into::into),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    native::run().await
}

#[cfg(target_arch = "wasm32")]
fn main() {}
