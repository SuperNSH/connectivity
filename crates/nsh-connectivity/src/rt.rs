//! Timers, tasks and blocking work for the core, on tokio (native) or the browser (wasm).

use std::{future::Future, time::Duration};

use crate::error::ConnectivityError;
#[cfg(not(target_arch = "wasm32"))]
use crate::error::ErrorKind;

/// Waits `delay`. On tokio's clock when a runtime is current (so paused-clock tests drive
/// it); without one, a thread times it so the caller still resolves.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn sleep(delay: Duration) {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::time::sleep(delay).await;
        return;
    }
    let (done, wait) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let _ = done.send(());
    });
    let _ = wait.await;
}

/// Waits `delay` on the browser's `setTimeout`, in a window or a worker.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn sleep(delay: Duration) {
    use wasm_bindgen::{JsCast, JsValue};
    let ms = delay.as_millis().min(i32::MAX as u128) as i32;
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let global = js_sys::global();
        let set_timeout = js_sys::Reflect::get(&global, &JsValue::from_str("setTimeout"))
            .ok()
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
        match set_timeout {
            Some(set_timeout) => {
                if set_timeout
                    .call2(&global, &resolve, &JsValue::from(ms))
                    .is_err()
                {
                    let _ = resolve.call0(&JsValue::NULL);
                }
            }
            // No timer at all: resolve now rather than hang.
            None => {
                let _ = resolve.call0(&JsValue::NULL);
            }
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Starts the core's event loop: on the current tokio runtime, which NSH enters while it
/// polls its tasks. Without one there is nowhere to run it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn spawn(
    task: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ConnectivityError> {
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| ConnectivityError::new(ErrorKind::NotAvailable))?;
    drop(handle.spawn(task));
    Ok(())
}

/// Starts the core's event loop on the page's event loop.
#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn(task: impl Future<Output = ()> + 'static) -> Result<(), ConnectivityError> {
    wasm_bindgen_futures::spawn_local(task);
    Ok(())
}

/// Runs blocking `work` off the async thread: tokio's blocking pool when a runtime is current
/// (it holds a paused clock still while it runs), else a thread.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn blocking<R: Send + 'static>(
    work: impl FnOnce() -> R + Send + 'static,
) -> Option<R> {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return handle.spawn_blocking(work).await.ok();
    }
    let (done, wait) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = done.send(work());
    });
    wait.await.ok()
}
