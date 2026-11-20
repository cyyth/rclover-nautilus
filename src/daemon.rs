//! Client side of the Nuvolo D-Bus interface (contract: nuvolo-sync docs/dbus.rst).

use crate::ffi;
use crate::nautilus::emblem;
use gio::glib::{self, prelude::*};
use gio::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const BUS: &str = "io.github.cyythanapong_netizen.Nuvolo";
const PATH: &str = "/io/github/cyythanapong_netizen/Nuvolo";
const IFACE: &str = "io.github.cyythanapong_netizen.Nuvolo1";
/// Short timeout for synchronous context-menu fallback lookups.
const SYNC_QUERY_TIMEOUT_MS: i32 = 300;
/// Generous timeout for non-blocking async D-Bus lookups.
const ASYNC_QUERY_TIMEOUT_MS: i32 = 5_000;
const MOUNTS_TTL: Duration = Duration::from_secs(30);
/// Short cache when Nuvolo is not running so we notice promptly once it starts.
const NO_DAEMON_TTL: Duration = Duration::from_secs(3);
const STATE_TTL: Duration = Duration::from_secs(60);
const MAX_RETRIES: u32 = 3;

/// Cached mount list state.
#[derive(Clone, Debug)]
enum MountCache {
    /// Never fetched yet, or last fetch failed transiently (retry right away).
    Unknown,
    /// Fetched successfully (may be empty if no accounts configured).
    Known(Instant, Vec<PathBuf>),
    /// Nuvolo daemon is not currently running on the bus; re-check after [`NO_DAEMON_TTL`].
    Absent(Instant),
}

impl MountCache {
    fn roots_if_fresh(&self) -> Option<&[PathBuf]> {
        match self {
            Self::Known(t, roots) if t.elapsed() < MOUNTS_TTL => Some(roots),
            Self::Absent(t) if t.elapsed() < NO_DAEMON_TTL => Some(&[]),
            _ => None,
        }
    }

    fn known_roots(&self) -> Vec<PathBuf> {
        match self {
            Self::Known(_, roots) => roots.clone(),
            _ => Vec::new(),
        }
    }
}

/// An in-progress async `NautilusInfoProvider::update_file_info` request.
struct PendingOp {
    id: usize,
    provider: *mut ffi::NautilusInfoProvider,
    file: *mut ffi::NautilusFileInfo,
    closure: *mut glib::gobject_ffi::GClosure,
    path: PathBuf,
    is_dir: bool,
}

impl PendingOp {
    /// Takes strong references to the Nautilus/GLib objects so they stay valid until
    /// completed or cancelled.
    unsafe fn new(
        id: usize,
        provider: *mut ffi::NautilusInfoProvider,
        file: *mut ffi::NautilusFileInfo,
        closure: *mut glib::gobject_ffi::GClosure,
        path: PathBuf,
        is_dir: bool,
    ) -> Self {
        glib::gobject_ffi::g_object_ref(provider.cast());
        glib::gobject_ffi::g_object_ref(file.cast());
        glib::gobject_ffi::g_closure_ref(closure);
        glib::gobject_ffi::g_closure_sink(closure);
        Self { id, provider, file, closure, path, is_dir }
    }

    /// Applies `state` (if any) as an emblem and notifies Nautilus that this async op finished.
    fn finish(self, state: Option<&str>, result: c_int) {
        if let Some(e) = state.and_then(emblem) {
            if let Ok(cstr) = CString::new(e) {
                // SAFETY: `self.file` is ref-counted and live until Drop.
                unsafe { ffi::nautilus_file_info_add_emblem(self.file, cstr.as_ptr()) };
            }
        }
        // SAFETY: `self.closure` and `self.provider` are ref-counted and live until Drop.
        unsafe {
            ffi::nautilus_info_provider_update_complete_invoke(
                self.closure,
                self.provider,
                self.id as *mut ffi::NautilusOperationHandle,
                result,
            );
        }
        // Dropping `self` unrefs closure, file, and provider.
    }
}

impl Drop for PendingOp {
    fn drop(&mut self) {
        unsafe {
            glib::gobject_ffi::g_closure_unref(self.closure);
            glib::gobject_ffi::g_object_unref(self.file.cast());
            glib::gobject_ffi::g_object_unref(self.provider.cast());
        }
    }
}

