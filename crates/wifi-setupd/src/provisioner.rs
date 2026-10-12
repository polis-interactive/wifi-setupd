//! One policy actor. Adapter futures run outside its mailbox; cancelled work must
//! reconcile before another mutation is admitted. No credential persistence here.
use crate::wifi::{Action, Adapter, Credentials, Error, Network, OperationId, Snapshot};
use kameo::{
    actor::{Actor, ActorRef, Spawn, WeakActorRef},
    error::{ActorStopReason, Infallible},
    message::{Context, Message},
};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, broadcast, oneshot, watch},
    task::JoinHandle,
    time::{Instant, timeout},
};
use wifi_setupd_api::{
    BackendState, LinkState, ProvisioningError, ProvisioningState as State, Status,
};

#[derive(Clone)]
pub struct Config {
    pub startup_grace: Duration,
    pub poll_interval: Duration,
    pub operation_timeout: Duration,
    pub cleanup_timeout: Duration,
    pub automatic: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            startup_grace: Duration::from_secs(20),
            poll_interval: Duration::from_secs(1),
            operation_timeout: Duration::from_secs(90),
            cleanup_timeout: Duration::from_secs(15),
            automatic: true,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandError {
    Busy,
    BackendUnavailable,
    InvalidInput,
    Closed,
}
impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CommandError {}

#[derive(Clone, Copy, Debug)]
pub struct Publication {
    pub status: Status,
    pub completed: bool,
    pub revision: u64,
}

pub struct Handle<A: Adapter + 'static> {
    actor: ActorRef<Provisioner<A>>,
    status: watch::Receiver<Status>,
    publications: broadcast::Sender<Publication>,
}
impl<A: Adapter + 'static> Clone for Handle<A> {
    fn clone(&self) -> Self {
        Self {
            actor: self.actor.clone(),
            status: self.status.clone(),
            publications: self.publications.clone(),
        }
    }
}
impl<A: Adapter + 'static> Handle<A> {
    pub fn spawn(adapter: A, config: Config) -> Result<Self, CommandError> {
        if config.poll_interval.is_zero()
            || config.operation_timeout.is_zero()
            || config.cleanup_timeout.is_zero()
        {
            return Err(CommandError::InvalidInput);
        }
        let (status, receiver) = crate::status::channel(Status::default());
        let (publications, _) = broadcast::channel(128);
        let actor = Provisioner::spawn(Provisioner {
            adapter: Arc::new(Mutex::new(adapter)),
            automatic: config.automatic,
            config,
            status,
            publications: publications.clone(),
            revision: 0,
            epoch: 0,
            next_id: 0,
            observing: false,
            job: None,
            recovery: None,
            cancelled: false,
            not_ready_since: None,
            ticker: None,
        });
        Ok(Self {
            actor,
            status: receiver,
            publications,
        })
    }
    pub fn status(&self) -> Status {
        *self.status.borrow()
    }
    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }
    pub fn publications(&self) -> broadcast::Receiver<Publication> {
        self.publications.subscribe()
    }
    pub(crate) async fn checkpoint(&self) -> Result<Publication, CommandError> {
        self.actor.ask(Read).await.map_err(|_| CommandError::Closed)
    }
    async fn command(&self, command: Command) -> Result<(), CommandError> {
        self.actor.ask(command).await.map_err(|e| match e {
            kameo::error::SendError::HandlerError(error) => error,
            _ => CommandError::Closed,
        })
    }
    pub async fn start(&self) -> Result<(), CommandError> {
        self.command(Command::Start).await
    }
    pub async fn stop(&self) -> Result<(), CommandError> {
        self.command(Command::Stop).await
    }
    /// Internal entry point for the later HTTP layer. Replacement is explicit.
    pub async fn submit(
        &self,
        credentials: Credentials,
        replace: bool,
    ) -> Result<(), CommandError> {
        if credentials.ssid.is_empty()
            || credentials.ssid.len() > 32
            || credentials.ssid.contains('\0')
            || (!credentials.password.is_empty() && !(8..=63).contains(&credentials.password.len()))
        {
            return Err(CommandError::InvalidInput);
        }
        self.command(Command::Connect(if replace {
            Action::ReplaceCredentials(credentials)
        } else {
            Action::Connect(credentials)
        }))
        .await
    }
    pub async fn connect_known(&self, network: Network) -> Result<(), CommandError> {
        self.command(Command::Connect(Action::ConnectKnown(network)))
            .await
    }
    /// Stop provisioning first. A failure/timeout deliberately leaves the actor
    /// alive and Stopping so callers cannot mistake uncertain cleanup for success.
    pub async fn shutdown(&self) -> Result<(), CommandError> {
        self.stop().await?;
        let mut status = self.subscribe();
        timeout(Duration::from_secs(30), async {
            while status.borrow().state.is_active() {
                status.changed().await.map_err(|_| CommandError::Closed)?;
            }
            Ok::<_, CommandError>(())
        })
        .await
        .map_err(|_| CommandError::Busy)??;
        self.actor
            .stop_gracefully()
            .await
            .map_err(|_| CommandError::Closed)?;
        self.actor.wait_for_shutdown().await;
        Ok(())
    }
}

