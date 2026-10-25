//! Loopback account service for a trusted HTTPS reverse proxy.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let config = df_hosted::config::Config::from_env().map_err(|error| {
        std::io::Error::other(format!("hosted configuration refused: {error:?}"))
    })?;
    let hosted = df_hosted::Hosted::open(config)
        .await
        .map_err(|error| std::io::Error::other(format!("hosted startup refused: {error:?}")))?;
    hosted.serve().await
}

#[cfg(target_arch = "wasm32")]
fn main() {}
