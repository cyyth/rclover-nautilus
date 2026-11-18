Usage
=====

Emblems
-------

Inside a Nuvolo mount, each file and folder shows one emblem:

=============  ======================================================
Cloud          Only in the cloud. Uses no disk space until opened.
Check (green)  Cached on this device.
Half (purple)  A folder with some of its content on this device.
Pin (purple)   Always kept on this device.
Sync (blue)    Uploading or downloading now.
Warning        The last transfer failed.
=============  ======================================================

Files outside a Nuvolo mount are never touched. If Nuvolo is not running, no emblems are
shown and browsing is not slowed down. The extension never starts Nuvolo just to look up a
state.

The Nuvolo menu
---------------

Right-click files inside a mount and open **Nuvolo**:

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

Unlike state lookups, these actions start Nuvolo if it is not running.
