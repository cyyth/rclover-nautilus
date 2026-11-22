//! End-to-end test for `rclover-nautilus` against a live mock `Rclover1` D-Bus service on an
//! isolated private `dbus-run-session` bus.
//!
//! Exercises the real exported C ABI (`nautilus_module_initialize`, `nautilus_module_list_types`,
//! `NautilusInfoProvider::update_file_info` and `cancel_update`) with a `GObject` implementing
//! `NautilusFileInfoInterface` and real floating `GClosure` callbacks, verifying:
//! 1. Non-blocking async lookup (`NAUTILUS_OPERATION_IN_PROGRESS` -> `NAUTILUS_OPERATION_COMPLETE`)
//!    via `GetMounts` + batched `GetStatesWithTypes`, adding `"rclover-pinned"` without
//!    double-freeing the floating closure.
//! 2. Immediate synchronous cache hit (`NAUTILUS_OPERATION_COMPLETE`) on the second query.
//! 3. Safe `cancel_update` without invoking the completion callback or double-freeing.
//! 4. `StatesChanged` signal invalidating the cache so subsequent queries re-fetch.

use gio::glib::{self, prelude::*, subclass::prelude::*, translate::*};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::ptr;
use std::rc::Rc;

const BUS: &str = "io.github.cyythanapong_netizen.Rclover";
const OBJ_PATH: &str = "/io/github/cyythanapong_netizen/Rclover";
const IFACE: &str = "io.github.cyythanapong_netizen.Rclover1";

const NAUTILUS_OPERATION_COMPLETE: c_int = 0;
const NAUTILUS_OPERATION_IN_PROGRESS: c_int = 2;

const DBUS_CONF: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;

const XML: &str = r#"<node>
  <interface name="io.github.cyythanapong_netizen.Rclover1">
    <method name="GetMounts"><arg type="as" direction="out"/></method>
    <method name="GetStates"><arg name="paths" type="as" direction="in"/><arg type="a{ss}" direction="out"/></method>
    <method name="GetStatesWithTypes"><arg name="entries" type="a(sb)" direction="in"/><arg type="a{ss}" direction="out"/></method>
    <method name="GetDirStates"><arg name="dir" type="s" direction="in"/><arg type="a{ss}" direction="out"/></method>
    <signal name="StatesChanged"><arg name="paths" type="as"/></signal>
  </interface>
</node>"#;

