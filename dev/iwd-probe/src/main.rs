//! Development-only probe: explicitly run on the Pi over verified Ethernet SSH.
use std::{
    io::{self, BufRead, Write},
    time::Duration,
};
use wifi_setupd::wifi::{
    Action, Adapter, Credentials, Error, Network, OperationId, Security, Snapshot,
    iwd::{AccessPointConfig, InterfaceAddresses, IwdAdapter, Options},
};

fn credentials() -> io::Result<Credentials> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut ssid = String::new();
    let mut password = String::new();
    // Input is supplied through the SSH pipe, never command arguments or a file.
    input.read_line(&mut ssid)?;
    input.read_line(&mut password)?;
    for value in [&mut ssid, &mut password] {
        if value.ends_with('\n') {
            value.pop();
        }
        if value.ends_with('\r') {
            value.pop();
        }
    }
    Ok(Credentials { ssid, password })
}

async fn apply(
    adapter: &mut impl Adapter,
    operation: OperationId,
    action: Action,
) -> Result<Snapshot, Error> {
    let result = adapter.apply(operation, action).await;
    if let Err(error) = &result {
        eprintln!("Operation failed: {error:?}");
        if let Err(cleanup) = adapter.cancel_and_reconcile(operation).await {
            eprintln!("Cleanup also failed: {cleanup:?}");
        }
    }
    result
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let interface = args
        .get(1)
        .ok_or("usage: probe INTERFACE COMMAND [SSID] [SECURITY]")?;
    let command = args.get(2).ok_or("missing command")?;
    let access_point = match command.as_str() {
        "ap" => Some(AccessPointConfig::Credentials(credentials()?)),
        "ap-profile" => Some(AccessPointConfig::Profile(
            args.get(3).ok_or("missing AP profile SSID")?.clone(),
        )),
        _ => None,
    };
    let options = Options {
        access_point,
        ..Options::default()
    };
    let connection = zbus::Connection::system().await?;
    let mut adapter = IwdAdapter::new(connection, interface.clone(), InterfaceAddresses, options)?;
    let operation = OperationId(1);
    match command.as_str() {
        "status" => println!("{:?}", adapter.snapshot().await?),
        "monitor" => {
            for _ in 0..30 {
                println!("{:?}", adapter.snapshot().await?);
                io::stdout().flush()?;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        "scan" => {
            let networks = adapter.scan().await?;
            println!("scan completed: {} networks", networks.len());
            for network in networks {
                println!("{network:?}");
            }
        }
        "known" => {
            for network in adapter.known_networks().await? {
                println!("{network:?}");
            }
        }
        "connect" | "replace" => {
            let credentials = credentials()?;
            let action = if command == "replace" {
                Action::ReplaceCredentials(credentials)
            } else {
                Action::Connect(credentials)
            };
            println!("{:?}", apply(&mut adapter, operation, action).await?);
        }
        "connect-known" | "forget" => {
            let ssid = args.get(3).ok_or("missing SSID")?.clone();
            let security = match args.get(4).map(String::as_str).unwrap_or("personal") {
                "open" => Security::Open,
                "personal" => Security::Personal,
                "enterprise" => Security::Enterprise,
                _ => return Err("security must be open, personal, or enterprise".into()),
            };
            let network = Network { ssid, security };
            let action = if command == "forget" {
                Action::Forget { network }
            } else {
                Action::ConnectKnown(network)
            };
            println!("{:?}", apply(&mut adapter, operation, action).await?);
        }
        "disconnect" => println!("{:?}", adapter.apply(operation, Action::Disconnect).await?),
        "ap" | "ap-profile" => {
            let started = apply(&mut adapter, operation, Action::StartAccessPoint).await?;
            println!("AP started: {started:?}");
            println!("The AP stays running after this command exits. Use stop-ap to stop it.");
        }
        "stop-ap" => println!(
            "{:?}",
            adapter.apply(operation, Action::StopAccessPoint).await?
        ),
        _ => return Err("unknown command".into()),
    }
    Ok(())
}
