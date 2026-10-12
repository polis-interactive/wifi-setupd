//! The provisioner owns the sender; readers receive watch snapshots.
use tokio::sync::watch;
use wifi_setupd_zbus_api::Status;

pub fn channel(initial: Status) -> (watch::Sender<Status>, watch::Receiver<Status>) {
    watch::channel(initial)
}
