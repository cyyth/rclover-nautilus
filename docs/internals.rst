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

* The list of mount points is cached for 30 seconds.
* States are fetched with ``GetDirStates``, once per folder, and cached for 60 seconds.
* State lookups use a 300 ms timeout and ``NO_AUTO_START``, because Nautilus calls them on
  its UI thread.
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