#[repr(C)]
pub struct NautilusFileInfo {
    _private: [u8; 0],
}
#[repr(C)]
struct NautilusInfoProvider {
    _private: [u8; 0],
}
#[repr(C)]
struct NautilusOperationHandle {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct NautilusFileInfoInterface {
    g_iface: glib::gobject_ffi::GTypeInterface,
    is_gone: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> glib::ffi::gboolean>,
    get_name: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> *mut c_char>,
    get_uri: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> *mut c_char>,
    get_parent_uri: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> *mut c_char>,
    get_uri_scheme: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> *mut c_char>,
    get_mime_type: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> *mut c_char>,
    is_mime_type: Option<unsafe extern "C" fn(*mut NautilusFileInfo, *const c_char) -> glib::ffi::gboolean>,
    is_directory: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> glib::ffi::gboolean>,
    add_emblem: Option<unsafe extern "C" fn(*mut NautilusFileInfo, *const c_char)>,
    get_string_attribute: Option<unsafe extern "C" fn(*mut NautilusFileInfo, *const c_char) -> *mut c_char>,
    add_string_attribute: Option<unsafe extern "C" fn(*mut NautilusFileInfo, *const c_char, *const c_char)>,
    invalidate_extension_info: Option<unsafe extern "C" fn(*mut NautilusFileInfo)>,
    get_activation_uri: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> *mut c_char>,
    get_file_type: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> gio::ffi::GFileType>,
    get_location: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> *mut gio::ffi::GFile>,
    get_parent_location: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> *mut gio::ffi::GFile>,
    get_parent_info: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> *mut NautilusFileInfo>,
    get_mount: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> *mut gio::ffi::GMount>,
    can_write: Option<unsafe extern "C" fn(*mut NautilusFileInfo) -> glib::ffi::gboolean>,
}

#[repr(C)]
struct NautilusInfoProviderInterface {
    g_iface: glib::gobject_ffi::GTypeInterface,
    update_file_info: Option<
        unsafe extern "C" fn(
            *mut NautilusInfoProvider,
            *mut NautilusFileInfo,
            *mut glib::gobject_ffi::GClosure,
            *mut *mut NautilusOperationHandle,
        ) -> c_int,
    >,
    cancel_update: Option<unsafe extern "C" fn(*mut NautilusInfoProvider, *mut NautilusOperationHandle)>,
}

extern "C" {
    fn nautilus_file_info_get_type() -> glib::ffi::GType;
    fn nautilus_info_provider_get_type() -> glib::ffi::GType;
    static mut nautilus_file_info_getter: Option<
        unsafe extern "C" fn(*mut gio::ffi::GFile, glib::ffi::gboolean) -> *mut NautilusFileInfo,
    >;
}

unsafe extern "C" fn test_file_info_getter(_location: *mut gio::ffi::GFile, _create: glib::ffi::gboolean) -> *mut NautilusFileInfo {
    ptr::null_mut()
}

glib::wrapper! {
    pub struct FileInfoIface(Interface<NautilusFileInfo, NautilusFileInfoInterface>);
    match fn { type_ => || nautilus_file_info_get_type(), }
}

unsafe impl<T: ObjectImpl> IsImplementable<T> for FileInfoIface
where
    T: FakeFileImpl,
{
    fn interface_init(iface: &mut glib::Interface<Self>) {
        let iface = iface.as_mut();
        iface.get_location = Some(fake_get_location::<T>);
        iface.is_directory = Some(fake_is_directory::<T>);
        iface.add_emblem = Some(fake_add_emblem::<T>);
    }
}

trait FakeFileImpl: ObjectImpl {
    fn location(&self) -> gio::File;
    fn is_dir(&self) -> bool;
    fn record_emblem(&self, name: String);
}

unsafe extern "C" fn fake_get_location<T: FakeFileImpl>(fi: *mut NautilusFileInfo) -> *mut gio::ffi::GFile {
    let imp = (*(fi as *mut T::Instance)).imp();
    imp.location().into_glib_ptr()
}

unsafe extern "C" fn fake_is_directory<T: FakeFileImpl>(fi: *mut NautilusFileInfo) -> glib::ffi::gboolean {
    let imp = (*(fi as *mut T::Instance)).imp();
    glib::ffi::gboolean::from(imp.is_dir())
}

unsafe extern "C" fn fake_add_emblem<T: FakeFileImpl>(fi: *mut NautilusFileInfo, emblem: *const c_char) {
    let imp = (*(fi as *mut T::Instance)).imp();
    let s = CStr::from_ptr(emblem).to_string_lossy().into_owned();
    imp.record_emblem(s);
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct FakeFile {
        pub path: RefCell<PathBuf>,
        pub is_dir: Cell<bool>,
        pub emblems: RefCell<Vec<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FakeFile {
        const NAME: &'static str = "RcloverTestFakeFile";
        type Type = super::FakeFile;
        type ParentType = glib::Object;
        type Interfaces = (FileInfoIface,);
    }

    impl ObjectImpl for FakeFile {}

    impl FakeFileImpl for FakeFile {
        fn location(&self) -> gio::File {
            gio::File::for_path(self.path.borrow().as_path())
        }
        fn is_dir(&self) -> bool {
            self.is_dir.get()
        }
        fn record_emblem(&self, name: String) {
            self.emblems.borrow_mut().push(name);
        }
    }
}

glib::wrapper! {
    pub struct FakeFile(ObjectSubclass<imp::FakeFile>) @implements FileInfoIface;
}

impl FakeFile {
    fn new(path: &str, is_dir: bool) -> Self {
        let obj: Self = glib::Object::new();
        *obj.imp().path.borrow_mut() = PathBuf::from(path);
        obj.imp().is_dir.set(is_dir);
        obj
    }

    fn as_nautilus_ptr(&self) -> *mut NautilusFileInfo {
        self.as_ptr().cast()
    }

    fn emblems(&self) -> Vec<String> {
        self.imp().emblems.borrow().clone()
    }
}

struct CallbackState {
    called: Cell<bool>,
    result: Cell<c_int>,
    loop_: glib::MainLoop,
}

/// Marshal function attached to our floating `GClosure` to observe
/// `nautilus_info_provider_update_complete_invoke(closure, provider, handle, result)` and
/// release Nautilus's own floating reference—matching real GNOME Files behavior.
unsafe extern "C" fn test_closure_marshal(
    closure: *mut glib::gobject_ffi::GClosure,
    _return_value: *mut glib::gobject_ffi::GValue,
    n_param_values: c_uint,
    param_values: *const glib::gobject_ffi::GValue,
    _invocation_hint: *mut c_void,
    marshal_data: *mut c_void,
) {
    let state = &*(marshal_data as *const CallbackState);
    // Params from nautilus_info_provider_update_complete_invoke: (provider, handle, result).
    if n_param_values >= 3 {
        let res_val = &*param_values.add(2);
        let res = glib::gobject_ffi::g_value_get_enum(res_val);
        state.result.set(res);
    }
    state.called.set(true);
    // Simulate Nautilus releasing its floating reference on completion.
    glib::gobject_ffi::g_closure_unref(closure);
    state.loop_.quit();
}

fn have_tool(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[test]
fn e2e_async_emblems_cache_cancel_and_invalidation() {
    if !have_tool("dbus-run-session") {
        eprintln!("skipping e2e test: requires dbus-run-session");
        return;
    }

    if std::env::var_os("RCLOVER_NAUTILUS_E2E_CHILD").is_none() {
        let dbus_conf = std::env::temp_dir().join(format!("rclover-nautilus-e2e-dbus-{}.conf", std::process::id()));
        std::fs::write(&dbus_conf, DBUS_CONF).unwrap();
        let exe = std::env::current_exe().expect("current_exe");
        let status = Command::new("dbus-run-session")
            .arg("--config-file")
            .arg(&dbus_conf)
            .arg("--")
            .arg(exe)
            .args(["--exact", "e2e_async_emblems_cache_cancel_and_invalidation", "--nocapture"])
            .env("RCLOVER_NAUTILUS_E2E_CHILD", "1")
            .env("GIO_USE_VFS", "local")
            .status()
            .expect("launch dbus-run-session");
        let _ = std::fs::remove_file(&dbus_conf);
        assert!(status.success(), "e2e child failed inside dbus-run-session: {status}");
        return;
    }

    let main_loop = glib::MainLoop::new(None, false);
    let conn = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).expect("connect private session bus");

    let own_reply = conn
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&(BUS, 0u32).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5_000,
            None::<&gio::Cancellable>,
        )
        .expect("RequestName");
    assert_eq!(own_reply.get::<(u32,)>(), Some((1,)), "must become primary owner of {BUS}");

    let mount_dir = "/tmp/rclover-nautilus-e2e-mount";
    let file_a = format!("{mount_dir}/pinned.txt");
    let file_b = format!("{mount_dir}/cancelled.txt");

    let mounts_calls = Rc::new(Cell::new(0u32));
    let states_calls = Rc::new(Cell::new(0u32));
    let (mc, sc) = (mounts_calls.clone(), states_calls.clone());

    let node = gio::DBusNodeInfo::for_xml(XML).expect("valid XML");
    let iface_info = node.lookup_interface(IFACE).expect("iface in XML");
    let _reg = conn
        .register_object(OBJ_PATH, &iface_info)
        .method_call(move |_, _, _, _, method, params, inv| match method {
            "GetMounts" => {
                mc.set(mc.get() + 1);
                inv.return_value(Some(&(vec![mount_dir.to_owned()],).to_variant()));
            }
            "GetStatesWithTypes" => {
                sc.set(sc.get() + 1);
                let entries: Vec<(String, bool)> = params.get::<(Vec<(String, bool)>,)>().map(|t| t.0).unwrap_or_default();
                let mut map = HashMap::new();
                for (p, _) in entries {
                    map.insert(p, "pinned".to_owned());
                }
                inv.return_value(Some(&(map,).to_variant()));
            }
            _ => inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
        })
        .build()
        .expect("register mock Rclover1 object");

    // Initialize the real extension via its exported C module ABI.
    unsafe { nautilus_file_info_getter = Some(test_file_info_getter) };
    rclover_nautilus::nautilus_module_initialize(ptr::null_mut());
    let mut types_ptr: *const glib::ffi::GType = ptr::null();
    let mut num_types: c_int = 0;
    unsafe { rclover_nautilus::nautilus_module_list_types(&mut types_ptr, &mut num_types) };
    assert_eq!(num_types, 1);
    let provider_type = unsafe { *types_ptr };

    unsafe {
        let provider_obj = glib::gobject_ffi::g_object_new(provider_type, ptr::null::<c_char>());
        assert!(!provider_obj.is_null());
        let provider = provider_obj as *mut NautilusInfoProvider;
        let class_ptr = *(provider_obj as *mut *mut glib::gobject_ffi::GTypeClass);
        let iface_ptr = glib::gobject_ffi::g_type_interface_peek(class_ptr.cast(), nautilus_info_provider_get_type())
            as *const NautilusInfoProviderInterface;
        assert!(!iface_ptr.is_null(), "provider must implement NautilusInfoProvider");
        let update_fn = (*iface_ptr).update_file_info.expect("update_file_info");
        let cancel_fn = (*iface_ptr).cancel_update.expect("cancel_update");

        // 1. Cold lookup for file_a: returns IN_PROGRESS, completes async with "rclover-pinned".
        let fi_a = FakeFile::new(&file_a, false);
        let cb_state = Box::new(CallbackState {
            called: Cell::new(false),
            result: Cell::new(-1),
            loop_: main_loop.clone(),
        });
        let closure = glib::gobject_ffi::g_closure_new_simple(64, ptr::null_mut());
        assert!(!closure.is_null());
        glib::gobject_ffi::g_closure_set_meta_marshal(
            closure,
            (&*cb_state as *const CallbackState) as *mut c_void,
            Some(test_closure_marshal),
        );

        let mut handle: *mut NautilusOperationHandle = ptr::null_mut();
        let res = update_fn(provider, fi_a.as_nautilus_ptr(), closure, &mut handle);
        assert_eq!(res, NAUTILUS_OPERATION_IN_PROGRESS, "cold lookup must not block UI");
        assert!(!handle.is_null(), "in-progress lookup must set a handle");

        let timeout_loop = main_loop.clone();
        let timeout_id = glib::timeout_add_seconds_local_once(5, move || timeout_loop.quit());
        main_loop.run();
        if cb_state.called.get() {
            timeout_id.remove();
        }
        assert!(cb_state.called.get(), "async completion callback must fire");
        assert_eq!(cb_state.result.get(), NAUTILUS_OPERATION_COMPLETE);
        assert_eq!(fi_a.emblems(), vec!["rclover-pinned".to_owned()]);
        assert_eq!(mounts_calls.get(), 1, "GetMounts called once");
        assert_eq!(states_calls.get(), 1, "GetStatesWithTypes called once");

        // 2. Second lookup for file_a: immediate synchronous cache hit.
        let fi_a2 = FakeFile::new(&file_a, false);
        let closure2 = glib::gobject_ffi::g_closure_new_simple(64, ptr::null_mut());
        let mut handle2: *mut NautilusOperationHandle = ptr::null_mut();
        let res2 = update_fn(provider, fi_a2.as_nautilus_ptr(), closure2, &mut handle2);
        assert_eq!(res2, NAUTILUS_OPERATION_COMPLETE, "cached lookup must complete immediately");
        assert_eq!(fi_a2.emblems(), vec!["rclover-pinned".to_owned()]);
        glib::gobject_ffi::g_closure_unref(closure2);
        assert_eq!(states_calls.get(), 1, "no D-Bus call on cache hit");

        // 3. Lookup file_b then cancel immediately: must not invoke callback or double-free.
        let fi_b = FakeFile::new(&file_b, false);
        let cb_cancel = Box::new(CallbackState {
            called: Cell::new(false),
            result: Cell::new(-1),
            loop_: main_loop.clone(),
        });
        let closure_b = glib::gobject_ffi::g_closure_new_simple(64, ptr::null_mut());
        glib::gobject_ffi::g_closure_set_meta_marshal(
            closure_b,
            (&*cb_cancel as *const CallbackState) as *mut c_void,
            Some(test_closure_marshal),
        );
        let mut handle_b: *mut NautilusOperationHandle = ptr::null_mut();
        let res_b = update_fn(provider, fi_b.as_nautilus_ptr(), closure_b, &mut handle_b);
        assert_eq!(res_b, NAUTILUS_OPERATION_IN_PROGRESS);
        assert!(!handle_b.is_null());
        cancel_fn(provider, handle_b);
        glib::gobject_ffi::g_closure_unref(closure_b);

        let drain_loop = main_loop.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || drain_loop.quit());
        main_loop.run();
        assert!(!cb_cancel.called.get(), "cancelled operation must not invoke completion callback");

        // 4. Emit StatesChanged for file_a and verify cache is invalidated and re-fetched.
        conn.emit_signal(None, OBJ_PATH, IFACE, "StatesChanged", Some(&(vec![file_a.clone()],).to_variant()))
            .expect("emit StatesChanged");
        let drain_loop2 = main_loop.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || drain_loop2.quit());
        main_loop.run();

