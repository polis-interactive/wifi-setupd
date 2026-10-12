//! Production D-Bus exporter. The actor owns policy; this interface holds only
//! a published snapshot. Export can be reattached after a bus reconnection.
use crate::{
    provisioner::{CommandError, Handle},
    wifi::Adapter,
};
use tokio::{sync::broadcast, task::JoinHandle};
use wifi_setupd_zbus_api::{
    BUS_NAME, BackendState, ControlError, LinkState, OBJECT_PATH, ProvisioningError,
    ProvisioningState, Status,
};
use zbus::{Connection, object_server::SignalEmitter};

struct Interface<A: Adapter + 'static> {
    handle: Handle<A>,
    status: Status,
}
fn control(error: CommandError) -> ControlError {
    match error {
        CommandError::Busy => ControlError::Busy("Radio transition is in progress".into()),
        CommandError::BackendUnavailable => {
            ControlError::BackendUnavailable("Wi-Fi backend unavailable".into())
        }
        CommandError::InvalidInput | CommandError::Closed => {
            ControlError::Internal("Unable to accept command".into())
        }
    }
}
#[zbus::interface(name = "tv.polis.WifiSetup1")]
impl<A: Adapter + 'static> Interface<A> {
    async fn start_provisioning(&self) -> Result<(), ControlError> {
        self.handle.start().await.map_err(control)
    }
    async fn stop_provisioning(&self) -> Result<(), ControlError> {
        self.handle.stop().await.map_err(control)
    }
    #[zbus(property)]
    fn state(&self) -> ProvisioningState {
        self.status.state
    }
    #[zbus(property)]
    fn provisioning_active(&self) -> bool {
        self.status.provisioning_active()
    }
    #[zbus(property)]
    fn last_error(&self) -> ProvisioningError {
        self.status.last_error
    }
    #[zbus(property)]
    fn link_state(&self) -> LinkState {
        self.status.link
    }
    #[zbus(property)]
    fn backend_state(&self) -> BackendState {
        self.status.backend
    }
    #[zbus(property)]
    fn status(&self) -> Status {
        self.status
    }
    #[zbus(signal)]
    async fn provisioning_completed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
impl<A: Adapter + 'static> Interface<A> {
    async fn publish(&self, emitter: &SignalEmitter<'_>) -> zbus::Result<()> {
        self.status_changed(emitter).await?;
        self.state_changed(emitter).await?;
        self.provisioning_active_changed(emitter).await?;
        self.link_state_changed(emitter).await?;
        self.backend_state_changed(emitter).await?;
        self.last_error_changed(emitter).await
    }
}
/// Drop aborts only publication forwarding, not the actor or radio. Use the
/// handle's explicit stop/shutdown methods when radio cleanup is intended.
pub struct Service {
    task: JoinHandle<zbus::Result<()>>,
}
impl Drop for Service {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Service {
    pub async fn attach<A: Adapter + 'static>(
        connection: Connection,
        handle: Handle<A>,
    ) -> zbus::Result<Self> {
        let mut updates = handle.publications();
        let checkpoint = handle
            .checkpoint()
            .await
            .map_err(|_| zbus::Error::Failure("Provisioner stopped".into()))?;
        connection
            .object_server()
            .at(
                OBJECT_PATH,
                Interface {
                    status: checkpoint.status,
                    handle: handle.clone(),
                },
            )
            .await?;
        connection.request_name(BUS_NAME).await?;
        let interface = connection
            .object_server()
            .interface::<_, Interface<A>>(OBJECT_PATH)
            .await?;
        let task = tokio::spawn(async move {
            let mut revision = checkpoint.revision;
            loop {
                let (status, completed) = match updates.recv().await {
                    Ok(update) => {
                        if update.revision <= revision {
                            continue;
                        }
                        revision = update.revision;
                        (update.status, update.completed)
                    }
                    // A signal is transient. Never invent/replay completions
                    // after lag; recover a fresh coherent property snapshot.
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        updates = handle.publications();
                        let checkpoint = handle
                            .checkpoint()
                            .await
                            .map_err(|_| zbus::Error::Failure("Provisioner stopped".into()))?;
                        revision = checkpoint.revision;
                        (checkpoint.status, false)
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                };
                let mut service = interface.get_mut().await;
                service.status = status;
                service.publish(interface.signal_emitter()).await?;
                if completed {
                    Interface::<A>::provisioning_completed(interface.signal_emitter()).await?;
                }
            }
        });
        Ok(Self { task })
    }
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}
