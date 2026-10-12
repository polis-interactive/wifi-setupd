use wifi_setupd_api::{ControlError, LinkState, Status, WifiSetupProxy};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let connection = zbus::Connection::system().await?;
    let wifi = WifiSetupProxy::new(&connection).await?;
    let status: Status = wifi.status().await?;
    if status.link != LinkState::Ready {
        let accepted: Result<(), ControlError> = wifi.start_provisioning().await;
        accepted?;
    }
    let _updates = wifi.receive_status_changed().await;
    let _completion = wifi.receive_provisioning_completed().await?;
    Ok(())
}
