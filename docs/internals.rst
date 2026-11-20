Internals
=========

Nautilus loads extensions as C shared libraries. This one is a Rust ``cdylib`` that exports
the three module entry points (``nautilus_module_initialize``, ``nautilus_module_list_types``
and ``nautilus_module_shutdown``). It registers one GObject type that implements
``NautilusMenuProvider`` and ``NautilusInfoProvider``.

There are no maintained Rust bindings for libnautilus-extension-4, so the few functions and
interface structs it needs are declared by hand in ``src/ffi.rs``.

Talking to Nuvolo
-----------------

All state comes from the Nuvolo app over D-Bus. The interface is documented in the
nuvolo-sync repository (``docs/dbus.rst``).

* The list of mount points is fetched asynchronously and cached for 30 seconds (failed
  lookups are not cached for 30 seconds, so a slow startup recovers within seconds).
* ``update_file_info()`` never blocks Nautilus's UI thread. On a cache hit it applies the
  emblem immediately; on a miss it returns ``NAUTILUS_OPERATION_IN_PROGRESS``, batches the
  visible entries' ``(path, is_directory)`` pairs in an idle callback, calls
  ``GetStatesWithTypes`` asynchronously (falling back to ``GetDirStates`` on older Nuvolo
  versions), and finishes each request with ``nautilus_info_provider_update_complete_invoke``.
* Timed-out or failed lookups are retried with a short backoff and re-invalidate the
  affected files so emblems appear without a manual refresh.
* When Nuvolo emits ``StatesChanged``, the extension drops the cached states for those paths
  and their parent folders. In a later main-loop iteration it asks Nautilus to refresh
  those files, which updates the emblems. Nautilus answers a refresh by calling back into
  the extension immediately, so the refresh must never happen while a cache is in use.

Crash safety
------------

Every function Nautilus or GLib can call runs inside ``guard()``, which catches a Rust panic
and logs it. A panic that reaches C aborts the whole GNOME Files process, so a bug in the
extension now costs at most a missing emblem or menu.

Emblems
-------

``nautilus_file_info_add_emblem("nuvolo-pinned")`` makes Nautilus show the icon
``emblem-nuvolo-pinned``. The SVGs in ``icons/`` are installed into the hicolor theme's
``emblems`` folder.
