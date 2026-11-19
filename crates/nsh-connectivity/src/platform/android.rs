//! Android: `ConnectivityManager` over JNI (jni 0.21), with the VM and the application from
//! `ndk-context`, installed once through libnativehelper when the host left it empty.
//!
//! The queries need `ACCESS_NETWORK_STATE`, which NSH 0.6.0's generated manifest does not
//! declare; without it Android throws `SecurityException`, reported as `PermissionDenied`.
//! Change events need a `NetworkCallback` subclass, which is Java code (a crate can embed a dex
//! and load it, as nsh-secure-storage does) and `registerDefaultNetworkCallback` needs the same
//! `ACCESS_NETWORK_STATE`, so `watch` answers `NotAvailable` until NSH declares it.

use std::{
    ffi::{c_char, c_int, c_void},
    mem::transmute,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::null_mut,
    sync::{Mutex, PoisonError},
};

use jni::{
    JNIEnv, JavaVM,
    objects::{JObject, JString, JValue},
    sys::{self, jint, jsize},
};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    backend::{Backend, BackendFuture, PlatformEvent},
    error::{ConnectivityError, ErrorKind},
    types::{ConnectionType, InterfaceInfo, PlatformSnapshot},
};

pub(crate) struct NativeBackend;

// android.net.NetworkCapabilities.TRANSPORT_* and NET_CAPABILITY_NOT_METERED.
const TRANSPORT_CELLULAR: jint = 0;
const TRANSPORT_WIFI: jint = 1;
const TRANSPORT_BLUETOOTH: jint = 2;
const TRANSPORT_ETHERNET: jint = 3;
const TRANSPORT_VPN: jint = 4;
const TRANSPORT_WIFI_AWARE: jint = 5;
const TRANSPORT_LOWPAN: jint = 6;
const TRANSPORT_USB: jint = 8;
const TRANSPORT_THREAD: jint = 9;
const TRANSPORT_SATELLITE: jint = 10;
const NET_CAPABILITY_NOT_METERED: jint = 11;

/// Strongest first; VPN last so it follows the transport it runs over.
const TRANSPORTS: [(jint, ConnectionType); 10] = [
    (TRANSPORT_ETHERNET, ConnectionType::Ethernet),
    (TRANSPORT_WIFI, ConnectionType::Wifi),
    (TRANSPORT_CELLULAR, ConnectionType::Cellular),
    (TRANSPORT_SATELLITE, ConnectionType::Satellite),
    (TRANSPORT_BLUETOOTH, ConnectionType::Bluetooth),
    (TRANSPORT_USB, ConnectionType::Ethernet),
    (TRANSPORT_WIFI_AWARE, ConnectionType::Other),
    (TRANSPORT_LOWPAN, ConnectionType::Other),
    (TRANSPORT_THREAD, ConnectionType::Other),
    (TRANSPORT_VPN, ConnectionType::Vpn),
];

/// `JNI_GetCreatedJavaVMs`, found by name in libnativehelper.
type GetCreatedVms = unsafe extern "C" fn(*mut *mut sys::JavaVM, jsize, *mut jsize) -> jint;

// libdl, linked by default on Android.
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// `RTLD_NOW` from bionic's dlfcn.h.
const RTLD_NOW: c_int = 2;

/// Set once `ndk-context` holds the VM and the application.
static CONTEXT: Mutex<bool> = Mutex::new(false);

impl From<jni::errors::Error> for ConnectivityError {
    fn from(_: jni::errors::Error) -> Self {
        ErrorKind::Backend.into()
    }
}

/// The types a network's transports give, strongest first, VPN last.
fn types_from(has_transport: impl Fn(jint) -> bool) -> Vec<ConnectionType> {
    let mut types = Vec::new();
    for (transport, connection) in TRANSPORTS {
        if has_transport(transport) && !types.contains(&connection) {
            types.push(connection);
        }
    }
    types
}

/// The VM of this process, through the runtime's own library.
fn find_vm() -> Result<*mut sys::JavaVM, ConnectivityError> {
    // SAFETY: valid C strings; the symbol is called with the JNI specification's signature.
    unsafe {
        let library = dlopen(c"libnativehelper.so".as_ptr(), RTLD_NOW);
        if library.is_null() {
            return Err(ErrorKind::NotAvailable.into());
        }
        let symbol = dlsym(library, c"JNI_GetCreatedJavaVMs".as_ptr());
        if symbol.is_null() {
            return Err(ErrorKind::NotAvailable.into());
        }
        let get_created_vms = transmute::<*mut c_void, GetCreatedVms>(symbol);
        let mut vm: *mut sys::JavaVM = null_mut();
        let mut count: jsize = 0;
        if get_created_vms(&mut vm, 1, &mut count) != 0 || count < 1 || vm.is_null() {
            return Err(ErrorKind::NotAvailable.into());
        }
        Ok(vm)
    }
}

/// Gives `ndk-context` the VM and the application once; a context the host set is adopted.
fn ensure_context() -> Result<(), ConnectivityError> {
    let mut ready = CONTEXT.lock().unwrap_or_else(PoisonError::into_inner);
    if *ready {
        return Ok(());
    }
    // ndk-context has no try-get and panics while empty; a silent hook keeps that quiet.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let probe = catch_unwind(AssertUnwindSafe(ndk_context::android_context));
    std::panic::set_hook(previous);
    if probe.is_ok() {
        *ready = true;
        return Ok(());
    }
    let vm = find_vm()?;
    // SAFETY: the pointer is the runtime's own VM, alive for the process.
    let java_vm = unsafe { JavaVM::from_raw(vm) }?;
    let mut env = java_vm.attach_current_thread()?;
    let application = env
        .call_static_method(
            "android/app/ActivityThread",
            "currentApplication",
            "()Landroid/app/Application;",
            &[],
        )?
        .l()?;
    if application.is_null() {
        return Err(ErrorKind::NotAvailable.into());
    }
    let global = env.new_global_ref(application)?;
    let raw = global.as_obj().as_raw();
    // ndk-context keeps the reference for the life of the process.
    std::mem::forget(global);
    // SAFETY: the VM and the application are this process's, and nothing is installed.
    unsafe { ndk_context::initialize_android_context(vm.cast(), raw.cast()) };
    *ready = true;
    Ok(())
}

