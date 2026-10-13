//! Direct iwd 3.12 adapter. Every observation rediscovers the named interface
//! from ObjectManager; no object path or property cache survives a restart.
//! Calls are pinned to a unique bus owner, so they cannot replay on a new iwd.
//! Construction and observation never change auto-connect or radio state.
mod address;
mod agent;
mod proxies;

use super::{
    Action, Adapter, Credentials, Error, KnownNetwork, Network, OperationId, Security, Snapshot,
};
pub use address::{AddressSource, InterfaceAddresses};
use agent::{Agent, Secret, SecretGuard};
use proxies::*;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use wifi_setupd_zbus_api::{BackendState, LinkState};
use zbus::{
    Connection,
    fdo::{DBusProxy, ManagedObjects, ObjectManagerProxy},
    zvariant::{OwnedObjectPath, OwnedValue},
};

/// AP secrets/configuration may be supplied by the image through iwd's own
/// profile mechanism. The adapter never reads or writes those files.
pub enum AccessPointConfig {
    Credentials(Credentials),
    Profile(String),
}

pub struct Options {
    pub operation_timeout: Duration,
    pub poll_interval: Duration,
    /// AP configuration only; submitted station credentials are never retained here.
    pub access_point: Option<AccessPointConfig>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            operation_timeout: Duration::from_secs(60),
            poll_interval: Duration::from_millis(250),
            access_point: None,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Identity {
    owner: String,
    device: OwnedObjectPath,
}
struct Frame {
    identity: Identity,
    objects: ManagedObjects,
}
impl Frame {
    fn properties(&self, interface: &str) -> Result<&HashMap<String, OwnedValue>, Error> {
        self.objects
            .get(&self.identity.device)
            .and_then(|interfaces| interfaces.get(interface))
            .ok_or(Error::Unavailable)
    }
    fn mode(&self) -> Result<&str, Error> {
        string(self.properties(DEVICE)?, "Mode")
    }
}
#[derive(Clone)]
struct Pending {
    operation: OperationId,
    identity: Identity,
    changes_radio: bool,
}

pub struct IwdAdapter<A = InterfaceAddresses> {
    connection: Connection,
    reconnect_system_bus: bool,
    interface: String,
    addresses: A,
    options: Options,
    agent: Agent,
    agent_path: OwnedObjectPath,
    pending: Option<Pending>,
    // These calls have no cancellation method. Keep their replies alive when
    // the caller drops a future, so reconciliation cannot race a late mutation.
    pending_call: tokio::sync::Mutex<Option<tokio::task::JoinHandle<Result<(), Error>>>>,
}

impl<A: AddressSource> IwdAdapter<A> {
    /// Only production opts into reconnecting the system bus. Injected private
    /// connections never fall back to the host bus.
    pub fn with_system_bus_reconnection(mut self) -> Self {
        self.reconnect_system_bus = true;
        self
    }

    async fn ensure_connection(&mut self) -> Result<(), Error> {
        if self.connection.is_closed() && self.reconnect_system_bus {
            self.connection = bounded(Duration::from_secs(3), async {
                Connection::system().await.map_err(|_| Error::Unavailable)
            })
            .await?;
        }
        Ok(())
    }

    pub fn new(
        connection: Connection,
        interface: String,
        addresses: A,
        options: Options,
    ) -> Result<Self, Error> {
        if interface.is_empty()
            || interface.len() > 15
            || interface.contains('/')
            || options.operation_timeout.is_zero()
            || options.poll_interval.is_zero()
        {
            return Err(Error::InvalidInput);
        }
        match &options.access_point {
            Some(AccessPointConfig::Credentials(credentials)) => validate(credentials, true)?,
            Some(AccessPointConfig::Profile(ssid))
                if ssid.is_empty() || ssid.len() > 32 || ssid.contains(['/', '\0', '\n', '\r']) =>
            {
                return Err(Error::InvalidInput);
            }
            _ => {}
        }
        static NEXT_AGENT: AtomicU64 = AtomicU64::new(1);
        let agent_path = format!(
            "/tv/polis/WifiSetup1/Agent{}",
            NEXT_AGENT.fetch_add(1, Ordering::Relaxed)
        )
        .try_into()
        .map_err(|_| Error::Internal)?;
        Ok(Self {
            connection,
            reconnect_system_bus: false,
            interface,
            addresses,
            options,
            agent: Agent::default(),
            agent_path,
            pending: None,
            pending_call: tokio::sync::Mutex::new(None),
        })
    }

