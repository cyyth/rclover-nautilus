//! GObject glue: the Nautilus provider interfaces and the Nuvolo provider that implements them.

use crate::{daemon, ffi};
use gdk::prelude::*;
use gio::glib::{self, subclass::prelude::*, translate::*};
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::ptr;

glib::wrapper! {
    pub struct MenuProvider(Interface<ffi::NautilusMenuProvider, ffi::NautilusMenuProviderInterface>);
    match fn { type_ => || ffi::nautilus_menu_provider_get_type(), }
}

glib::wrapper! {
    pub struct InfoProvider(Interface<ffi::NautilusInfoProvider, ffi::NautilusInfoProviderInterface>);
    match fn { type_ => || ffi::nautilus_info_provider_get_type(), }
}

/// A borrowed `NautilusFileInfo`, valid for the duration of one callback.
pub struct FileInfo(*mut ffi::NautilusFileInfo);

impl FileInfo {
    pub fn path(&self) -> Option<PathBuf> {
        // SAFETY: get_location returns a new reference.
        let f: gio::File = unsafe { from_glib_full(ffi::nautilus_file_info_get_location(self.0)) };
        f.path()
    }

    pub fn is_directory(&self) -> bool {
        // SAFETY: `self.0` is valid for the duration of the callback.
        unsafe { ffi::nautilus_file_info_is_directory(self.0) != 0 }
    }
}

pub trait MenuProviderImpl: ObjectImpl {
    fn file_items(&self, files: &[FileInfo]) -> Vec<glib::Object>;
}

pub trait InfoProviderImpl: ObjectImpl {}

unsafe impl<T: MenuProviderImpl> IsImplementable<T> for MenuProvider {
    fn interface_init(iface: &mut glib::Interface<Self>) {
        let iface = iface.as_mut();
        iface.get_file_items = Some(get_file_items::<T>);
    }
}

unsafe impl<T: InfoProviderImpl> IsImplementable<T> for InfoProvider {
    fn interface_init(iface: &mut glib::Interface<Self>) {
        let iface = iface.as_mut();
        iface.update_file_info = Some(update_file_info);
        iface.cancel_update = Some(cancel_update);
    }
}

unsafe extern "C" fn get_file_items<T: MenuProviderImpl>(provider: *mut ffi::NautilusMenuProvider, files: *mut glib::ffi::GList) -> *mut glib::ffi::GList {
    crate::guard(ptr::null_mut(), || file_items_list::<T>(provider, files))
}

unsafe fn file_items_list<T: MenuProviderImpl>(provider: *mut ffi::NautilusMenuProvider, files: *mut glib::ffi::GList) -> *mut glib::ffi::GList {
    let imp = (*(provider as *mut T::Instance)).imp();
    let mut infos = Vec::new();
    let mut l = files;
    while !l.is_null() {
        infos.push(FileInfo((*l).data.cast()));
        l = (*l).next;
    }
    let mut out = ptr::null_mut();
    for item in imp.file_items(&infos) {
        // Nautilus takes ownership of the list and its references.
        let raw: *mut glib::gobject_ffi::GObject = item.into_glib_ptr();
        out = glib::ffi::g_list_append(out, raw.cast());
    }
    out
}

unsafe extern "C" fn update_file_info(
    provider: *mut ffi::NautilusInfoProvider,
    file: *mut ffi::NautilusFileInfo,
    update_complete: *mut glib::gobject_ffi::GClosure,
    handle: *mut *mut ffi::NautilusOperationHandle,
) -> std::os::raw::c_int {
    crate::guard(ffi::NAUTILUS_OPERATION_COMPLETE, || {
        let info = FileInfo(file);
        let Some(path) = info.path() else {
            return ffi::NAUTILUS_OPERATION_COMPLETE;
        };
        let is_dir = info.is_directory();
        if let Some(id) = daemon::with(|d| d.request_info(provider, file, update_complete, path, is_dir)) {
            if !handle.is_null() {
                *handle = id as *mut ffi::NautilusOperationHandle;
            }
            ffi::NAUTILUS_OPERATION_IN_PROGRESS
        } else {
            ffi::NAUTILUS_OPERATION_COMPLETE
        }
    })
}

unsafe extern "C" fn cancel_update(_provider: *mut ffi::NautilusInfoProvider, handle: *mut ffi::NautilusOperationHandle) {
    crate::guard((), || {
        if !handle.is_null() {
            daemon::with(|d| d.cancel_op(handle as usize));
        }
    });
}

