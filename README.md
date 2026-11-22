# rclover-nautilus

A GNOME Files extension for [rclover](https://github.com/cyythanapong-netizen/rclover-sync).
It shows a sync emblem on every file in your cloud drives and adds a **rclover** menu to the
right-click menu: keep a file on this device, free its cache, sync now, copy a share link,
or open it in the browser.

## Install

Needs Rust, `nautilus-devel` (Nautilus 43 or newer) and a running rclover.

```sh
make
sudo make install
nautilus -q
```

## Documentation

Documentation: https://cyythanapong-netizen.github.io/rclover-nautilus/ (sources in [`docs/`](docs/), built with Sphinx).

## License

GPL-3.0-or-later