    async fn owner(&self) -> Result<String, Error> {
        let dbus = DBusProxy::new(&self.connection).await.map_err(map_error)?;
        dbus.get_name_owner(SERVICE.try_into().map_err(|_| Error::Internal)?)
            .await
            .map(|owner| owner.to_string())
            .map_err(|e| map_error(e.into()))
    }

    async fn frame(&self) -> Result<Frame, Error> {
        let owner = self.owner().await?;
        let manager = ObjectManagerProxy::builder(&self.connection)
            .destination(owner.as_str())
            .map_err(map_error)?
            .path("/")
            .map_err(map_error)?
            .build()
            .await
            .map_err(map_error)?;
        let objects = manager
            .get_managed_objects()
            .await
            .map_err(|e| map_error(e.into()))?;
        let mut devices = objects.iter().filter_map(|(path, interfaces)| {
            let properties = interfaces.get(DEVICE)?;
            (string(properties, "Name").ok()? == self.interface).then_some(path.clone())
        });
        let device = devices.next().ok_or(Error::Unavailable)?;
        if devices.next().is_some() {
            return Err(Error::Internal);
        }
        if self.owner().await? != owner {
            return Err(Error::Unavailable);
        }
        Ok(Frame {
            identity: Identity { owner, device },
            objects,
        })
    }

    async fn same_frame(&self, identity: &Identity) -> Result<Frame, Error> {
        let frame = self.frame().await?;
        if frame.identity != *identity {
            return Err(Error::Unavailable);
        }
        Ok(frame)
    }

    fn observed(&self, frame: &Frame) -> Result<Snapshot, Error> {
        if !boolean(frame.properties(DEVICE)?, "Powered")? {
            return Ok(unavailable());
        }
        let mut connected_ssid = None;
        let (link, access_point_active) = match frame.mode()? {
            "ap" => (
                LinkState::Disconnected,
                boolean(frame.properties(AP)?, "Started")?,
            ),
            "station" => {
                let station = frame.properties(STATION)?;
                let link = match string(station, "State")? {
                    "connected" | "roaming" => {
                        // ConnectedNetwork may be absent while association is changing.
                        let connected: OwnedObjectPath = object_path(station, "ConnectedNetwork")?;
                        let network = frame
                            .objects
                            .get(&connected)
                            .and_then(|interfaces| interfaces.get(NETWORK))
                            .ok_or(Error::Unavailable)?;
                        connected_ssid = Some(string(network, "Name")?.to_owned());
                        if self.addresses.has_usable_address(&self.interface)? {
                            LinkState::Ready
                        } else {
                            LinkState::Associated
                        }
                    }
                    "connecting" => LinkState::Associating,
                    "disconnected" | "disconnecting" => LinkState::Disconnected,
                    _ => LinkState::Unknown,
                };
                (link, false)
            }
            _ => (LinkState::Unknown, false),
        };
        Ok(Snapshot {
            backend: BackendState::Available,
            link,
            access_point_active,
            connected_ssid,
        })
    }

    async fn wait_for(
        &self,
        identity: &Identity,
        predicate: impl Fn(&Frame) -> Result<bool, Error>,
    ) -> Result<Frame, Error> {
        loop {
            let frame = self.same_frame(identity).await?;
            if predicate(&frame)? {
                return Ok(frame);
            }
            tokio::time::sleep(self.options.poll_interval).await;
        }
    }