pub struct Daemon {
    conn: Option<gio::DBusConnection>,
    mounts: RefCell<MountCache>,
    mounts_in_flight: Cell<bool>,
    /// Per-path cached state, populated from batched `GetStatesWithTypes` or `GetDirStates`.
    states: RefCell<HashMap<PathBuf, (Instant, String)>>,
    /// In-flight `update_file_info` operations keyed by opaque handle id.
    pending: RefCell<HashMap<usize, PendingOp>>,
    next_id: Cell<usize>,
    /// Pending op ids waiting for the initial/refreshed `GetMounts` reply.
    waiting_mounts: RefCell<Vec<usize>>,
    /// Pending op ids queued for the next idle-coalesced state batch.
    queued_batch: RefCell<Vec<usize>>,
    batch_scheduled: Cell<bool>,
    /// Retry count per path after transient failures/timeouts.
    retries: RefCell<HashMap<PathBuf, u32>>,
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
                crate::guard((), || with(|d| d.invalidate(&paths)));
            });
        }
        Daemon {
            conn,
            mounts: RefCell::new(MountCache::Unknown),
            mounts_in_flight: Cell::new(false),
            states: RefCell::new(HashMap::new()),
            pending: RefCell::new(HashMap::new()),
            next_id: Cell::new(1),
            waiting_mounts: RefCell::new(Vec::new()),
            queued_batch: RefCell::new(Vec::new()),
            batch_scheduled: Cell::new(false),
            retries: RefCell::new(HashMap::new()),
        }
    }

    fn alloc_id(&self) -> usize {
        let id = self.next_id.get();
        let next = id.wrapping_add(1);
        self.next_id.set(if next == 0 { 1 } else { next });
        id
    }

    /// A short blocking call used only as a fallback for context-menu checks.
    fn query_sync(&self, method: &str, args: &glib::Variant) -> Option<glib::Variant> {
        self.conn
            .as_ref()?
            .call_sync(Some(BUS), PATH, IFACE, method, Some(args), None, gio::DBusCallFlags::NO_AUTO_START, SYNC_QUERY_TIMEOUT_MS, None::<&gio::Cancellable>)
            .ok()
    }

    /// Whether `path` is inside a Nuvolo mount (for synchronous context-menu checks).
    /// Never caches a transient failure as an empty mount list for 30 s.
    pub fn inside(&self, path: &Path) -> bool {
        let Ok(mut cache) = self.mounts.try_borrow_mut() else { return false };
        if let Some(roots) = cache.roots_if_fresh() {
            return roots.iter().any(|root| path.starts_with(root));
        }
        let Some(conn) = &self.conn else { return false };
        match conn.call_sync(
            Some(BUS),
            PATH,
            IFACE,
            "GetMounts",
            Some(&().to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            SYNC_QUERY_TIMEOUT_MS,
            None::<&gio::Cancellable>,
        ) {
            Ok(v) => {
                let roots: Vec<PathBuf> = v
                    .get::<(Vec<String>,)>()
                    .map(|t| t.0.into_iter().map(PathBuf::from).collect())
                    .unwrap_or_default();
                let hit = roots.iter().any(|root| path.starts_with(root));
                *cache = MountCache::Known(Instant::now(), roots);
                hit
            }
            Err(e) if is_service_absent(&e) => {
                *cache = MountCache::Absent(Instant::now());
                false
            }
            Err(_) => {
                // Transient timeout/error: do not poison the cache for 30 s.
                *cache = MountCache::Unknown;
                false
            }
        }
    }

    /// Looks up a fresh cached state without performing any D-Bus I/O.
    pub fn cached_state(&self, path: &Path) -> Option<String> {
        let states = self.states.try_borrow().ok()?;
        let (t, state) = states.get(path)?;
        (t.elapsed() < STATE_TTL).then(|| state.clone())
    }

    /// Non-blocking entry point for `NautilusInfoProvider::update_file_info`.
    ///
    /// Returns `None` when handled immediately (outside mounts, or cache hit with emblem
    /// already applied), or `Some(handle_id)` when an async lookup has been queued and
    /// Nautilus should expect `nautilus_info_provider_update_complete_invoke` later.
    pub unsafe fn request_info(
        &self,
        provider: *mut ffi::NautilusInfoProvider,
        file: *mut ffi::NautilusFileInfo,
        closure: *mut glib::gobject_ffi::GClosure,
        path: PathBuf,
        is_dir: bool,
    ) -> Option<usize> {
        self.conn.as_ref()?;
        // Check mount cache without blocking.
        let mount_decision = {
            let Ok(cache) = self.mounts.try_borrow() else { return None };
            cache.roots_if_fresh().map(|roots| roots.iter().any(|r| path.starts_with(r)))
        };
        match mount_decision {
            Some(false) => None, // known to be outside every mount
            Some(true) => {
                if let Some(state) = self.cached_state(&path) {
                    if let Some(e) = emblem(&state) {
                        if let Ok(cstr) = CString::new(e) {
                            ffi::nautilus_file_info_add_emblem(file, cstr.as_ptr());
                        }
                    }
                    return None;
                }
                let id = self.alloc_id();
                let op = PendingOp::new(id, provider, file, closure, path, is_dir);
                let Ok(mut pending) = self.pending.try_borrow_mut() else { return None };
                pending.insert(id, op);
                drop(pending);
                self.enqueue_batch_id(id);
                Some(id)
            }
            None => {
                // Mount list not known yet (or expired): fetch GetMounts asynchronously.
                let id = self.alloc_id();
                let op = PendingOp::new(id, provider, file, closure, path, is_dir);
                let Ok(mut pending) = self.pending.try_borrow_mut() else { return None };
                pending.insert(id, op);
                drop(pending);
                if let Ok(mut w) = self.waiting_mounts.try_borrow_mut() {
                    w.push(id);
                }
                self.fetch_mounts_async();
                Some(id)
            }
        }
    }

    /// Cancels an in-progress `update_file_info` operation without invoking its completion closure.
    pub fn cancel_op(&self, id: usize) {
        if let Ok(mut pending) = self.pending.try_borrow_mut() {
            pending.remove(&id);
        }
    }

    fn fetch_mounts_async(&self) {
        if self.mounts_in_flight.replace(true) {
            return;
        }
        let Some(conn) = &self.conn else {
            self.mounts_in_flight.set(false);
            return;
        };
        conn.call(
            Some(BUS),
            PATH,
            IFACE,
            "GetMounts",
            Some(&().to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            ASYNC_QUERY_TIMEOUT_MS,
            None::<&gio::Cancellable>,
            move |res| {
                crate::guard((), || with(|d| d.on_mounts_reply(res)));
            },
        );
    }

    fn on_mounts_reply(&self, res: Result<glib::Variant, glib::Error>) {
        self.mounts_in_flight.set(false);
        let waiting: Vec<usize> = self.waiting_mounts.borrow_mut().drain(..).collect();
        match res {
            Ok(v) => {
                let roots: Vec<PathBuf> = v
                    .get::<(Vec<String>,)>()
                    .map(|t| t.0.into_iter().map(PathBuf::from).collect())
                    .unwrap_or_default();
                *self.mounts.borrow_mut() = MountCache::Known(Instant::now(), roots.clone());
                let mut to_complete = Vec::new();
                let mut to_batch = Vec::new();
                {
                    let mut pending = self.pending.borrow_mut();
                    for id in waiting {
                        let Some(op) = pending.get(&id) else { continue };
                        if !roots.iter().any(|r| op.path.starts_with(r)) {
                            if let Some(op) = pending.remove(&id) {
                                to_complete.push((op, None));
                            }
                        } else if let Some(state) = self.cached_state(&op.path) {
                            if let Some(op) = pending.remove(&id) {
                                to_complete.push((op, Some(state)));
                            }
                        } else {
                            to_batch.push(id);
                        }
                    }
                }
                for (op, state) in to_complete {
                    op.finish(state.as_deref(), ffi::NAUTILUS_OPERATION_COMPLETE);
                }
                for id in to_batch {
                    self.enqueue_batch_id(id);
                }
            }
            Err(e) if is_service_absent(&e) => {
                *self.mounts.borrow_mut() = MountCache::Absent(Instant::now());
                let ops: Vec<PendingOp> = {
                    let mut pending = self.pending.borrow_mut();
                    waiting.into_iter().filter_map(|id| pending.remove(&id)).collect()
                };
                for op in ops {
                    op.finish(None, ffi::NAUTILUS_OPERATION_COMPLETE);
                }
            }
            Err(_) => {
                // Transient timeout/error: do not cache empty mounts; schedule retry.
                *self.mounts.borrow_mut() = MountCache::Unknown;
                let ops: Vec<PendingOp> = {
                    let mut pending = self.pending.borrow_mut();
                    waiting.into_iter().filter_map(|id| pending.remove(&id)).collect()
                };
                let retry_paths: Vec<PathBuf> = ops.iter().map(|op| op.path.clone()).collect();
                for op in ops {
                    op.finish(None, ffi::NAUTILUS_OPERATION_FAILED);
                }
                self.schedule_retry(retry_paths);
            }
        }
    }

    fn enqueue_batch_id(&self, id: usize) {
        if let Ok(mut q) = self.queued_batch.try_borrow_mut() {
            q.push(id);
        }
        if !self.batch_scheduled.replace(true) {
            glib::idle_add_local_once(|| {
                crate::guard((), || with(|d| d.flush_batch()));
            });
        }
    }

    fn flush_batch(&self) {
        self.batch_scheduled.set(false);
        let ids: Vec<usize> = self.queued_batch.borrow_mut().drain(..).collect();
        let mut live_ids = Vec::new();
        let mut entries: Vec<(String, bool)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        {
            let pending = self.pending.borrow();
            for id in ids {
                if let Some(op) = pending.get(&id) {
                    live_ids.push(id);
                    let key = op.path.to_string_lossy().into_owned();
                    if seen.insert(key.clone()) {
                        entries.push((key, op.is_dir));
                    }
                }
            }
        }
        if live_ids.is_empty() {
            return;
        }
        let Some(conn) = &self.conn else {
            let ops: Vec<PendingOp> = {
                let mut pending = self.pending.borrow_mut();
                live_ids.into_iter().filter_map(|id| pending.remove(&id)).collect()
            };
            for op in ops {
                op.finish(None, ffi::NAUTILUS_OPERATION_FAILED);
            }
            return;
        };
        conn.call(
            Some(BUS),
            PATH,
            IFACE,
            "GetStatesWithTypes",
            Some(&(entries,).to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            ASYNC_QUERY_TIMEOUT_MS,
            None::<&gio::Cancellable>,
            move |res| {
                crate::guard((), || with(|d| d.on_states_reply(live_ids, res)));
            },
        );
    }

    fn on_states_reply(&self, ids: Vec<usize>, res: Result<glib::Variant, glib::Error>) {
        match res {
            Ok(v) => {
                let map = v.get::<(HashMap<String, String>,)>().map(|t| t.0).unwrap_or_default();
                let now = Instant::now();
                {
                    let mut cache = self.states.borrow_mut();
                    let mut retries = self.retries.borrow_mut();
                    for (k, st) in &map {
                        let pb = PathBuf::from(k);
                        cache.insert(pb.clone(), (now, st.clone()));
                        retries.remove(&pb);
                    }
                }
                let ops: Vec<PendingOp> = {
                    let mut pending = self.pending.borrow_mut();
                    ids.into_iter().filter_map(|id| pending.remove(&id)).collect()
                };
                for op in ops {
                    let key = op.path.to_string_lossy();
                    let st = map.get(key.as_ref()).map(String::as_str);
                    op.finish(st, ffi::NAUTILUS_OPERATION_COMPLETE);
                }
            }
            Err(e) if e.message().contains("UnknownMethod") => {
                // Older nuvolo-sync without GetStatesWithTypes: fall back to async GetDirStates.
                self.fallback_dir_states(ids);
            }
            Err(_) => {
                let ops: Vec<PendingOp> = {
                    let mut pending = self.pending.borrow_mut();
                    ids.into_iter().filter_map(|id| pending.remove(&id)).collect()
                };
                let retry_paths: Vec<PathBuf> = ops.iter().map(|op| op.path.clone()).collect();
                for op in ops {
                    op.finish(None, ffi::NAUTILUS_OPERATION_FAILED);
                }
                self.schedule_retry(retry_paths);
            }
        }
    }

    fn fallback_dir_states(&self, ids: Vec<usize>) {
        let mut by_dir: HashMap<PathBuf, Vec<usize>> = HashMap::new();
        {
            let pending = self.pending.borrow();
            for id in ids {
                if let Some(op) = pending.get(&id) {
                    if let Some(dir) = op.path.parent() {
                        by_dir.entry(dir.to_path_buf()).or_default().push(id);
                    }
                }
            }
        }
        let Some(conn) = &self.conn else { return };
        for (dir, dir_ids) in by_dir {
            conn.call(
                Some(BUS),
                PATH,
                IFACE,
                "GetDirStates",
                Some(&(dir.to_string_lossy().into_owned(),).to_variant()),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                ASYNC_QUERY_TIMEOUT_MS,
                None::<&gio::Cancellable>,
                move |res| {
                    crate::guard((), || with(|d| d.on_states_reply(dir_ids, res)));
                },
            );
        }
    }

    fn schedule_retry(&self, paths: Vec<PathBuf>) {
        let mut due_uris: HashMap<u64, Vec<String>> = HashMap::new();
        {
            let mut retries = self.retries.borrow_mut();
            for p in paths {
                let count = retries.entry(p.clone()).or_insert(0);
                if *count >= MAX_RETRIES {
                    continue;
                }
                *count += 1;
                let delay_secs = 1u64 << (*count - 1); // 1 s, 2 s, 4 s
                due_uris.entry(delay_secs).or_default().push(gio::File::for_path(&p).uri().to_string());
            }
        }
        for (delay, mut uris) in due_uris {
            uris.sort();
            uris.dedup();
            glib::timeout_add_seconds_local_once(delay as u32, move || {
                crate::guard((), || invalidate_uris(uris));
            });
        }
    }

    /// Context-menu state lookup: serves from cache when all paths are cached, otherwise falls
    /// back to a short synchronous `GetStates` call.
    pub fn states(&self, paths: &[String]) -> HashMap<String, String> {
        let mut out = HashMap::new();
        let mut missing = Vec::new();
        for p in paths {
            if let Some(st) = self.cached_state(Path::new(p)) {
                out.insert(p.clone(), st);
            } else {
                missing.push(p.clone());
            }
        }
        if !missing.is_empty() {
            if let Some(fetched) = self
                .query_sync("GetStates", &(missing,).to_variant())
                .and_then(|v| v.get::<(HashMap<String, String>,)>())
                .map(|t| t.0)
            {
                let now = Instant::now();
                if let Ok(mut cache) = self.states.try_borrow_mut() {
                    for (k, v) in &fetched {
                        cache.insert(PathBuf::from(k), (now, v.clone()));
                    }
                }
                out.extend(fetched);
            }
        }
        out
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
                Ok(Some((url,))) => crate::guard((), move || done(url)),
                Ok(None) => eprintln!("nuvolo-nautilus: unexpected reply"),
                Err(e) => eprintln!("nuvolo-nautilus: {e}"),
            }
        });
    }

    /// Drops cached states for `paths` and their ancestors and tells Nautilus to ask again.
    ///
    /// Nautilus answers an invalidation by calling `update_file_info` right away, which reads
    /// the caches. So the caches are updated and released first, and Nautilus is told in a
    /// later main-loop iteration, never while this extension is in the middle of a call.
    fn invalidate(&self, paths: &[String]) {
        let roots = self.mounts.borrow().known_roots();
        let mut uris = Vec::new();
        {
            let mut states = self.states.borrow_mut();
            let mut retries = self.retries.borrow_mut();
            for p in paths {
                for a in Path::new(p).ancestors().take_while(|a| roots.iter().any(|r| a.starts_with(r))) {
                    states.remove(a);
                    retries.remove(a);
                    uris.push(gio::File::for_path(a).uri().to_string());
                }
            }
        }
        uris.sort();
        uris.dedup();
        glib::idle_add_local_once(move || {
            crate::guard((), || invalidate_uris(uris));
        });
    }
}