        let fi_a3 = FakeFile::new(&file_a, false);
        let cb_state3 = Box::new(CallbackState {
            called: Cell::new(false),
            result: Cell::new(-1),
            loop_: main_loop.clone(),
        });
        let closure3 = glib::gobject_ffi::g_closure_new_simple(64, ptr::null_mut());
        glib::gobject_ffi::g_closure_set_meta_marshal(
            closure3,
            (&*cb_state3 as *const CallbackState) as *mut c_void,
            Some(test_closure_marshal),
        );
        let mut handle3: *mut NautilusOperationHandle = ptr::null_mut();
        let res3 = update_fn(provider, fi_a3.as_nautilus_ptr(), closure3, &mut handle3);
        assert_eq!(res3, NAUTILUS_OPERATION_IN_PROGRESS, "invalidated entry must re-query");
        let timeout_loop3 = main_loop.clone();
        let tid3 = glib::timeout_add_seconds_local_once(5, move || timeout_loop3.quit());
        main_loop.run();
        if cb_state3.called.get() {
            tid3.remove();
        }
        assert!(cb_state3.called.get());
        assert_eq!(fi_a3.emblems(), vec!["rclover-pinned".to_owned()]);
        assert_eq!(states_calls.get(), 2, "StatesChanged triggered a fresh GetStatesWithTypes call");

        glib::gobject_ffi::g_object_unref(provider_obj.cast());
    }
}
