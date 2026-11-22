//! GNOME Files (Nautilus 43+) extension for rclover.
//!
//! Adds a sync emblem to every file inside an rclover mount and a "rclover" submenu to the
//! context menu. All state lives in the rclover app; this extension only asks it over D-Bus.

mod daemon;
mod ffi;
mod nautilus;

use gio::glib::{self, prelude::*, translate::IntoGlib};
use std::os::raw::c_int;
use std::sync::OnceLock;

static TYPES: OnceLock<[glib::ffi::GType; 1]> = OnceLock::new();

/// Runs `f`, turning a panic into `fallback`. Every entry point reachable from Nautilus or
/// GLib goes through this: a panic must never cross into C, where it aborts GNOME Files.
pub(crate) fn guard<R>(fallback: R, f: impl FnOnce() -> R) -> R {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| {
        eprintln!("rclover-nautilus: internal error, ignored (please report it)");
        fallback
    })
}

// ponytail: the provider type is registered statically rather than on the GTypeModule.
// Nautilus never unloads extensions, so that is safe; switch to dynamic types if that changes.
#[no_mangle]
pub extern "C" fn nautilus_module_initialize(_module: *mut glib::gobject_ffi::GTypeModule) {
    TYPES.get_or_init(|| [nautilus::Provider::static_type().into_glib()]);
}

#[no_mangle]
pub extern "C" fn nautilus_module_shutdown() {}

/// # Safety
/// Called by Nautilus with valid out-pointers.
#[no_mangle]
pub unsafe extern "C" fn nautilus_module_list_types(types: *mut *const glib::ffi::GType, num_types: *mut c_int) {
    let t = TYPES.get_or_init(|| [nautilus::Provider::static_type().into_glib()]);
    *types = t.as_ptr();
    *num_types = t.len() as c_int;
}

#[cfg(test)]
mod tests {
    #[test]
    fn guard_contains_panics() {
        assert_eq!(super::guard(7, || -> i32 { panic!("boom") }), 7);
        assert_eq!(super::guard(7, || 1), 1);
    }
}
