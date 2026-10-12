use std::{error::Error, time::Duration};
use wifi_setupd::{
    provisioner::{Config, Handle},
    service::Service,
    wifi::iwd::{AccessPointConfig, InterfaceAddresses, IwdAdapter, Options},
};

const USAGE: &str = "wifi-setupd --interface NAME --ap-profile SSID --web-root PATH [--http-port 80] [--management-key-file PATH] [--startup-grace-seconds N] [--no-auto]\n\nThe AP profile must already exist in iwd's configuration. No password is\naccepted on the command line. Only Wi-Fi readiness suppresses automatic AP\nstartup; Ethernet does not. --no-auto requires an explicit D-Bus Start.";
struct Arguments {
    interface: String,
    profile: String,
    config: Config,
    web_root: std::path::PathBuf,
    port: u16,
    management_key: Option<String>,
}
fn arguments() -> Result<Option<Arguments>, Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut interface = None;
    let mut profile = None;
    let mut config = Config::default();
    let mut web_root: Option<std::path::PathBuf> = None;
    let mut port = 80;
    let mut management_key = None;
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
            "--web-root" => web_root = Some(args.next().ok_or("missing web root")?.into()),
            "--http-port" => port = args.next().ok_or("missing port")?.parse()?,
            "--management-key-file" => {
                let key = std::fs::read_to_string(args.next().ok_or("missing key file")?)?;
                if key.trim().len() < 32 {
                    return Err("management key must have at least 32 characters".into());
                }
                management_key = Some(key.trim().to_owned());
            }
            "--no-auto" => config.automatic = false,
            _ => return Err("unknown option; run wifi-setupd --help".into()),
        }
    }
    let web_root = web_root.ok_or("--web-root is required")?;
    if !web_root.join("index.html").is_file() {
        return Err("--web-root must contain the built portal index.html".into());
    }
    if port == 0 {
        return Err("--http-port must be nonzero".into());
    }
    Ok(Some(Arguments {
        interface: interface.ok_or("--interface is required; run wifi-setupd --help")?,
        profile: profile.ok_or("--ap-profile is required; run wifi-setupd --help")?,
        config,
        web_root,
        port,
        management_key,
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
        args.interface.clone(),
        InterfaceAddresses,
        Options {
            access_point: Some(AccessPointConfig::Profile(args.profile.clone())),
            ..Options::default()
        },
    )?
    .with_system_bus_reconnection();
    let handle = Handle::spawn(adapter, args.config)?;
    let http = tokio::spawn(wifi_setupd::http::serve_interface(
        handle.clone(),
        args.interface,
        wifi_setupd::http::Config {
            web_root: args.web_root,
            ap_name: args.profile,
            management_key: args.management_key,
        },
        args.port,
    ));
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
    http.abort();
    drop(service);
    connection.close().await?;
    Ok(())
}