    async fn station<'a>(&'a self, identity: &'a Identity) -> Result<StationProxy<'a>, Error> {
        StationProxy::builder(&self.connection)
            .destination(identity.owner.as_str())
            .map_err(map_error)?
            .path(identity.device.as_str())
            .map_err(map_error)?
            .build()
            .await
            .map_err(map_error)
    }
    async fn ap<'a>(&'a self, identity: &'a Identity) -> Result<AccessPointProxy<'a>, Error> {
        AccessPointProxy::builder(&self.connection)
            .destination(identity.owner.as_str())
            .map_err(map_error)?
            .path(identity.device.as_str())
            .map_err(map_error)?
            .build()
            .await
            .map_err(map_error)
    }
    async fn tracked_call(
        &self,
        future: impl std::future::Future<Output = Result<(), Error>> + Send + 'static,
    ) -> Result<(), Error> {
        let mut slot = self.pending_call.lock().await;
        if slot.is_some() {
            return Err(Error::Busy);
        }
        *slot = Some(tokio::spawn(future));
        let result = slot.as_mut().expect("installed call").await;
        *slot = None;
        result.map_err(|_| Error::Internal)?
    }

    async fn settle_call(&self) -> Result<(), Error> {
        let mut slot = self.pending_call.lock().await;
        if let Some(call) = slot.as_mut() {
            let result = call.await;
            *slot = None;
            // A received error reply still means the method is no longer pending.
            let _ = result.map_err(|_| Error::Internal)?;
        }
        Ok(())
    }

    async fn set_mode(&self, identity: &Identity, mode: &str) -> Result<(), Error> {
        let connection = self.connection.clone();
        let target = identity.clone();
        let requested = mode.to_owned();
        self.tracked_call(async move {
            let device = DeviceProxy::builder(&connection)
                .destination(target.owner.as_str())
                .map_err(map_error)?
                .path(target.device.as_str())
                .map_err(map_error)?
                .build()
                .await
                .map_err(map_error)?;
            device.set_mode(&requested).await.map_err(map_error)
        })
        .await?;
        self.wait_for(identity, |frame| {
            Ok(frame.mode()? == mode
                && frame
                    .properties(if mode == "ap" { AP } else { STATION })
                    .is_ok())
        })
        .await?;
        Ok(())
    }

    fn known(frame: &Frame) -> Result<Vec<(OwnedObjectPath, KnownNetwork)>, Error> {
        frame
            .objects
            .iter()
            .filter_map(|(path, interfaces)| interfaces.get(KNOWN).map(|props| (path, props)))
            .map(|(path, props)| {
                Ok((
                    path.clone(),
                    KnownNetwork {
                        network: network(props)?,
                        auto_connect: boolean(props, "AutoConnect")?,
                    },
                ))
            })
            .collect()
    }

    async fn forget(&self, frame: &Frame, target: &Network) -> Result<(), Error> {
        if target.security == Security::Unsupported {
            return Err(Error::Unsupported);
        }
        for (path, saved) in Self::known(frame)? {
            if saved.network == *target {
                let connection = self.connection.clone();
                let owner = frame.identity.owner.clone();
                self.tracked_call(async move {
                    let proxy = KnownNetworkProxy::builder(&connection)
                        .destination(owner.as_str())
                        .map_err(map_error)?
                        .path(path.as_str())
                        .map_err(map_error)?
                        .build()
                        .await
                        .map_err(map_error)?;
                    proxy.forget().await.map_err(map_error)
                })
                .await?;
            }
        }
        self.wait_for(&frame.identity, |frame| {
            Ok(!Self::known(frame)?
                .iter()
                .any(|(_, saved)| saved.network == *target))
        })
        .await?;
        Ok(())
    }

    fn network_path(frame: &Frame, target: &Network) -> Result<OwnedObjectPath, Error> {
        for (path, interfaces) in &frame.objects {
            if let Some(props) = interfaces.get(NETWORK) {
                let device: OwnedObjectPath = object_path(props, "Device")?;
                if device == frame.identity.device && network(props)? == *target {
                    return Ok(path.clone());
                }
            }
        }
        Err(Error::NetworkUnavailable)
    }

