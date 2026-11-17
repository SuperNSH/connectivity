//! The reachability probe: a GET whose 204 means the Internet answered.

use std::time::Duration;

use crate::{backend::BackendFuture, types::ProbeOutcome};

/// What answers "is the Internet there". [`HttpProbe`] by default; a test passes its own.
pub trait Probe: Send + Sync {
    /// GETs `url` and classifies the answer. The core also bounds it by `timeout`.
    fn probe(&self, url: &str, timeout: Duration) -> BackendFuture<'_, ProbeOutcome>;
}

/// 204 is reachable; any other complete response is a portal answering for the Internet.
pub fn classify(status: u16) -> ProbeOutcome {
    if status == 204 {
        ProbeOutcome::Reachable
    } else {
        ProbeOutcome::CaptivePortal
    }
}

/// The HTTP probe: ureq with rustls on native targets, the browser's `fetch` on the web.
#[derive(Debug, Clone, Copy, Default)]
pub struct HttpProbe;

#[cfg(not(target_arch = "wasm32"))]
impl Probe for HttpProbe {
    fn probe(&self, url: &str, timeout: Duration) -> BackendFuture<'_, ProbeOutcome> {
        let url = url.to_owned();
        Box::pin(async move {
            crate::rt::blocking(move || get(&url, timeout))
                .await
                .unwrap_or(ProbeOutcome::Unreachable)
        })
    }
}

/// One GET with no redirects followed: a portal's 302 is its own answer.
#[cfg(not(target_arch = "wasm32"))]
fn get(url: &str, timeout: Duration) -> ProbeOutcome {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .build()
        .into();
    match agent.get(url).call() {
        Ok(response) => classify(response.status().as_u16()),
        Err(_) => ProbeOutcome::Unreachable,
    }
}

#[cfg(target_arch = "wasm32")]
impl Probe for HttpProbe {
    fn probe(&self, url: &str, timeout: Duration) -> BackendFuture<'_, ProbeOutcome> {
        let url = url.to_owned();
        Box::pin(async move { web_get(&url, timeout).await })
    }
}

/// `fetch(url, { signal: AbortSignal.timeout(ms) })` off the global, in a window or a worker.
/// A cross-origin URL must allow CORS, or the browser hides the status and this reads
/// unreachable.
#[cfg(target_arch = "wasm32")]
async fn web_get(url: &str, timeout: Duration) -> ProbeOutcome {
    use wasm_bindgen::{JsCast, JsValue};
    let global = js_sys::global();
    let Some(fetch) = js_sys::Reflect::get(&global, &JsValue::from_str("fetch"))
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
    else {
        return ProbeOutcome::Unreachable;
    };
    let init = web_sys::RequestInit::new();
    init.set_method("GET");
    let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    init.set_signal(Some(&web_sys::AbortSignal::timeout_with_u32(ms)));
    let Ok(promise) = fetch.call2(&global, &JsValue::from_str(url), &init) else {
        return ProbeOutcome::Unreachable;
    };
    let Ok(promise) = promise.dyn_into::<js_sys::Promise>() else {
        return ProbeOutcome::Unreachable;
    };
    match wasm_bindgen_futures::JsFuture::from(promise).await {
        Ok(response) => match response.dyn_into::<web_sys::Response>() {
            Ok(response) => classify(response.status()),
            Err(_) => ProbeOutcome::Unreachable,
        },
        Err(_) => ProbeOutcome::Unreachable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_204_is_reachable() {
        assert_eq!(classify(204), ProbeOutcome::Reachable);
        for status in [200, 302, 404, 500, 511] {
            assert_eq!(classify(status), ProbeOutcome::CaptivePortal);
        }
    }
}
