//! GNOME Files (Nautilus 43+) extension for Nuvolo.
//!
//! Adds a sync emblem to every file inside a Nuvolo mount and a "Nuvolo" submenu to the
//! context menu. All state lives in the Nuvolo app; this extension only asks it over D-Bus.

mod daemon;
mod ffi;
mod nautilus;

use gio::glib::{self, prelude::*, translate::IntoGlib};
use std::os::raw::c_int;
use std::sync::OnceLock;

static TYPES: OnceLock<[glib::ffi::GType; 1]> = OnceLock::new();

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
