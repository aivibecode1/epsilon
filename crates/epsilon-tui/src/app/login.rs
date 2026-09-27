//! The OAuth login adapter: the bridge between the background login thread in
//! `epsilon-llm` and the TUI's channels and overlay.
//!
//! ## Why this file exists
//!
//! `OauthUiEvent` is what the worker thread reports, `TuiLoginUi` is that thread's view of
//! the terminal, and the `LoginUi` impl is how it says anything at all. The three belong
//! together and to nothing else - they are the only place in the TUI that implements a
//! foreign trait for a local type.
//!
//! ## Why the `real` gate matters here
//!
//! All of it is `#[cfg(feature = "real")]`, because a faux provider has nothing to log in
//! to. That makes this file the one part of `app` that compiles in some builds and not
//! others, which is exactly the kind of thing that is much easier to see when it has an
//! address than when it is three items in the middle of a long file.
//!
//! ## The rule for this move
//!
//! Sliced by line range and byte-identical to `app.rs` at the parent commit.

// ---- OAuth login plumbing (M52) ---------------------------------------
/// M52: what the background OAuth login thread tells the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "real"), allow(dead_code))]
pub(super) enum OauthUiEvent {
    /// The flow is ready for a pasted authorization code / redirect URL.
    NeedCode {
        /// Provider whose login is waiting.
        provider: String,
    },
    /// The ceremony finished (success or failure) — close the overlay.
    Done(String),
}

/// M52: the login thread's view of the terminal. Notices and events travel
/// over the app's channels; the pasted code arrives from the overlay.
#[cfg(feature = "real")]
pub(super) struct TuiLoginUi {
    pub(super) notices: tokio::sync::mpsc::UnboundedSender<String>,
    pub(super) events: tokio::sync::mpsc::UnboundedSender<OauthUiEvent>,
    pub(super) code_rx: std::sync::mpsc::Receiver<String>,
    pub(super) provider: String,
}

#[cfg(feature = "real")]
impl epsilon_llm::oauth_login::LoginUi for TuiLoginUi {
    fn notify(&mut self, message: &str) {
        let _ = self.notices.send(message.to_string());
    }

    fn device_code(&mut self, prompt: &epsilon_llm::oauth_login::DevicePrompt) {
        let _ = self.notices.send(format!(
            "device login: open {} and enter code {} (valid for {}s)",
            prompt.verification_uri, prompt.user_code, prompt.expires_in_seconds
        ));
    }

    fn wait_for_code(
        &mut self,
        callback: Option<&epsilon_llm::oauth_login::CallbackListener>,
    ) -> Option<String> {
        let _ = self.events.send(OauthUiEvent::NeedCode {
            provider: self.provider.clone(),
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
        loop {
            match self.code_rx.try_recv() {
                Ok(line) if !line.trim().is_empty() => return Some(line),
                Ok(_) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Esc (or shutdown) dropped the sender: the user cancelled.
                    return None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            if let Some(cb) = callback {
                match cb.try_recv() {
                    Some(Ok(code)) => return Some(code.code),
                    Some(Err(_)) => return None,
                    None => {}
                }
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
    }
}
