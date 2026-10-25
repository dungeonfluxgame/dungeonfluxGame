//! A loopback-only preview of the portal for browser checks: the memory store, the development
//! outbox and the dev views, on `127.0.0.1:<port>`. Not the server composition (that is the
//! gameplay server's, behind `DF_PORTAL=on`); nothing here persists or sends mail.
//!
//! `cargo run -p df-portal --example portal_preview -- <port> <outbox directory>`

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use df_customer::{
        BuiltInCommonPasswords, CustomerConfig, CustomerService, DevOutbox, HashingParameters,
        HostConfig, IdentitySource, SystemClock, Tunables, fakes::MemoryAccountStore,
    };
    use df_portal::{PortalOrigin, PortalState, dev_routes, router};

    let mut arguments = std::env::args().skip(1);
    let port: u16 = arguments
        .next()
        .and_then(|port| port.parse().ok())
        .ok_or_else(|| std::io::Error::other("usage: portal_preview <port> <outbox directory>"))?;
    let outbox_directory = arguments
        .next()
        .ok_or_else(|| std::io::Error::other("usage: portal_preview <port> <outbox directory>"))?;
    if port == 63309 {
        return Err(std::io::Error::other("port 63309 belongs to the game"));
    }
    let origin = format!("http://127.0.0.1:{port}");
    let outbox = DevOutbox::open(outbox_directory.into()).await?;
    let (service, tasks) = CustomerService::start(
        CustomerConfig {
            hosts: HostConfig::local(&origin)
                .map_err(|error| std::io::Error::other(format!("{error:?}")))?,
            identity: IdentitySource::local_passwords(),
            tunables: Tunables::default(),
            hashing: HashingParameters::PRODUCTION,
            maximum_concurrent_hashes: 4,
            hash_permit_wait: Duration::from_secs(5),
            provider_signup_floor: Duration::ZERO,
            dev_allow_local_passwords_on_https: false,
        },
        MemoryAccountStore::default(),
        Arc::clone(&outbox),
        Arc::new(SystemClock),
        Arc::new(BuiltInCommonPasswords),
    )
    .await
    .map_err(|error| std::io::Error::other(format!("{error:?}")))?;
    let origin = PortalOrigin::parse(&origin)
        .map_err(|error| std::io::Error::other(format!("{error:?}")))?;
    let state = PortalState::new(service, origin);
    let app = router(state.clone()).merge(dev_routes(state, Some(outbox)));
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
    println!("portal preview on http://127.0.0.1:{port}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _signal = tokio::signal::ctrl_c().await;
    })
    .await?;
    tasks.shutdown().await;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
