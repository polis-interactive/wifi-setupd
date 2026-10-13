use super::*;
fn fixture() -> Radio {
    let mut config = Simulation::parse(include_str!("../example.toml")).unwrap();
    config.networks.extend(
        Simulation::parse(
            r#"
[[networks]]
ssid = "Open guest network"
password = ""
[[networks]]
ssid = "Old network"
password = "old-network-password"
saved = true
visible = false
"#,
        )
        .unwrap()
        .networks,
    );
    for network in &mut config.networks {
        network.connect_delay_ms = 0;
    }
    Radio::new(config)
}
fn credentials(ssid: &str, password: &str) -> Credentials {
    Credentials {
        ssid: ssid.into(),
        password: password.into(),
    }
}
#[tokio::test]
async fn only_configured_visible_pairs_are_accepted() {
    let mut radio = fixture();
    assert_eq!(
        radio
            .connect(credentials("Home Wi-Fi", "wrong-password"))
            .await,
        Err(Error::Authentication)
    );
    assert_eq!(
        radio
            .connect(credentials("Missing", "working-password"))
            .await,
        Err(Error::NetworkUnavailable)
    );
    assert_eq!(
        radio
            .connect(credentials("Old network", "old-network-password"))
            .await,
        Err(Error::NetworkUnavailable)
    );
    radio
        .connect(credentials("Home Wi-Fi", "working-password"))
        .await
        .unwrap();
    assert_eq!(radio.snapshot.link, LinkState::Ready);
    assert!(
        radio
            .known_networks()
            .await
            .unwrap()
            .iter()
            .any(|n| n.network.ssid == "Home Wi-Fi")
    );
    radio
        .connect(credentials("Open guest network", ""))
        .await
        .unwrap();
    assert_eq!(radio.connected.unwrap().security, Security::Open);
}
#[tokio::test]
async fn replacement_and_scoped_deletion_match_the_ui_workflow() {
    let mut radio = fixture();
    let id = OperationId(1);
    radio
        .connect(credentials("Home Wi-Fi", "working-password"))
        .await
        .unwrap();
    assert_eq!(
        radio
            .apply(
                id,
                Action::Connect(credentials("Home Wi-Fi", "working-password"))
            )
            .await,
        Err(Error::CredentialConflict)
    );
    assert_eq!(
        radio
            .apply(
                id,
                Action::ReplaceCredentials(credentials("Home Wi-Fi", "wrong-password"))
            )
            .await,
        Err(Error::Authentication)
    );
    assert!(!radio.saved.iter().any(|n| n.ssid == "Home Wi-Fi"));
    assert!(radio.saved.iter().any(|n| n.ssid == "Old network"));
    radio.cancel_and_reconcile(id).await.unwrap();
    radio
        .connect(credentials("Home Wi-Fi", "working-password"))
        .await
        .unwrap();
    radio
        .apply(
            id,
            Action::Forget {
                network: network("Old network", "old-network-password"),
            },
        )
        .await
        .unwrap();
    assert_eq!(radio.snapshot.link, LinkState::Ready);
    radio
        .apply(
            id,
            Action::Forget {
                network: network("Home Wi-Fi", "working-password"),
            },
        )
        .await
        .unwrap();
    assert_eq!(radio.snapshot.link, LinkState::Disconnected);
}
#[tokio::test]
async fn fixture_controls_discovery_saved_networks_and_initial_view() {
    let mut config = Simulation::parse(include_str!("../example.toml")).unwrap();
    config.initially_connected = Some("Workshop".into());
    let mut radio = Radio::new(config);
    assert_eq!(radio.snapshot.link, LinkState::Ready);
    let scan = radio.scan().await.unwrap();
    assert_eq!(scan.len(), 2);
    assert!(!scan.iter().any(|n| n.ssid == "Old network"));
    assert_eq!(radio.known_networks().await.unwrap().len(), 1);
}
#[test]
fn rejects_invalid_fixtures_without_echoing_passwords() {
    for input in [
        "port = 0",
        "connect_delay_ms = 10", // Global delay is no longer supported.
        "[[networks]]\nssid = 'Home'\npassword = ''\nconnect_delay_ms = 60001",
        "initially_connected = 'Missing'",
        "typo = true",
        "[[networks]]\nssid = 'Home'\npassword = 'secret'",
        "[[networks]]\nssid = 'Home'\npassword = ''\n[[networks]]\nssid = 'Home'\npassword = ''",
    ] {
        let error = Simulation::parse(input).err().expect("invalid fixture");
        assert!(!error.contains("secret"));
    }
}

#[tokio::test(start_paused = true)]
async fn connection_attempts_use_the_selected_network_delay() {
    let mut radio = Radio::new(Simulation::parse(include_str!("../example.toml")).unwrap());
    for (ssid, password, delay) in [
        ("Home Wi-Fi", "working-password", 2000),
        ("Workshop", "workshop-password", 5000),
        ("Home Wi-Fi", "wrong-password", 2000),
    ] {
        let started = tokio::time::Instant::now();
        let result = radio.connect(credentials(ssid, password)).await;
        assert_eq!(
            result,
            if password == "wrong-password" {
                Err(Error::Authentication)
            } else {
                Ok(())
            }
        );
        assert_eq!(started.elapsed(), Duration::from_millis(delay));
    }
}
