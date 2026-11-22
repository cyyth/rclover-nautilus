Usage
=====

Emblems
-------

Inside an rclover mount, each file and folder shows one emblem:

=============  ======================================================
Cloud          Only in the cloud. Uses no disk space until opened.
Check (green)  Cached on this device.
Half (purple)  A folder with some of its content on this device.
Pin (purple)   Always kept on this device.
Sync (blue)    Uploading or downloading now.
Warning        The last transfer failed.
=============  ======================================================

Files reached through a symbolic link into a mount show their emblems and the menu too, for
example in a home folder moved to the cloud (``~/Documents`` linked to
``~/Cloud/gdrive/Documents``). Links are followed only up to the mount, once per folder.

Files outside an rclover mount are never touched. If rclover is not running, no emblems are
shown and browsing is not slowed down. The extension never starts rclover just to look up a
state.

The rclover menu
----------------

Right-click files inside a mount and open **rclover**:

Always Keep on Device / Stop Keeping on Device
   Pins or unpins the selection. Pinned files are downloaded right away.

Free Local Cache
   Unpins the selection so its cache space can be reclaimed.

Sync Folder Now / Sync Now
   Fetches the latest listing from the cloud.

Copy Share Link
   Creates a public link and copies it to the clipboard (one file only).

Open in Browser
   Opens the file in the provider's web interface (one file only).

Unlike state lookups, these actions start rclover if it is not running.
