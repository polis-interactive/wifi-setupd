/// A dismissed backend error stays hidden across identical status polls.
/// A cleared/changed error or a newly accepted attempt permits a fresh notice.
#[derive(Default)]
pub struct ErrorNotice {
    current: Option<String>,
    dismissed: bool,
}
impl ErrorNotice {
    pub fn observe(&mut self, error: Option<String>) {
        if self.current != error {
            self.current = error;
            self.dismissed = false;
        }
    }
    pub fn visible(&self) -> Option<&str> {
        if self.dismissed {
            None
        } else {
            self.current.as_deref()
        }
    }
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }
    pub fn new_attempt(&mut self) {
        *self = Self::default();
    }
}

pub fn friendly(error: &str) -> &str {
    match error {
        "AuthenticationFailed" => "The network rejected the password. Check it and try again.",
        "NetworkUnavailable" => "That network could not be found. Check the name and try again.",
        "CredentialConflict" => "The saved network could not be replaced. Please retry.",
        "TimedOut" => "The connection took too long. Check the network and try again.",
        "AddressUnavailable" => {
            "Wi-Fi connected but did not receive an IP address. Check the router and retry."
        }
        _ => "The connection could not be completed. Check the network and password, then retry.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dismissal_survives_polling_but_a_new_attempt_can_fail_again() {
        let mut notice = ErrorNotice::default();
        notice.observe(Some("AuthenticationFailed".into()));
        assert_eq!(notice.visible(), Some("AuthenticationFailed"));
        notice.dismiss();
        notice.observe(Some("AuthenticationFailed".into()));
        assert_eq!(notice.visible(), None);
        // Even if polling missed the intervening Connecting state.
        notice.new_attempt();
        notice.observe(Some("AuthenticationFailed".into()));
        assert_eq!(notice.visible(), Some("AuthenticationFailed"));
    }

    #[test]
    fn new_failure_after_clear_or_changed_error_is_visible() {
        let mut notice = ErrorNotice::default();
        notice.observe(Some("AuthenticationFailed".into()));
        notice.dismiss();
        notice.observe(None);
        notice.observe(Some("AuthenticationFailed".into()));
        assert_eq!(notice.visible(), Some("AuthenticationFailed"));
        notice.dismiss();
        notice.observe(Some("NetworkUnavailable".into()));
        assert_eq!(notice.visible(), Some("NetworkUnavailable"));
    }
}
