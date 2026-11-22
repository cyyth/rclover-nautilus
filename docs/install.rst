Installation
============

Requirements
------------

* GNOME Files (Nautilus) 43 or newer
* Rust 1.84 or newer
* ``libnautilus-extension-4`` development files
* `rclover <https://github.com/cyythanapong-netizen/rclover-sync>`_, installed with its
  D-Bus service file

On Fedora::

   sudo dnf install cargo nautilus-devel

Build and install
-----------------

Build as your normal user, then install as root::

   make
   sudo make install
   nautilus -q

This installs:

* ``librclover-nautilus.so`` into Nautilus' extension folder
  (``pkg-config --variable=extensiondir libnautilus-extension-4``)
* the emblem icons into ``/usr/share/icons/hicolor/scalable/emblems``

``nautilus -q`` closes GNOME Files so the next window loads the extension.

Uninstall
---------

::

   sudo make uninstall
   nautilus -q
