use std::process::{Child, Command, Stdio};

pub struct PrivateBus {
    child: Child,
    _directory: tempfile::TempDir,
    pub address: String,
}

impl PrivateBus {
    pub async fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let address = format!("unix:path={}", directory.path().join("bus").display());
        let child = Command::new("dbus-daemon")
            .args([
                "--session",
                "--nofork",
                "--nopidfile",
                "--address",
                &address,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .expect("contract tests require dbus-daemon on PATH");
        let mut bus = Self {
            child,
            _directory: directory,
            address,
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                assert!(
                    bus.child.try_wait().unwrap().is_none(),
                    "private bus exited"
                );
                if zbus::connection::Builder::address(bus.address.as_str())
                    .unwrap()
                    .build()
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("private bus did not become ready within 5 seconds");
        bus
    }

    pub async fn connect(&self) -> zbus::Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap()
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
