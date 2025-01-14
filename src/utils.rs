use iroh::Endpoint;

pub async fn wait_for_relay(endpoint: &Endpoint) -> anyhow::Result<()> {
    while endpoint.home_relay().get().is_err() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Ok(())
}