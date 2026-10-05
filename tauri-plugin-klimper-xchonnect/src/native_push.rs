// SPDX-License-Identifier: AGPL-3.0-or-later
//! The native side of push (clapandpay TASK-087): the Swift plugin in `ios/` asks for
//! notification permission, registers with APNs and reports two things over a channel, the
//! device token and "a notification arrived". Elsewhere there is no native side.
//!
//! The channel is the only way in. It is not a webview command, so a page cannot hand the
//! wallet a device token or fake a wake-up.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Runtime,
    ipc::{Channel, InvokeResponseBody},
    plugin::{PluginApi, PluginHandle},
};

use xchonnect_core::push::MAX_DEVICE_TOKEN;

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_klimper_xchonnect);

/// What the phone's notification settings say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionState {
    Granted,
    Denied,
    Undetermined,
}

#[cfg_attr(not(mobile), allow(dead_code))]
#[derive(Debug, Deserialize)]
struct Reply {
    state: PermissionState,
}

/// What the Swift side reports. A notification's own content is never forwarded, so
/// there is nothing here for it to inject.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NativeEvent {
    Token { token: String },
    Wake,
}

/// Parse one event from the channel. A device token is hex and at most the length the
/// protocol allows; anything else is dropped rather than sealed into a registration.
pub fn parse_event(json: &str) -> Option<NativeEvent> {
    let event: NativeEvent = serde_json::from_str(json).ok()?;
    if let NativeEvent::Token { token } = &event
        && (token.is_empty()
            || token.len() > MAX_DEVICE_TOKEN
            || !token.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(event)
}

pub struct NativePush<R: Runtime>(Option<PluginHandle<R>>);

impl<R: Runtime> Clone for NativePush<R> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<R: Runtime> std::fmt::Debug for NativePush<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativePush").finish_non_exhaustive()
    }
}

#[derive(Serialize)]
struct StartArgs {
    channel: Channel<InvokeResponseBody>,
}

impl<R: Runtime> NativePush<R> {
    #[cfg_attr(not(target_os = "ios"), allow(clippy::unnecessary_wraps))]
    pub fn init<C: serde::de::DeserializeOwned>(
        _app: &AppHandle<R>,
        _api: &PluginApi<R, C>,
    ) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        #[cfg(target_os = "ios")]
        return Ok(Self(Some(
            _api.register_ios_plugin(init_plugin_klimper_xchonnect)?,
        )));
        #[cfg(not(target_os = "ios"))]
        Ok(Self(None))
    }

    /// Start listening and, when the user already allowed notifications, register with
    /// APNs. Never shows the permission prompt. Blocks until Swift answers: call it off the
    /// async runtime.
    pub fn start(
        &self,
        on_event: impl Fn(NativeEvent) + Send + Sync + 'static,
    ) -> Option<PermissionState> {
        let on_event = Arc::new(on_event);
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(json) = body
                && let Some(event) = parse_event(&json)
            {
                on_event(event);
            }
            Ok(())
        });
        self.run("start", StartArgs { channel })
    }

    /// Show the permission prompt (once; later calls only report). Blocks.
    pub fn request_permission(&self) -> Option<PermissionState> {
        self.run("requestPermission", ())
    }

    #[cfg_attr(not(mobile), allow(unused_variables, clippy::unused_self))]
    fn run<Q: Serialize>(&self, method: &str, args: Q) -> Option<PermissionState> {
        #[cfg(mobile)]
        if let Some(handle) = &self.0 {
            return match handle.run_mobile_plugin::<Reply>(method, args) {
                Ok(reply) => Some(reply.state),
                Err(error) => {
                    tracing::warn!("push: {method}: {error}");
                    None
                }
            };
        }
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_hex_token_and_a_wake_are_understood() {
        assert_eq!(
            parse_event(r#"{"kind":"token","token":"ab01CD"}"#),
            Some(NativeEvent::Token {
                token: "ab01CD".to_owned()
            })
        );
        assert_eq!(parse_event(r#"{"kind":"wake"}"#), Some(NativeEvent::Wake));
    }

    #[test]
    fn anything_else_is_dropped() {
        for bad in [
            r#"{"kind":"token","token":""}"#,
            r#"{"kind":"token","token":"not hex"}"#,
            r#"{"kind":"token"}"#,
            r#"{"kind":"preview","text":"1000 XCH"}"#,
            "[]",
            "nonsense",
        ] {
            assert_eq!(parse_event(bad), None, "{bad}");
        }
        let long = format!(r#"{{"kind":"token","token":"{}"}}"#, "a".repeat(4097));
        assert_eq!(parse_event(&long), None);
    }

    /// A wake-up carries no content: extra fields a hostile payload might add are ignored,
    /// never kept.
    #[test]
    fn a_wake_keeps_nothing_it_was_sent() {
        assert_eq!(
            parse_event(r#"{"kind":"wake","aps":{"alert":"send 5 XCH to xch1evil"}}"#),
            Some(NativeEvent::Wake)
        );
    }
}
