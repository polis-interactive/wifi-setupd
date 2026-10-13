mod api;
mod error;
use dioxus::dioxus_core::{Runtime, current_scope_id};
use dioxus::prelude::*;
use error::ErrorNotice;
pub use error::friendly;
use wifi_setupd_http_api::{Accepted, ConnectRequest, Network, PortalStatus};

#[derive(Clone, Copy)]
pub struct PortalState {
    owner: ScopeId,
    pub status: Signal<Option<PortalStatus>>,
    pub offline: Signal<bool>,
    pub ssid: Signal<String>,
    pub password: Signal<String>,
    pub open_network: Signal<bool>,
    pub show_password: Signal<bool>,
    pub busy: Signal<bool>,
    pub switching: Signal<bool>,
    pub message: Signal<String>,
    pub device_key: Signal<String>,
    pub confirm: Signal<Option<Network>>,
    pub forgetting: Signal<Option<Network>>,
    pub rejoining: Signal<bool>,
    forget_epoch: Signal<u64>,
    pub error: Signal<ErrorNotice>,
}

pub fn use_portal_state() -> PortalState {
    let status = use_signal(|| None::<PortalStatus>);
    let offline = use_signal(|| false);
    let ssid = use_signal(String::new);
    let password = use_signal(String::new);
    let open_network = use_signal(|| false);
    let show_password = use_signal(|| false);
    let busy = use_signal(|| false);
    let switching = use_signal(|| false);
    let message = use_signal(String::new);
    let device_key = use_signal(String::new);
    let confirm = use_signal(|| None::<Network>);
    let forgetting = use_signal(|| None::<Network>);
    let rejoining = use_signal(|| false);
    let forget_epoch = use_signal(|| 0);
    let error = use_signal(ErrorNotice::default);
    let mut state = PortalState {
        owner: current_scope_id(),
        status,
        offline,
        ssid,
        password,
        open_network,
        show_password,
        busy,
        switching,
        message,
        device_key,
        confirm,
        forgetting,
        rejoining,
        forget_epoch,
        error,
    };
    use_future(move || async move {
        loop {
            // Only reconcile a deletion with a status request started after its response.
            let reconcile_forget = !*state.busy.peek() && state.forgetting.peek().is_some();
            let epoch = *state.forget_epoch.peek();
            match api::get_status().await {
                Ok(mut next) => {
                    if epoch != *state.forget_epoch.peek() {
                        continue;
                    }
                    if reconcile_forget && next.saved_available {
                        let selected = state.forgetting.peek().clone();
                        if selected
                            .as_ref()
                            .is_some_and(|network| !next.saved.contains(network))
                        {
                            state.forgetting.set(None);
                        } else if next.error.is_some() {
                            state.rejoining.set(false);
                            state.forgetting.set(None);
                            state.confirm.set(selected);
                            state
                                .message
                                .set("The network could not be forgotten. Please retry.".into());
                        }
                    }
                    if !*state.busy.peek()
                        && state.forgetting.peek().is_none()
                        && (next.link == "Ready" || next.state == "AwaitingCredentials")
                    {
                        state.rejoining.set(false);
                    }
                    if !next.saved_available
                        && let Some(previous) = state.status.peek().as_ref()
                    {
                        next.saved = previous.saved.clone();
                    }
                    if next.link == "Ready"
                        || (next.state == "AwaitingCredentials" && next.error.is_some())
                    {
                        state.switching.set(false);
                    }
                    state.error.write().observe(next.error.clone());
                    state.status.set(Some(next));
                    state.offline.set(false);
                }
                Err(_) => state.offline.set(true),
            }
            gloo_timers::future::TimeoutFuture::new(2000).await;
        }
    });
    state
}

impl PortalState {
    pub fn ready(self) -> bool {
        !*self.rejoining.read()
            && self
                .status
                .read()
                .as_ref()
                .is_some_and(|s| s.link == "Ready")
    }
    pub fn awaiting(self) -> bool {
        self.status
            .read()
            .as_ref()
            .is_some_and(|s| s.state == "AwaitingCredentials")
    }
    pub fn disabled(self) -> bool {
        *self.busy.read() || *self.switching.read() || !self.awaiting() || *self.offline.read()
    }
    pub fn dismiss_error(mut self) {
        if self.error.peek().visible().is_some() {
            self.error.write().dismiss();
        }
    }
    pub fn submit(mut self) {
        self.dismiss_error();
        if self.disabled() {
            return;
        }
        let Some(status) = self.status.peek().clone() else {
            return;
        };
        self.busy.set(true);
        self.message.set(String::new());
        let request = ConnectRequest {
            ssid: self.ssid.peek().clone(),
            password: if *self.open_network.peek() {
                String::new()
            } else {
                self.password.peek().clone()
            },
            replace: true,
        };
        // Requests belong to App, so changing pages does not cancel them.
        Runtime::current().spawn(self.owner, async move {
            match api::post("/api/connect", &request, &status.csrf, "").await {
                Ok(response) => match response.json::<Accepted>().await {
                    Ok(accepted) => {
                        self.password.set(String::new());
                        self.switching.set(true);
                        // Receipt of acceptance precedes intentional AP teardown.
                        match api::post(
                            &format!("/api/connect/{}/begin", accepted.attempt),
                            &(),
                            &status.csrf,
                            "",
                        )
                        .await
                        {
                            Ok(_) => self.error.write().new_attempt(),
                            Err(error) => self.message.set(error.to_string()),
                        }
                    }
                    Err(_) => self
                        .message
                        .set("Could not read acceptance. Wait 30 seconds, then retry.".into()),
                },
                Err(error) => self.message.set(error.to_string()),
            }
            self.busy.set(false);
        });
    }
    pub fn forget(mut self, selected: Network) {
        if *self.busy.peek() || self.forgetting.peek().is_some() {
            return;
        }
        let Some(status) = self.status.peek().clone() else {
            return;
        };
        self.busy.set(true);
        self.message.set(String::new());
        self.forgetting.set(Some(selected.clone()));
        self.forget_epoch += 1;
        self.rejoining.set(
            status.link == "Ready"
                && status.connected_ssid.as_deref() == Some(selected.ssid.as_str()),
        );
        self.confirm.set(None);
        let key = self.device_key.peek().clone();
        // Requests belong to App, so changing pages does not cancel them.
        Runtime::current().spawn(self.owner, async move {
            match api::post("/api/forget", &selected, &status.csrf, &key).await {
                Ok(_) => {}
                Err(api::PostError::Rejected(error)) => {
                    self.forget_epoch += 1;
                    self.rejoining.set(false);
                    self.forgetting.set(None);
                    self.confirm.set(Some(selected));
                    self.message.set(error);
                }
                Err(api::PostError::Uncertain(error)) => {
                    // Disconnecting can lose the response after deletion succeeded.
                    // Keep the reconnect screen until a fresh status reconciles it.
                    self.message.set(error);
                }
            }
            self.busy.set(false);
        });
    }
}
