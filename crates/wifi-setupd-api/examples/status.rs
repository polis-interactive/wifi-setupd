//! A consumer supplies its own bus connection. Pass an explicit private-bus
//! address as the first argument for local use; otherwise use the system bus.
use wifi_setupd_api::WifiSetupProxy;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let connection = match std::env::args().nth(1) {
        Some(address) => {
            zbus::connection::Builder::address(address.as_str())?
                .build()
                .await?
        }
        None => zbus::Connection::system().await?,
    };
    let wifi = WifiSetupProxy::new(&connection).await?;
    println!("{:?}", wifi.status().await?);
    Ok(())
}