fn invalidate_uris(uris: Vec<String>) {
    for uri in uris {
        let Ok(uri) = CString::new(uri) else { continue };
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

fn is_service_absent(err: &glib::Error) -> bool {
    let msg = err.message();
    msg.contains("ServiceUnknown") || msg.contains("NameHasNoOwner")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offline() -> Daemon {
        let d = Daemon {
            conn: None,
            mounts: RefCell::new(MountCache::Known(Instant::now(), vec![PathBuf::from("/mnt/cloud")])),
            mounts_in_flight: Cell::new(false),
            states: RefCell::new(HashMap::new()),
            pending: RefCell::new(HashMap::new()),
            next_id: Cell::new(1),
            waiting_mounts: RefCell::new(Vec::new()),
            queued_batch: RefCell::new(Vec::new()),
            batch_scheduled: Cell::new(false),
            retries: RefCell::new(HashMap::new()),
        };
        d.states
            .borrow_mut()
            .insert(PathBuf::from("/mnt/cloud/a.txt"), (Instant::now(), "cached".to_owned()));
        d
    }

    #[test]
    fn state_lookup_while_cache_is_borrowed_does_not_panic() {
        let d = offline();
        let _held = d.states.borrow_mut();
        assert_eq!(d.cached_state(Path::new("/mnt/cloud/a.txt")), None);
    }

    #[test]
    fn state_comes_from_the_cache_and_checks_mounts() {
        let d = offline();
        assert!(d.inside(Path::new("/mnt/cloud/a.txt")));
        assert!(!d.inside(Path::new("/elsewhere/a.txt")));
        assert_eq!(d.cached_state(Path::new("/mnt/cloud/a.txt")).as_deref(), Some("cached"));
        assert_eq!(d.cached_state(Path::new("/mnt/cloud/missing.txt")), None);
    }

    #[test]
    fn transient_mount_error_does_not_cache_empty_mounts() {
        let cache = MountCache::Unknown;
        assert!(cache.roots_if_fresh().is_none(), "unknown mounts must trigger a fresh lookup");
        let absent = MountCache::Absent(Instant::now() - Duration::from_secs(5));
        assert!(absent.roots_if_fresh().is_none(), "absent daemon cache expires quickly");
    }
}
