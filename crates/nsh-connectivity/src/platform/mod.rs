//! The platform backends. Each target gets one `NativeBackend`; a target with none answers
//! `NotAvailable`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_vendor = "apple")]
mod apple;
#[cfg(target_os = "linux")]
mod linux;
// Also compiled under `test`, so its answers are tested on every host.
#[cfg(any(
    test,
    not(any(
        target_vendor = "apple",
        target_os = "android",
        target_os = "linux",
        target_os = "windows",
        target_arch = "wasm32"
    ))
))]
mod unsupported;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_os = "windows")]
mod windows;

use std::sync::Arc;

use crate::backend::Backend;

#[cfg(target_os = "android")]
use android::NativeBackend;
#[cfg(target_vendor = "apple")]
use apple::NativeBackend;
#[cfg(target_os = "linux")]
use linux::NativeBackend;
#[cfg(not(any(
    target_vendor = "apple",
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    target_arch = "wasm32"
)))]
use unsupported::NativeBackend;
#[cfg(target_arch = "wasm32")]
use web::NativeBackend;
#[cfg(target_os = "windows")]
use windows::NativeBackend;

/// The backend of the target this build runs on.
pub(crate) fn default_backend() -> Arc<dyn Backend> {
    Arc::new(NativeBackend)
}
