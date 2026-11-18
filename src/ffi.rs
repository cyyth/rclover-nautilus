//! Hand-written bindings for the parts of libnautilus-extension-4 this extension uses.

use gio::glib::ffi::{GList, GType};
use gio::glib::gobject_ffi::{GClosure, GTypeInterface};
use std::os::raw::{c_char, c_int};

#[repr(C)]
pub struct NautilusFileInfo {
    _private: [u8; 0],
}
#[repr(C)]
pub struct NautilusMenuProvider {
    _private: [u8; 0],
}
#[repr(C)]
pub struct NautilusInfoProvider {
    _private: [u8; 0],
}
#[repr(C)]
pub struct NautilusMenuItem {
    _private: [u8; 0],
}
#[repr(C)]
pub struct NautilusMenu {
    _private: [u8; 0],
}
#[repr(C)]
pub struct NautilusOperationHandle {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct NautilusMenuProviderInterface {
    pub g_iface: GTypeInterface,
    pub get_file_items: Option<unsafe extern "C" fn(*mut NautilusMenuProvider, *mut GList) -> *mut GList>,
    pub get_background_items: Option<unsafe extern "C" fn(*mut NautilusMenuProvider, *mut NautilusFileInfo) -> *mut GList>,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct NautilusInfoProviderInterface {
    pub g_iface: GTypeInterface,
    pub update_file_info: Option<
        unsafe extern "C" fn(*mut NautilusInfoProvider, *mut NautilusFileInfo, *mut GClosure, *mut *mut NautilusOperationHandle) -> c_int,
    >,
    pub cancel_update: Option<unsafe extern "C" fn(*mut NautilusInfoProvider, *mut NautilusOperationHandle)>,
}

pub const NAUTILUS_OPERATION_COMPLETE: c_int = 0;

extern "C" {
    pub fn nautilus_menu_provider_get_type() -> GType;
    pub fn nautilus_info_provider_get_type() -> GType;
    pub fn nautilus_file_info_get_location(file: *mut NautilusFileInfo) -> *mut gio::ffi::GFile;
    pub fn nautilus_file_info_add_emblem(file: *mut NautilusFileInfo, emblem_name: *const c_char);
    pub fn nautilus_file_info_invalidate_extension_info(file: *mut NautilusFileInfo);
    pub fn nautilus_file_info_lookup_for_uri(uri: *const c_char) -> *mut NautilusFileInfo;
    pub fn nautilus_menu_item_new(name: *const c_char, label: *const c_char, tip: *const c_char, icon: *const c_char) -> *mut NautilusMenuItem;
    pub fn nautilus_menu_item_set_submenu(item: *mut NautilusMenuItem, menu: *mut NautilusMenu);
    pub fn nautilus_menu_new() -> *mut NautilusMenu;
    pub fn nautilus_menu_append_item(menu: *mut NautilusMenu, item: *mut NautilusMenuItem);
}
