use iroh::Endpoint;

/// Waits for the endpoint to establish a connection with its home relay.
///
/// # Arguments
///
/// * `endpoint` - An Iroh endpoint to monitor for relay connection
///
/// # Returns
///
/// * `Ok(())` when relay connection is established
/// * `Err` propagates any errors that occur during the process
pub async fn wait_for_relay(endpoint: &Endpoint) -> anyhow::Result<()> {
    while endpoint.home_relay().get().is_err() {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Ok(())
}