enum Command {
    Start,
    Stop,
    Connect(Action),
}
struct Tick;
struct Read;
struct Observation {
    epoch: u64,
    result: Result<Snapshot, Error>,
}
struct Stage {
    id: OperationId,
    state: State,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Start,
    Connect,
    Stop,
    Cleanup,
}
enum Work {
    Start,
    Connect(Action),
    Stop { disconnect: bool },
    Cleanup(OperationId),
}
impl Work {
    fn kind(&self) -> Kind {
        match self {
            Self::Start => Kind::Start,
            Self::Connect(_) => Kind::Connect,
            Self::Stop { .. } => Kind::Stop,
            Self::Cleanup(_) => Kind::Cleanup,
        }
    }
}
struct Job {
    id: OperationId,
    kind: Kind,
    cancel: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}
#[derive(Clone, Copy)]
enum Goal {
    Cancel,
    Fail,
    Portal,
}
struct Recovery {
    id: OperationId,
    goal: Goal,
    error: Error,
}
struct Finished {
    id: OperationId,
    result: Result<Snapshot, Error>,
    cleanup: Result<Snapshot, Error>,
}

pub struct Provisioner<A: Adapter + 'static> {
    adapter: Arc<Mutex<A>>,
    config: Config,
    automatic: bool,
    status: watch::Sender<Status>,
    publications: broadcast::Sender<Publication>,
    revision: u64,
    epoch: u64,
    next_id: u64,
    observing: bool,
    job: Option<Job>,
    recovery: Option<Recovery>,
    cancelled: bool,
    not_ready_since: Option<Instant>,
    ticker: Option<JoinHandle<()>>,
}
impl<A: Adapter + 'static> Actor for Provisioner<A> {
    type Args = Self;
    type Error = Infallible;
    async fn on_start(mut state: Self, actor: ActorRef<Self>) -> Result<Self, Infallible> {
        let weak = actor.downgrade();
        let interval = state.config.poll_interval;
        state.ticker = Some(tokio::spawn(async move {
            let mut timer = tokio::time::interval(interval);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;
                let Some(actor) = weak.upgrade() else { break };
                if actor.tell(Tick).await.is_err() {
                    break;
                }
            }
        }));
        Ok(state)
    }
    async fn on_stop(
        &mut self,
        _: WeakActorRef<Self>,
        _: ActorStopReason,
    ) -> Result<(), Infallible> {
        if let Some(ticker) = self.ticker.take() {
            ticker.abort();
        }
        if let Some(mut job) = self.job.take() {
            if let Some(cancel) = job.cancel.take() {
                let _ = cancel.send(());
            }
            let _ = job.task.await;
        }
        Ok(())
    }
}
impl<A: Adapter + 'static> Provisioner<A> {
    fn current(&self) -> Status {
        *self.status.borrow()
    }
    fn publish(&mut self, status: Status, completed: bool) {
        if status == self.current() && !completed {
            return;
        }
        self.status.send_replace(status);
        self.revision += 1;
        let _ = self.publications.send(Publication {
            status,
            completed,
            revision: self.revision,
        });
    }
    fn snapshot_into(status: &mut Status, snapshot: &Snapshot) {
        status.backend = snapshot.backend;
        status.link = if snapshot.backend == BackendState::Available {
            snapshot.link
        } else {
            LinkState::Unknown
        };
    }
    fn launch(&mut self, work: Work, actor: &ActorRef<Self>) {
        debug_assert!(self.job.is_none());
        self.next_id += 1;
        self.epoch += 1;
        let id = OperationId(self.next_id);
        let kind = work.kind();
        let (cancel, receiver) = oneshot::channel();
        let adapter = self.adapter.clone();
        let config = self.config.clone();
        let weak = actor.downgrade();
        let task = tokio::spawn(async move {
            let mut adapter = adapter.lock().await;
            let cleanup_id = match &work {
                Work::Cleanup(previous) => *previous,
                _ => id,
            };
            let result = tokio::select! {
                biased;
                _ = receiver => Err(Error::Cancelled),
                result = timeout(config.operation_timeout, perform(&mut *adapter, work, id, &weak)) => result.unwrap_or(Err(Error::Timeout)),
            };
            let cleanup = match &result {
                Ok(snapshot) => Ok(snapshot.clone()),
                Err(_) => timeout(
                    config.cleanup_timeout,
                    adapter.cancel_and_reconcile(cleanup_id),
                )
                .await
                .unwrap_or(Err(Error::Timeout))
                .and_then(available),
            };
            drop(adapter);
            if let Some(actor) = weak.upgrade() {
                let _ = actor
                    .tell(Finished {
                        id,
                        result,
                        cleanup,
                    })
                    .await;
            }
        });
        self.job = Some(Job {
            id,
            kind,
            cancel: Some(cancel),
            task,
        });
    }
    fn finish_recovery(
        &mut self,
        goal: Goal,
        error: Error,
        snapshot: Snapshot,
        actor: &ActorRef<Self>,
    ) {
        let mut status = self.current();
        Self::snapshot_into(&mut status, &snapshot);
        if self.cancelled || matches!(goal, Goal::Cancel) {
            // A late successful AP start may still need an explicit stop.
            if snapshot.access_point_active {
                self.launch(Work::Stop { disconnect: false }, actor);
                return;
            }
            status.state = State::Idle;
            status.last_error = ProvisioningError::Cancelled;
            self.cancelled = false;
        } else if matches!(goal, Goal::Portal) {
            status.state = State::StartingAccessPoint;
            status.last_error = error.into();
            self.publish(status, false);
            self.launch(Work::Start, actor);
            return;
        } else {
            status.state = State::Failed;
            status.last_error = error.into();
        }
        self.publish(status, false);
    }
}
fn available(snapshot: Snapshot) -> Result<Snapshot, Error> {
    if snapshot.backend == BackendState::Unavailable {
        Err(Error::Unavailable)
    } else {
        Ok(snapshot)
    }
}
async fn perform<A: Adapter + 'static>(
    adapter: &mut A,
    work: Work,
    id: OperationId,
    actor: &WeakActorRef<Provisioner<A>>,
) -> Result<Snapshot, Error> {
    match work {
        Work::Cleanup(previous) => adapter
            .cancel_and_reconcile(previous)
            .await
            .and_then(available),
        Work::Start => {
            let snapshot = available(adapter.snapshot().await?)?;
            if snapshot.link == LinkState::Ready || snapshot.access_point_active {
                return Ok(snapshot);
            }
            adapter.scan().await?;
            // Auto-connect can finish during the scan; do not tear down a
            // working connection based on the earlier snapshot.
            let snapshot = available(adapter.snapshot().await?)?;
            if snapshot.link == LinkState::Ready || snapshot.access_point_active {
                return Ok(snapshot);
            }
            adapter
                .apply(id, Action::StartAccessPoint)
                .await
                .and_then(available)
        }
        Work::Stop { disconnect } => {
            let snapshot = available(adapter.snapshot().await?)?;
            if snapshot.access_point_active {
                adapter.apply(id, Action::StopAccessPoint).await?;
            }
            if disconnect {
                adapter.apply(id, Action::Disconnect).await?;
            }
            adapter.snapshot().await.and_then(available)
        }
        Work::Connect(action) => {
            let known = if let Action::ConnectKnown(network) = &action {
                Some(network.clone())
            } else {
                None
            };
            // Explicit mode transition followed by fresh discovery avoids using
            // network paths from before AP teardown.
            adapter.apply(id, Action::StopAccessPoint).await?;
            adapter.scan().await?;
            let result = adapter.apply(id, action).await;
            if result == Err(Error::Authentication)
                && let Some(network) = known
            {
                adapter.cancel_and_reconcile(id).await?;
                adapter.apply(id, Action::Forget { network }).await?;
            }
            let snapshot = available(result?)?;
            if let Some(actor) = actor.upgrade() {
                let _ = actor
                    .tell(Stage {
                        id,
                        state: State::Verifying,
                    })
                    .await;
            }
            let verified = available(adapter.snapshot().await?)?;
            if snapshot.link != LinkState::Ready || verified.link != LinkState::Ready {
                return Err(Error::AddressUnavailable);
            }
            Ok(verified)
        }
    }
}
impl<A: Adapter + 'static> Message<Command> for Provisioner<A> {
    type Reply = Result<(), CommandError>;
    async fn handle(
        &mut self,
        command: Command,
        ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let mut status = self.current();
        match command {
            Command::Start => {
                if status.state == State::Stopping {
                    return Err(CommandError::Busy);
                }
                if status.state.is_active() {
                    return Ok(());
                }
                if status.backend == BackendState::Unavailable {
                    return Err(CommandError::BackendUnavailable);
                }
                self.automatic = true;
                if status.link == LinkState::Ready {
                    return Ok(());
                }
                status.state = State::StartingAccessPoint;
                status.last_error = ProvisioningError::None;
                self.publish(status, false);
                self.launch(Work::Start, ctx.actor_ref());
            }
            Command::Stop => {
                self.automatic = false;
                if !status.state.is_active() {
                    return Ok(());
                }
                self.cancelled = true;
                status.state = State::Stopping;
                self.publish(status, false);
                if let Some(job) = &mut self.job {
                    if let Some(cancel) = job.cancel.take() {
                        let _ = cancel.send(());
                    }
                } else if let Some(recovery) = &mut self.recovery {
                    recovery.goal = Goal::Cancel;
                } else {
                    self.launch(Work::Stop { disconnect: false }, ctx.actor_ref());
                }
            }
            Command::Connect(action) => {
                if self.job.is_some() || status.state != State::AwaitingCredentials {
                    return Err(CommandError::Busy);
                }
                if status.backend == BackendState::Unavailable {
                    return Err(CommandError::BackendUnavailable);
                }
                status.state = State::Connecting;
                status.last_error = ProvisioningError::None;
                self.publish(status, false);
                self.launch(Work::Connect(action), ctx.actor_ref());
            }
        }
        Ok(())
    }
}
impl<A: Adapter + 'static> Message<Tick> for Provisioner<A> {
    type Reply = ();
    async fn handle(&mut self, _: Tick, ctx: &mut Context<Self, ()>) {
        if self.job.is_some() {
            return;
        }
        if let Some(recovery) = &self.recovery {
            self.launch(Work::Cleanup(recovery.id), ctx.actor_ref());
            return;
        }
        if self.observing {
            return;
        }
        self.observing = true;
        let adapter = self.adapter.clone();
        let actor = ctx.actor_ref().downgrade();
        let epoch = self.epoch;
        let deadline = self.config.cleanup_timeout;
        tokio::spawn(async move {
            let result = timeout(deadline, async { adapter.lock().await.snapshot().await })
                .await
                .unwrap_or(Err(Error::Timeout));
            if let Some(actor) = actor.upgrade() {
                let _ = actor.tell(Observation { epoch, result }).await;
            }
        });
    }
}
impl<A: Adapter + 'static> Message<Observation> for Provisioner<A> {
    type Reply = ();
    async fn handle(&mut self, observation: Observation, ctx: &mut Context<Self, ()>) {
        self.observing = false;
        if observation.epoch != self.epoch || self.job.is_some() || self.recovery.is_some() {
            return;
        }
        let snapshot = observation.result.unwrap_or(Snapshot {
            backend: BackendState::Unavailable,
            link: LinkState::Unknown,
            access_point_active: false,
        });
        let mut status = self.current();
        Self::snapshot_into(&mut status, &snapshot);
        if snapshot.backend == BackendState::Unavailable {
            self.not_ready_since = None;
            if status.state.is_active() {
                status.state = State::Stopping;
                status.last_error = ProvisioningError::BackendUnavailable;
                self.recovery = Some(Recovery {
                    id: OperationId(self.next_id),
                    goal: Goal::Portal,
                    error: Error::Unavailable,
                });
            }
        } else if snapshot.link == LinkState::Ready {
            self.not_ready_since = None;
            status.state = State::Idle;
        } else if snapshot.access_point_active {
            status.state = State::AwaitingCredentials;
        } else if status.state == State::AwaitingCredentials {
            status.state = State::StartingAccessPoint;
            self.publish(status, false);
            self.launch(Work::Start, ctx.actor_ref());
            return;
        } else {
            let since = self.not_ready_since.get_or_insert_with(Instant::now);
            if self.automatic
                && status.state == State::Idle
                && since.elapsed() >= self.config.startup_grace
            {
                status.state = State::StartingAccessPoint;
                status.last_error = ProvisioningError::None;
                self.publish(status, false);
                self.launch(Work::Start, ctx.actor_ref());
                return;
            }
        }
        self.publish(status, false);
    }
}
impl<A: Adapter + 'static> Message<Stage> for Provisioner<A> {
    type Reply = ();
    async fn handle(&mut self, stage: Stage, _: &mut Context<Self, ()>) {
        if self.job.as_ref().is_some_and(|job| job.id == stage.id) && !self.cancelled {
            let mut status = self.current();
            status.state = stage.state;
            self.publish(status, false);
        }
    }
}
impl<A: Adapter + 'static> Message<Finished> for Provisioner<A> {
    type Reply = ();
    async fn handle(&mut self, finished: Finished, ctx: &mut Context<Self, ()>) {
        if !self.job.as_ref().is_some_and(|job| job.id == finished.id) {
            return;
        }
        let job = self.job.take().expect("matched operation");
        if job.kind == Kind::Cleanup {
            if let Ok(snapshot) = finished.result {
                let recovery = self.recovery.take().expect("cleanup has recovery goal");
                self.finish_recovery(recovery.goal, recovery.error, snapshot, ctx.actor_ref());
            }
            return;
        }
        if self.cancelled && finished.result.is_ok() && job.kind != Kind::Stop {
            self.launch(
                Work::Stop {
                    disconnect: job.kind == Kind::Connect,
                },
                ctx.actor_ref(),
            );
            return;
        }
        match finished.result {
            Ok(snapshot) => {
                let mut status = self.current();
                Self::snapshot_into(&mut status, &snapshot);
                if job.kind == Kind::Stop {
                    self.finish_recovery(Goal::Cancel, Error::Cancelled, snapshot, ctx.actor_ref());
                } else if snapshot.link == LinkState::Ready {
                    status.state = State::Idle;
                    status.last_error = ProvisioningError::None;
                    self.publish(status, job.kind == Kind::Connect);
                } else if snapshot.access_point_active {
                    status.state = State::AwaitingCredentials;
                    self.publish(status, false);
                } else {
                    status.state = State::Failed;
                    status.last_error = ProvisioningError::AccessPointFailed;
                    self.publish(status, false);
                }
            }
            Err(error) => {
                let goal = if self.cancelled {
                    Goal::Cancel
                } else if job.kind == Kind::Connect {
                    Goal::Portal
                } else {
                    Goal::Fail
                };
                match finished.cleanup {
                    Ok(snapshot) => self.finish_recovery(goal, error, snapshot, ctx.actor_ref()),
                    Err(cleanup) => {
                        let mut status = self.current();
                        status.state = State::Stopping;
                        status.last_error = error.into();
                        if cleanup == Error::Unavailable {
                            status.backend = BackendState::Unavailable;
                            status.link = LinkState::Unknown;
                        }
                        self.publish(status, false);
                        self.recovery = Some(Recovery {
                            id: job.id,
                            goal,
                            error,
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

impl<A: Adapter + 'static> Message<Read> for Provisioner<A> {
    type Reply = Result<Publication, Infallible>;
    async fn handle(&mut self, _: Read, _: &mut Context<Self, Self::Reply>) -> Self::Reply {
        Ok(Publication {
            status: self.current(),
            completed: false,
            revision: self.revision,
        })
    }
}
