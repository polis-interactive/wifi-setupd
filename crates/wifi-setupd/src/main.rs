use std::{error::Error, time::Duration};
use wifi_setupd::{
    provisioner::{Config, Handle},
    service::Service,
    wifi::iwd::{AccessPointConfig, InterfaceAddresses, IwdAdapter, Options},
};

const USAGE: &str = "wifi-setupd --interface NAME --ap-profile SSID [--startup-grace-seconds N] [--no-auto]\n\nThe AP profile must already exist in iwd's configuration. No password is\naccepted on the command line. Only Wi-Fi readiness suppresses automatic AP\nstartup; Ethernet does not. --no-auto requires an explicit D-Bus Start.";
struct Arguments {
    interface: String,
    profile: String,
    config: Config,
}
fn arguments() -> Result<Option<Arguments>, Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut interface = None;
    let mut profile = None;
    let mut config = Config::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "--interface" => interface = Some(args.next().ok_or("--interface requires a name")?),
            "--ap-profile" => {
                profile = Some(
                    args.next()
                        .ok_or("--ap-profile requires an iwd profile SSID")?,
                )
            }
            "--startup-grace-seconds" => {
                config.startup_grace =
                    Duration::from_secs(args.next().ok_or("missing startup grace")?.parse()?)
            }
            "--no-auto" => config.automatic = false,
            _ => return Err("unknown option; run wifi-setupd --help".into()),
        }
    }
    Ok(Some(Arguments {
        interface: interface.ok_or("--interface is required; run wifi-setupd --help")?,
        profile: profile.ok_or("--ap-profile is required; run wifi-setupd --help")?,
        config,
    }))
}
async fn connect() -> zbus::Connection {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), zbus::Connection::system()).await {
            Ok(Ok(connection)) => return connection,
            _ => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let Some(args) = arguments()? else {
        return Ok(());
    };
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut connection = tokio::select! {
        connection = connect() => connection,
        _ = term.recv() => return Ok(()),
        _ = interrupt.recv() => return Ok(()),
    };
    let adapter = IwdAdapter::new(
        connection.clone(),
        args.interface,
        InterfaceAddresses,
        Options {
            access_point: Some(AccessPointConfig::Profile(args.profile)),
            ..Options::default()
        },
    )?
    .with_system_bus_reconnection();
    let handle = Handle::spawn(adapter, args.config)?;
    let mut service = Service::attach(connection.clone(), handle.clone()).await?;
    eprintln!("wifi-setupd: D-Bus service ready");
    loop {
        tokio::select! {
            _ = term.recv() => break,
            _ = interrupt.recv() => break,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                if connection.is_closed() || service.is_finished() {
                    drop(service);
                    let _ = connection.close().await;
                    connection = tokio::select! {
                        connection = connect() => connection,
                        _ = term.recv() => { handle.shutdown().await?; return Ok(()); },
                        _ = interrupt.recv() => { handle.shutdown().await?; return Ok(()); },
                    };
                    service = Service::attach(connection.clone(), handle.clone()).await?;
                }
            }
        }
    }
    handle.shutdown().await?;
    drop(service);
    connection.close().await?;
    Ok(())
}
