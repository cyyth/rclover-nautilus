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

    pub fn add_emblem(&self, name: &str) {
        let n = CString::new(name).expect("emblem name has no NUL");
        unsafe { ffi::nautilus_file_info_add_emblem(self.0, n.as_ptr()) }
    }
}

pub trait MenuProviderImpl: ObjectImpl {
    fn file_items(&self, files: &[FileInfo]) -> Vec<glib::Object>;
}

pub trait InfoProviderImpl: ObjectImpl {
    fn update_file_info(&self, file: &FileInfo);
}

unsafe impl<T: MenuProviderImpl> IsImplementable<T> for MenuProvider {
    fn interface_init(iface: &mut glib::Interface<Self>) {
        let iface = iface.as_mut();
        iface.get_file_items = Some(get_file_items::<T>);
    }
}

unsafe impl<T: InfoProviderImpl> IsImplementable<T> for InfoProvider {
    fn interface_init(iface: &mut glib::Interface<Self>) {
        let iface = iface.as_mut();
        iface.update_file_info = Some(update_file_info::<T>);
    }
}

unsafe extern "C" fn get_file_items<T: MenuProviderImpl>(provider: *mut ffi::NautilusMenuProvider, files: *mut glib::ffi::GList) -> *mut glib::ffi::GList {
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

unsafe extern "C" fn update_file_info<T: InfoProviderImpl>(
    provider: *mut ffi::NautilusInfoProvider,
    file: *mut ffi::NautilusFileInfo,
    _update_complete: *mut glib::gobject_ffi::GClosure,
    _handle: *mut *mut ffi::NautilusOperationHandle,
) -> std::os::raw::c_int {
    (*(provider as *mut T::Instance)).imp().update_file_info(&FileInfo(file));
    ffi::NAUTILUS_OPERATION_COMPLETE
}

fn menu_item(name: &str, label: &str, on_activate: impl Fn() + 'static) -> glib::Object {
    let (n, l) = (CString::new(name).unwrap(), CString::new(label).unwrap());
    // SAFETY: nautilus_menu_item_new returns a new reference.
    let item: glib::Object = unsafe { from_glib_full(ffi::nautilus_menu_item_new(n.as_ptr(), l.as_ptr(), ptr::null(), ptr::null()).cast::<glib::gobject_ffi::GObject>()) };
    item.connect_local("activate", false, move |_| {
        on_activate();
        None
    });
    item
}

fn emblem(state: &str) -> Option<&'static str> {
    Some(match state {
        "cloud" => "nuvolo-cloud",
        "cached" => "nuvolo-cached",
        "pinned" => "nuvolo-pinned",
        "syncing" => "nuvolo-syncing",
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

    impl InfoProviderImpl for Provider {
        fn update_file_info(&self, file: &FileInfo) {
            let Some(path) = file.path() else { return };
            if let Some(e) = daemon::with(|d| d.state(&path)).as_deref().and_then(emblem) {
                file.add_emblem(e);
            }
        }
    }

    impl MenuProviderImpl for Provider {
        fn file_items(&self, files: &[FileInfo]) -> Vec<glib::Object> {
            let paths: Vec<String> = files.iter().filter_map(|f| f.path()).map(|p| p.to_string_lossy().into_owned()).collect();
            if paths.is_empty() || paths.len() != files.len() || !daemon::with(|d| paths.iter().all(|p| d.inside(Path::new(p)))) {
                return Vec::new();
            }
            let states = daemon::with(|d| d.states(&paths));
            let all_pinned = paths.iter().all(|p| states.get(p).is_some_and(|s| s == "pinned"));
            let is_dir = paths.len() == 1 && Path::new(&paths[0]).is_dir();

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