/// A failed call: a pending `SecurityException` is the missing permission; anything else
/// is the platform failing. The exception is cleared either way.
fn failure(env: &mut JNIEnv<'_>, error: jni::errors::Error) -> ConnectivityError {
    if !matches!(error, jni::errors::Error::JavaException) {
        return error.into();
    }
    let denied = env
        .exception_occurred()
        .ok()
        .filter(|throwable| !throwable.is_null())
        .map(|throwable| {
            let _ = env.exception_clear();
            env.is_instance_of(&throwable, "java/lang/SecurityException")
                .unwrap_or(false)
        })
        .unwrap_or(false);
    let _ = env.exception_clear();
    if denied {
        ErrorKind::PermissionDenied.into()
    } else {
        ErrorKind::Backend.into()
    }
}

/// Reads the active network's capabilities on this thread.
fn query() -> Result<PlatformSnapshot, ConnectivityError> {
    ensure_context()?;
    let context = ndk_context::android_context();
    // SAFETY: the VM is the one ndk-context holds, alive for the process.
    let vm = unsafe { JavaVM::from_raw(context.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    // SAFETY: ndk-context holds a global reference to the application.
    let application = unsafe { JObject::from_raw(context.context().cast()) };
    // A JObject does not delete its reference on drop, so ndk-context's global stays intact.
    read(&mut env, &application).map_err(|error| failure(&mut env, error))
}

fn read(
    env: &mut JNIEnv<'_>,
    application: &JObject<'_>,
) -> Result<PlatformSnapshot, jni::errors::Error> {
    let name = env.new_string("connectivity")?;
    let manager = env
        .call_method(
            application,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&name)],
        )?
        .l()?;
    if manager.is_null() {
        return Ok(PlatformSnapshot::default());
    }
    let network = env
        .call_method(&manager, "getActiveNetwork", "()Landroid/net/Network;", &[])?
        .l()?;
    if network.is_null() {
        return Ok(PlatformSnapshot::default());
    }
    let capabilities = env
        .call_method(
            &manager,
            "getNetworkCapabilities",
            "(Landroid/net/Network;)Landroid/net/NetworkCapabilities;",
            &[JValue::Object(&network)],
        )?
        .l()?;
    if capabilities.is_null() {
        return Ok(PlatformSnapshot::default());
    }
    let mut transports = Vec::new();
    for (transport, _) in TRANSPORTS {
        let has = env
            .call_method(
                &capabilities,
                "hasTransport",
                "(I)Z",
                &[JValue::Int(transport)],
            )?
            .z()?;
        if has {
            transports.push(transport);
        }
    }
    let unmetered = env
        .call_method(
            &capabilities,
            "hasCapability",
            "(I)Z",
            &[JValue::Int(NET_CAPABILITY_NOT_METERED)],
        )?
        .z()?;
    let types = types_from(|transport| transports.contains(&transport));
    let interface = interface_name(env, &manager, &network)?.map(|name| InterfaceInfo {
        name,
        r#type: types.first().copied().unwrap_or(ConnectionType::Unknown),
        ip_address: None,
        subnet: None,
        // The SSID needs a location permission NSH cannot declare; see docs/decisions.md.
        ssid: None,
    });
    Ok(PlatformSnapshot {
        types,
        interface,
        is_expensive: !unmetered,
    })
}

/// `LinkProperties.getInterfaceName()` of the network, such as `wlan0`.
fn interface_name(
    env: &mut JNIEnv<'_>,
    manager: &JObject<'_>,
    network: &JObject<'_>,
) -> Result<Option<String>, jni::errors::Error> {
    let properties = env
        .call_method(
            manager,
            "getLinkProperties",
            "(Landroid/net/Network;)Landroid/net/LinkProperties;",
            &[JValue::Object(network)],
        )?
        .l()?;
    if properties.is_null() {
        return Ok(None);
    }
    let name = env
        .call_method(&properties, "getInterfaceName", "()Ljava/lang/String;", &[])?
        .l()?;
    if name.is_null() {
        return Ok(None);
    }
    let name = JString::from(name);
    let name: String = env.get_string(&name)?.into();
    Ok(Some(name))
}

impl Backend for NativeBackend {
    fn snapshot(&self) -> BackendFuture<'_, Result<PlatformSnapshot, ConnectivityError>> {
        Box::pin(async {
            crate::rt::blocking(query)
                .await
                .unwrap_or_else(|| Err(ErrorKind::Backend.into()))
        })
    }

    fn watch(&self, _sink: UnboundedSender<PlatformEvent>) -> Result<(), ConnectivityError> {
        Err(ErrorKind::NotAvailable.into())
    }

    fn stop_watching(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transports_map_strongest_first_with_vpn_last() {
        let types =
            types_from(|t| [TRANSPORT_VPN, TRANSPORT_WIFI, TRANSPORT_CELLULAR].contains(&t));
        assert_eq!(
            types,
            vec![
                ConnectionType::Wifi,
                ConnectionType::Cellular,
                ConnectionType::Vpn
            ]
        );
        assert!(types_from(|_| false).is_empty());
        assert_eq!(
            types_from(|t| t == TRANSPORT_SATELLITE),
            vec![ConnectionType::Satellite]
        );
    }
}
