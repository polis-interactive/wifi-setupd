use std::sync::{Arc, Mutex};
use zbus::{message::Header, zvariant::OwnedObjectPath};

#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "net.connman.iwd.Agent.Error")]
pub enum AgentError {
    Canceled(String),
}

// No Debug: callback state contains a short-lived password.
pub struct Secret {
    pub owner: String,
    pub network: OwnedObjectPath,
    pub password: String,
}

#[derive(Clone, Default)]
pub struct Agent(pub Arc<Mutex<Option<Secret>>>);

/// Clear credentials even if the Connect future is dropped or times out.
pub struct SecretGuard(pub Agent);
impl Drop for SecretGuard {
    fn drop(&mut self) {
        self.0.clear();
    }
}

impl Agent {
    pub fn clear(&self) {
        if let Ok(mut secret) = self.0.lock() {
            *secret = None;
        }
    }
    fn cancel_for(&self, header: Header<'_>) {
        if let Ok(mut secret) = self.0.lock()
            && secret.as_ref().is_some_and(|s| {
                header
                    .sender()
                    .is_some_and(|sender| sender.as_str() == s.owner)
            })
        {
            *secret = None;
        }
    }
}

#[zbus::interface(name = "net.connman.iwd.Agent")]
impl Agent {
    fn request_passphrase(
        &self,
        network: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<String, AgentError> {
        let mut slot = self
            .0
            .lock()
            .map_err(|_| AgentError::Canceled("No pending request".into()))?;
        let matches = slot.as_ref().is_some_and(|secret| {
            network == secret.network
                && header
                    .sender()
                    .is_some_and(|sender| sender.as_str() == secret.owner)
        });
        if !matches {
            return Err(AgentError::Canceled("No matching request".into()));
        }
        // One callback per attempt. A retry needs new user input.
        Ok(slot.take().expect("checked secret").password)
    }
    fn cancel(&self, _reason: &str, #[zbus(header)] header: Header<'_>) {
        self.cancel_for(header);
    }
    fn release(&self, #[zbus(header)] header: Header<'_>) {
        self.cancel_for(header);
    }
}