fn menu_item(name: &str, label: &str, on_activate: impl Fn() + 'static) -> glib::Object {
    let (n, l) = (CString::new(name).unwrap(), CString::new(label).unwrap());
    // SAFETY: nautilus_menu_item_new returns a new reference.
    let item: glib::Object = unsafe { from_glib_full(ffi::nautilus_menu_item_new(n.as_ptr(), l.as_ptr(), ptr::null(), ptr::null()).cast::<glib::gobject_ffi::GObject>()) };
    item.connect_local("activate", false, move |_| {
        crate::guard((), &on_activate);
        None
    });
    item
}

pub(crate) fn emblem(state: &str) -> Option<&'static str> {
    Some(match state {
        "cloud" => "nuvolo-cloud",
        "cached" => "nuvolo-cached",
        "pinned" => "nuvolo-pinned",
        "syncing" => "nuvolo-syncing",
        "partial" => "nuvolo-partial",
        "error" => "nuvolo-error",
        _ => return None,
    })
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Provider;

    #[glib::object_subclass]
    impl ObjectSubclass for Provider {
        const NAME: &'static str = "NuvoloNautilusProvider";
        type Type = super::Provider;
        type ParentType = glib::Object;
        type Interfaces = (MenuProvider, InfoProvider);
    }

    impl ObjectImpl for Provider {}

    impl InfoProviderImpl for Provider {}

    impl MenuProviderImpl for Provider {
        fn file_items(&self, files: &[FileInfo]) -> Vec<glib::Object> {
            let paths: Vec<String> = files.iter().filter_map(|f| f.path()).map(|p| p.to_string_lossy().into_owned()).collect();
            if paths.is_empty() || paths.len() != files.len() || !daemon::with(|d| paths.iter().all(|p| d.inside(Path::new(p)))) {
                return Vec::new();
            }
            let states = daemon::with(|d| d.states(&paths));
            let all_pinned = paths.iter().all(|p| states.get(p).is_some_and(|s| s == "pinned"));
            let is_dir = files.len() == 1 && files[0].is_directory();

            let act = |method: &'static str| {
                let paths = paths.clone();
                move || daemon::with(|d| d.act(method, (paths.clone(),).to_variant()))
            };
            let mut sub = vec![
                if all_pinned {
                    menu_item("Nuvolo::unpin", "Stop Keeping on Device", act("Unpin"))
                } else {
                    menu_item("Nuvolo::pin", "Always Keep on Device", act("Pin"))
                },
                menu_item("Nuvolo::free", "Free Local Cache", act("Unpin")),
                menu_item("Nuvolo::sync", if is_dir { "Sync Folder Now" } else { "Sync Now" }, act("SyncNow")),
            ];
            if let [path] = paths.as_slice() {
                let p = path.clone();
                sub.push(menu_item("Nuvolo::link", "Copy Share Link", move || {
                    daemon::with(|d| {
                        d.url("GetShareLink", p.clone(), |url| {
                            if let Some(display) = gdk::Display::default() {
                                display.clipboard().set_text(&url);
                            }
                        })
                    })
                }));
                let p = path.clone();
                sub.push(menu_item("Nuvolo::web", "Open in Browser", move || {
                    daemon::with(|d| {
                        d.url("GetWebUrl", p.clone(), |url| {
                            if let Err(e) = gio::AppInfo::launch_default_for_uri(&url, None::<&gio::AppLaunchContext>) {
                                eprintln!("nuvolo-nautilus: {e}");
                            }
                        })
                    })
                }));
            }

            let top = menu_item("Nuvolo::top", "Nuvolo", || {});
            // SAFETY: menu_new returns a new reference; append and set_submenu take their own.
            unsafe {
                let menu: glib::Object = from_glib_full(ffi::nautilus_menu_new().cast::<glib::gobject_ffi::GObject>());
                for item in &sub {
                    ffi::nautilus_menu_append_item(menu.as_ptr().cast(), item.as_ptr().cast());
                }
                ffi::nautilus_menu_item_set_submenu(top.as_ptr().cast(), menu.as_ptr().cast());
            }
            vec![top]
        }
    }
}

glib::wrapper! {
    pub struct Provider(ObjectSubclass<imp::Provider>) @implements MenuProvider, InfoProvider;
}