    async fn register_agent(&self, identity: &Identity) -> Result<(), Error> {
        // at() returns false when this adapter's object is already installed.
        self.connection
            .object_server()
            .at(self.agent_path.clone(), self.agent.clone())
            .await
            .map_err(map_error)?;
        let manager = AgentManagerProxy::builder(&self.connection)
            .destination(identity.owner.as_str())
            .map_err(map_error)?
            .path(MANAGER)
            .map_err(map_error)?
            .build()
            .await
            .map_err(map_error)?;
        match manager.register_agent(&self.agent_path).await {
            Ok(()) => Ok(()),
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "net.connman.iwd.AlreadyExists" =>
            {
                Ok(())
            }
            Err(error) => Err(map_error(error)),
        }
    }

    async fn connect(
        &self,
        frame: &Frame,
        target: &Network,
        credentials: Option<Credentials>,
    ) -> Result<Snapshot, Error> {
        if frame.mode()? != "station" {
            return Err(Error::Busy);
        }
        let path = Self::network_path(frame, target)?;
        let _secret_guard = SecretGuard(self.agent.clone());
        if let Some(credentials) = credentials {
            self.register_agent(&frame.identity).await?;
            *self.agent.0.lock().map_err(|_| Error::Internal)? = Some(Secret {
                owner: frame.identity.owner.clone(),
                network: path.clone(),
                password: credentials.password,
            });
        }
        let proxy = NetworkProxy::builder(&self.connection)
            .destination(frame.identity.owner.as_str())
            .map_err(map_error)?
            .path(path.as_str())
            .map_err(map_error)?
            .build()
            .await
            .map_err(map_error)?;
        // Failed is intentionally generic: iwd's Failed is NOT proof of a bad password.
        proxy.connect().await.map_err(map_error)?;
        let observed = self
            .wait_for(&frame.identity, |frame| {
                let current: Option<OwnedObjectPath> =
                    object_path(frame.properties(STATION)?, "ConnectedNetwork").ok();
                Ok(current.as_ref() == Some(&path)
                    && self.observed(frame)?.link == LinkState::Ready)
            })
            .await?;
        self.observed(&observed)
    }

