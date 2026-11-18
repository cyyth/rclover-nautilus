//! Client side of the Nuvolo D-Bus interface (contract: nuvolo-sync docs/dbus.rst).

use crate::ffi;
use gio::glib::{self, prelude::*};
use gio::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const BUS: &str = "io.github.cyythanapong_netizen.Nuvolo";
const PATH: &str = "/io/github/cyythanapong_netizen/Nuvolo";
const IFACE: &str = "io.github.cyythanapong_netizen.Nuvolo1";
/// State lookups run on the Nautilus UI thread, so they must give up quickly.
const QUERY_TIMEOUT_MS: i32 = 300;
const MOUNTS_TTL: Duration = Duration::from_secs(30);
const DIR_TTL: Duration = Duration::from_secs(60);

pub struct Daemon {
    conn: Option<gio::DBusConnection>,
    mounts: RefCell<(Option<Instant>, Vec<PathBuf>)>,
    dirs: RefCell<HashMap<PathBuf, (Instant, HashMap<String, String>)>>,
}

thread_local! {
    static DAEMON: Daemon = Daemon::connect();
}

pub fn with<R>(f: impl FnOnce(&Daemon) -> R) -> R {
    DAEMON.with(f)
}

impl Daemon {
    fn connect() -> Daemon {
        let conn = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).ok();
        if let Some(c) = &conn {
            c.signal_subscribe(Some(BUS), Some(IFACE), Some("StatesChanged"), Some(PATH), None, gio::DBusSignalFlags::NONE, |_, _, _, _, _, params| {
                let paths = params.get::<(Vec<String>,)>().map(|t| t.0).unwrap_or_default();
                with(|d| d.invalidate(&paths));
            });
        }
        Daemon { conn, mounts: RefCell::new((None, Vec::new())), dirs: RefCell::new(HashMap::new()) }
    }

    /// A blocking call that never starts Nuvolo: browsing must stay fast when it is not running.
    fn query(&self, method: &str, args: &glib::Variant) -> Option<glib::Variant> {
        self.conn
            .as_ref()?
            .call_sync(Some(BUS), PATH, IFACE, method, Some(args), None, gio::DBusCallFlags::NO_AUTO_START, QUERY_TIMEOUT_MS, None::<&gio::Cancellable>)
            .ok()
    }

    /// Whether `path` is inside a Nuvolo mount. The mount list is cached for 30 s.
    pub fn inside(&self, path: &Path) -> bool {
        let mut m = self.mounts.borrow_mut();
        if m.0.is_none_or(|t| t.elapsed() > MOUNTS_TTL) {
            m.1 = self
                .query("GetMounts", &().to_variant())
                .and_then(|v| v.get::<(Vec<String>,)>())
                .map(|t| t.0.into_iter().map(PathBuf::from).collect())
                .unwrap_or_default();
            m.0 = Some(Instant::now());
        }
        m.1.iter().any(|root| path.starts_with(root))
    }

    /// The state of `path`, fetched one directory at a time and cached.
    pub fn state(&self, path: &Path) -> Option<String> {
        if !self.inside(path) {
            return None;
        }
        let dir = path.parent()?.to_path_buf();
        let key = path.to_string_lossy().into_owned();
        if let Some((t, states)) = self.dirs.borrow().get(&dir) {
            if t.elapsed() < DIR_TTL {
                return states.get(&key).cloned();
            }
        }
        let states = self
            .query("GetDirStates", &(dir.to_string_lossy().into_owned(),).to_variant())
            .and_then(|v| v.get::<(HashMap<String, String>,)>())
            .map(|t| t.0)?;
        let found = states.get(&key).cloned();
        self.dirs.borrow_mut().insert(dir, (Instant::now(), states));
        found
    }

    pub fn states(&self, paths: &[String]) -> HashMap<String, String> {
        self.query("GetStates", &(paths.to_vec(),).to_variant())
            .and_then(|v| v.get::<(HashMap<String, String>,)>())
            .map(|t| t.0)
            .unwrap_or_default()
    }

    /// Fire-and-forget action call; this one may start Nuvolo.
    pub fn act(&self, method: &str, args: glib::Variant) {
        let Some(c) = &self.conn else { return };
        let method_name = method.to_owned();
        c.call(Some(BUS), PATH, IFACE, method, Some(&args), None, gio::DBusCallFlags::NONE, -1, None::<&gio::Cancellable>, move |res| {
            if let Err(e) = res {
                eprintln!("nuvolo-nautilus: {method_name}: {e}");
            }
        });
    }

    /// Asks for a URL, then hands it to `done` on the main thread.
    pub fn url(&self, method: &str, path: String, done: impl FnOnce(String) + 'static) {
        let Some(c) = &self.conn else { return };
        c.call(Some(BUS), PATH, IFACE, method, Some(&(path,).to_variant()), None, gio::DBusCallFlags::NONE, 30_000, None::<&gio::Cancellable>, move |res| {
            match res.map(|v| v.get::<(String,)>()) {
                Ok(Some((url,))) => done(url),
                Ok(None) => eprintln!("nuvolo-nautilus: unexpected reply"),
                Err(e) => eprintln!("nuvolo-nautilus: {e}"),
            }
        });
    }

    /// Drops cached states for `paths` and their ancestors and tells Nautilus to ask again.
    fn invalidate(&self, paths: &[String]) {
        let mut dirs = self.dirs.borrow_mut();
        for p in paths {
            for a in Path::new(p).ancestors() {
                if !self.mounts.borrow().1.iter().any(|root| a.starts_with(root)) {
                    break;
                }
                dirs.remove(a);
                if let Some(parent) = a.parent() {
                    dirs.remove(parent);
                }
                let Ok(uri) = CString::new(gio::File::for_path(a).uri().as_str()) else { continue };
                // SAFETY: lookup returns a new reference (or NULL), released below.
                unsafe {
                    let fi = ffi::nautilus_file_info_lookup_for_uri(uri.as_ptr());
                    if !fi.is_null() {
                        ffi::nautilus_file_info_invalidate_extension_info(fi);
                        glib::gobject_ffi::g_object_unref(fi.cast());
                    }
                }
            }
        }
    }
}
