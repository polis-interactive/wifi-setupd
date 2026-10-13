use std::{error::Error, ffi::OsString, time::Duration};
use wifi_setupd::{
    provisioner::{Config, Handle},
    service::Service,
    wifi::iwd::{AccessPointConfig, InterfaceAddresses, IwdAdapter, Options},
};

const USAGE: &str = "wifi-setupd --interface NAME --ap-profile SSID --web-root PATH [--http-port 80] [--http-hostname NAME] [--device-key-file PATH] [--startup-grace-seconds N] [--no-auto]\n\nThe AP profile must already exist in iwd's configuration. No password is\naccepted on the command line. Only Wi-Fi readiness suppresses automatic AP\nstartup; Ethernet does not. --no-auto requires an explicit D-Bus Start.\nDefault HTTP hostname: <system-host-label>.local, read at startup.\nRepeat --http-hostname to override that default with explicit DNS names.\nNames use --http-port; advertising/resolution is configured separately.";
struct Arguments {
    interface: String,
    profile: String,
    config: Config,
    web_root: std::path::PathBuf,
    port: u16,
    device_key: Option<String>,
    hostnames: Vec<wifi_setupd::http::Hostname>,
}
fn arguments() -> Result<Option<Arguments>, Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut interface = None;
    let mut profile = None;
    let mut config = Config::default();
    let mut web_root: Option<std::path::PathBuf> = None;
    let mut port = 80;
    let mut device_key = None;
    let mut hostnames = Vec::new();
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
            "--http-hostname" => {
                hostnames.push(args.next().ok_or("missing HTTP hostname")?.parse()?);
            }
            "--device-key-file" => {
                let value = std::fs::read_to_string(args.next().ok_or("missing device key file")?)?;
                let value = value.trim_end_matches(['\r', '\n']);
                if !(8..=63).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_graphic())
                {
                    return Err(
                        "device key must be 8–63 printable ASCII characters without spaces".into(),
                    );
                }
                device_key = Some(value.to_owned());
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
        device_key,
        hostnames: resolve_hostnames(hostnames, nix::unistd::gethostname)?,
    }))
}

fn resolve_hostnames(
    explicit: Vec<wifi_setupd::http::Hostname>,
    system_hostname: impl FnOnce() -> Result<OsString, nix::Error>,
) -> Result<Vec<wifi_setupd::http::Hostname>, Box<dyn Error>> {
    if !explicit.is_empty() {
        return Ok(explicit);
    }
    let hostname = system_hostname()?;
    let hostname = hostname
        .to_str()
        .ok_or("system hostname must be valid UTF-8")?;
    Ok(vec![wifi_setupd::http::Hostname::from_system_hostname(
        hostname,
    )?])
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
            device_key: args.device_key,
            hostnames: args.hostnames,
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

#[cfg(test)]
mod tests {
    use super::*;
    use wifi_setupd::http::Hostname;

    #[test]
    fn default_uses_runtime_system_host_label_with_one_local_suffix() {
        for system in ["polis-dev", "POLIS-DEV.local", "polis-dev.example.org."] {
            let names = resolve_hostnames(vec![], || Ok(system.into())).unwrap();
            assert_eq!(names, vec!["polis-dev.local".parse::<Hostname>().unwrap()]);
        }
        for system in ["", "bad name", "*.local", "192.168.1.125"] {
            assert!(resolve_hostnames(vec![], || Ok(system.into())).is_err());
        }
        assert!(resolve_hostnames(vec![], || Err(nix::errno::Errno::EIO)).is_err());
    }

    #[test]
    fn explicit_hostnames_replace_default_without_reading_system_hostname() {
        let explicit: Vec<Hostname> = ["custom.local", "setup.example.org"]
            .into_iter()
            .map(|name| name.parse().unwrap())
            .collect();
        let names = resolve_hostnames(explicit.clone(), || {
            panic!("override must skip system lookup")
        })
        .unwrap();
        assert_eq!(names, explicit);
    }
}