    async fn apply_inner(
        &mut self,
        operation: OperationId,
        action: Action,
    ) -> Result<Snapshot, Error> {
        let frame = self.frame().await?;
        if !boolean(frame.properties(DEVICE)?, "Powered")? {
            return Err(Error::Unavailable);
        }
        if matches!(
            &action,
            Action::Connect(_) | Action::ReplaceCredentials(_) | Action::ConnectKnown(_)
        ) && frame.mode()? != "station"
        {
            return Err(Error::Busy);
        }
        // Preflight input and replacement intent before any mutation.
        match &action {
            Action::Connect(credentials) | Action::ReplaceCredentials(credentials) => {
                validate(credentials, false)?;
                let target = credential_network(credentials);
                Self::network_path(&frame, &target)?;
                if matches!(&action, Action::Connect(_))
                    && Self::known(&frame)?
                        .iter()
                        .any(|(_, saved)| saved.network == target)
                {
                    return Err(Error::CredentialConflict);
                }
            }
            Action::ConnectKnown(target)
                if !Self::known(&frame)?
                    .iter()
                    .any(|(_, saved)| saved.network == *target) =>
            {
                return Err(Error::NetworkUnavailable);
            }
            Action::StartAccessPoint if self.options.access_point.is_none() => {
                return Err(Error::InvalidInput);
            }
            _ => {}
        }
        self.pending = Some(Pending {
            operation,
            identity: frame.identity.clone(),
            changes_radio: !matches!(&action, Action::Forget { .. }),
        });
        match action {
            Action::StartAccessPoint => {
                self.set_mode(&frame.identity, "ap").await?;
                let config = self
                    .options
                    .access_point
                    .as_ref()
                    .ok_or(Error::InvalidInput)?;
                let ap = self.ap(&frame.identity).await?;
                match config {
                    AccessPointConfig::Credentials(credentials) => {
                        ap.start(&credentials.ssid, &credentials.password).await
                    }
                    AccessPointConfig::Profile(ssid) => ap.start_profile(ssid).await,
                }
                .map_err(map_error)?;
                let fresh = self
                    .wait_for(&frame.identity, |frame| {
                        boolean(frame.properties(AP)?, "Started")
                    })
                    .await?;
                self.observed(&fresh)
            }
            Action::StopAccessPoint => {
                if frame.mode()? == "ap" {
                    self.ap(&frame.identity)
                        .await?
                        .stop()
                        .await
                        .map_err(map_error)?;
                }
                self.set_mode(&frame.identity, "station").await?;
                self.observed(&self.same_frame(&frame.identity).await?)
            }
            Action::Connect(credentials) => {
                self.connect(&frame, &credential_network(&credentials), Some(credentials))
                    .await
            }
            Action::ReplaceCredentials(credentials) => {
                let target = credential_network(&credentials);
                self.forget(&frame, &target).await?;
                let fresh = self.same_frame(&frame.identity).await?;
                self.connect(&fresh, &target, Some(credentials)).await
            }
            Action::ConnectKnown(target) => self.connect(&frame, &target, None).await,
            Action::Disconnect => {
                self.station(&frame.identity)
                    .await?
                    .disconnect()
                    .await
                    .map_err(map_error)?;
                let fresh = self
                    .wait_for(&frame.identity, |frame| {
                        Ok(string(frame.properties(STATION)?, "State")? == "disconnected")
                    })
                    .await?;
                self.observed(&fresh)
            }
            Action::Forget { network: selected } => {
                self.forget(&frame, &selected).await?;
                let mut fresh = self.same_frame(&frame.identity).await?;
                // Forget may remove only the profile. Explicitly end a matching
                // station connection, while leaving unrelated connections alone.
                let connected = if fresh.mode()? == "station" {
                    object_path(fresh.properties(STATION)?, "ConnectedNetwork")
                        .ok()
                        .and_then(|path| fresh.objects.get(&path))
                        .and_then(|interfaces| interfaces.get(NETWORK))
                        .map(network)
                        .transpose()?
                } else {
                    None
                };
                if connected.as_ref() == Some(&selected) {
                    self.pending
                        .as_mut()
                        .expect("active operation")
                        .changes_radio = true;
                    let connection = self.connection.clone();
                    let identity = frame.identity.clone();
                    self.tracked_call(async move {
                        StationProxy::builder(&connection)
                            .destination(identity.owner.as_str())
                            .map_err(map_error)?
                            .path(identity.device.as_str())
                            .map_err(map_error)?
                            .build()
                            .await
                            .map_err(map_error)?
                            .disconnect()
                            .await
                            .map_err(map_error)
                    })
                    .await?;
                    fresh = self
                        .wait_for(&frame.identity, |frame| {
                            Ok(string(frame.properties(STATION)?, "State")? == "disconnected")
                        })
                        .await?;
                }
                self.observed(&fresh)
            }
        }
    }

    async fn reconcile(&mut self, operation: OperationId) -> Result<Snapshot, Error> {
        let Some(pending) = self.pending.clone() else {
            return self.observed(&self.frame().await?);
        };
        if pending.operation != operation {
            return Err(Error::Busy);
        }
        self.agent.clear();
        let mut frame = self.frame().await?;
        if frame.identity.owner == pending.identity.owner {
            self.settle_call().await?;
            frame = self.frame().await?;
        }
        if frame.identity.owner != pending.identity.owner {
            let bus = DBusProxy::new(&self.connection).await.map_err(map_error)?;
            if bus
                .name_has_owner(
                    pending
                        .identity
                        .owner
                        .as_str()
                        .try_into()
                        .map_err(|_| Error::Internal)?,
                )
                .await
                .map_err(|e| map_error(e.into()))?
            {
                return Err(Error::Unavailable);
            }
            // The old owner has exited, so its work cannot complete on the new device.
            if let Some(call) = self.pending_call.lock().await.take() {
                call.abort();
            }
        } else if frame.identity.device != pending.identity.device {
            if frame.objects.contains_key(&pending.identity.device) {
                return Err(Error::Unavailable);
            }
        } else if pending.changes_radio {
            if frame.mode()? == "ap" {
                self.ap(&frame.identity)
                    .await?
                    .stop()
                    .await
                    .map_err(map_error)?;
                self.set_mode(&frame.identity, "station").await?;
            }
            self.station(&frame.identity)
                .await?
                .disconnect()
                .await
                .map_err(map_error)?;
            self.wait_for(&frame.identity, |frame| {
                Ok(frame.mode()? == "station"
                    && string(frame.properties(STATION)?, "State")? == "disconnected")
            })
            .await?;
        }
        let fresh = self.frame().await?;
        let snapshot = self.observed(&fresh)?;
        self.pending = None;
        Ok(snapshot)
    }

    /// Count associated AP clients; this alone is NOT evidence of a DHCP lease.
    pub async fn access_point_clients(&self) -> Result<usize, Error> {
        bounded(self.options.operation_timeout, async {
            let frame = self.frame().await?;
            if !self.observed(&frame)?.access_point_active {
                return Err(Error::Busy);
            }
            let proxy = AccessPointDiagnosticProxy::builder(&self.connection)
                .destination(frame.identity.owner.as_str())
                .map_err(map_error)?
                .path(frame.identity.device.as_str())
                .map_err(map_error)?
                .build()
                .await
                .map_err(map_error)?;
            let clients = proxy.get_diagnostics().await.map_err(map_error)?;
            self.same_frame(&frame.identity).await?;
            Ok(clients.len())
        })
        .await
    }
}

impl<A: AddressSource> Adapter for IwdAdapter<A> {
    async fn snapshot(&mut self) -> Result<Snapshot, Error> {
        if self.ensure_connection().await.is_err() {
            return Ok(unavailable());
        }
        match bounded(
            self.options.operation_timeout.min(Duration::from_secs(3)),
            async { self.observed(&self.frame().await?) },
        )
        .await
        {
            Err(Error::Unavailable | Error::Timeout) => Ok(unavailable()),
            result => result,
        }
    }
    async fn known_networks(&mut self) -> Result<Vec<KnownNetwork>, Error> {
        self.ensure_connection().await?;
        bounded(self.options.operation_timeout, async {
            let mut known: Vec<_> = Self::known(&self.frame().await?)?
                .into_iter()
                .map(|(_, saved)| saved)
                .collect();
            known.sort_by(|a, b| a.network.ssid.cmp(&b.network.ssid));
            Ok(known)
        })
        .await
    }
    async fn scan(&mut self) -> Result<Vec<Network>, Error> {
        self.ensure_connection().await?;
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        bounded(self.options.operation_timeout, async {
            let frame = self.frame().await?;
            if frame.mode()? != "station" {
                return Err(Error::Unsupported);
            }
            let station = self.station(&frame.identity).await?;
            match station.scan().await {
                Ok(()) => {}
                Err(error) => {
                    let error = map_error(error);
                    if error != Error::Busy {
                        return Err(error);
                    }
                }
            }
            let fresh = self
                .wait_for(&frame.identity, |frame| {
                    Ok(!boolean(frame.properties(STATION)?, "Scanning")?)
                })
                .await?;
            let ordered = station.get_ordered_networks().await.map_err(map_error)?;
            let mut networks = Vec::new();
            for (path, _) in ordered {
                if let Some(props) = fresh.objects.get(&path).and_then(|i| i.get(NETWORK)) {
                    networks.push(network(props)?);
                }
            }
            self.same_frame(&frame.identity).await?;
            Ok(networks)
        })
        .await
    }
    async fn apply(&mut self, operation: OperationId, action: Action) -> Result<Snapshot, Error> {
        self.ensure_connection().await?;
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        let result = bounded(
            self.options.operation_timeout,
            self.apply_inner(operation, action),
        )
        .await;
        if result.is_ok() {
            self.pending = None;
        }
        result
    }
    async fn cancel_and_reconcile(&mut self, operation: OperationId) -> Result<Snapshot, Error> {
        self.ensure_connection().await?;
        bounded(self.options.operation_timeout, self.reconcile(operation)).await
    }
}

async fn bounded<T>(
    duration: Duration,
    future: impl std::future::Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    tokio::time::timeout(duration, future)
        .await
        .map_err(|_| Error::Timeout)?
}
fn unavailable() -> Snapshot {
    Snapshot {
        backend: BackendState::Unavailable,
        link: LinkState::Unknown,
        access_point_active: false,
        connected_ssid: None,
    }
}
fn string<'a>(props: &'a HashMap<String, OwnedValue>, name: &str) -> Result<&'a str, Error> {
    props
        .get(name)
        .and_then(|value| <&str>::try_from(value).ok())
        .ok_or(Error::Internal)
}
fn object_path(props: &HashMap<String, OwnedValue>, name: &str) -> Result<OwnedObjectPath, Error> {
    let value = props.get(name).ok_or(Error::Unavailable)?;
    OwnedObjectPath::try_from(value.try_clone().map_err(|_| Error::Internal)?)
        .map_err(|_| Error::Internal)
}
fn boolean(props: &HashMap<String, OwnedValue>, name: &str) -> Result<bool, Error> {
    props
        .get(name)
        .and_then(|value| bool::try_from(value).ok())
        .ok_or(Error::Internal)
}
fn network(props: &HashMap<String, OwnedValue>) -> Result<Network, Error> {
    Ok(Network {
        ssid: string(props, "Name")?.into(),
        security: match string(props, "Type")? {
            "open" => Security::Open,
            "psk" => Security::Personal,
            "8021x" => Security::Enterprise,
            _ => Security::Unsupported,
        },
    })
}
fn credential_network(credentials: &Credentials) -> Network {
    Network {
        ssid: credentials.ssid.clone(),
        security: if credentials.password.is_empty() {
            Security::Open
        } else {
            Security::Personal
        },
    }
}
fn validate(credentials: &Credentials, ap: bool) -> Result<(), Error> {
    if credentials.ssid.is_empty() || credentials.ssid.len() > 32 || credentials.ssid.contains('\0')
    {
        return Err(Error::InvalidInput);
    }
    let password = &credentials.password;
    if (!password.is_empty() || ap)
        && (!(8..=63).contains(&password.len())
            || !password.is_ascii()
            || password.bytes().any(|b| b < 32 || b == 127))
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}
fn map_error(error: zbus::Error) -> Error {
    if let zbus::Error::FDO(error) = &error {
        return match error.as_ref() {
            zbus::fdo::Error::ZBus(error) => map_error(error.clone()),
            zbus::fdo::Error::NameHasNoOwner(_)
            | zbus::fdo::Error::ServiceUnknown(_)
            | zbus::fdo::Error::UnknownObject(_)
            | zbus::fdo::Error::Disconnected(_)
            | zbus::fdo::Error::IOError(_)
            | zbus::fdo::Error::UnknownInterface(_) => Error::Unavailable,
            zbus::fdo::Error::NoReply(_)
            | zbus::fdo::Error::Timeout(_)
            | zbus::fdo::Error::TimedOut(_) => Error::Timeout,
            _ => Error::Internal,
        };
    }
    match error {
        zbus::Error::MethodError(name, _, _) => match name.as_str() {
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.DBus.Error.UnknownInterface"
            | "net.connman.iwd.NotAvailable" => Error::Unavailable,
            "org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout" => {
                Error::Timeout
            }
            "net.connman.iwd.Busy" | "net.connman.iwd.InProgress" => Error::Busy,
            "net.connman.iwd.Aborted" | "net.connman.iwd.Agent.Error.Canceled" => Error::Cancelled,
            "net.connman.iwd.NotSupported" => Error::Unsupported,
            "net.connman.iwd.NotFound" => Error::NetworkUnavailable,
            "net.connman.iwd.InvalidArguments" => Error::InvalidInput,
            // iwd 3.12 returns Failed for multiple causes, including auth failure.
            // Never infer invalid credentials or automatically Forget from Failed.
            _ => Error::Internal,
        },
        zbus::Error::InputOutput(_) => Error::Unavailable,
        _ => Error::Internal,
    }
}

#[cfg(test)]
mod tests;
