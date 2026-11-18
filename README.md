# nuvolo-nautilus

A GNOME Files extension for [Nuvolo](https://github.com/cyythanapong-netizen/nuvolo-sync).
It shows a sync emblem on every file in your cloud drives and adds a **Nuvolo** menu to the
right-click menu: keep a file on this device, free its cache, sync now, copy a share link,
or open it in the browser.

## Install

Needs Rust, `nautilus-devel` (Nautilus 43 or newer) and a running Nuvolo.

```sh
make
sudo make install
nautilus -q
```

## Documentation

Documentation: https://cyythanapong-netizen.github.io/nuvolo-nautilus/ (sources in [`docs/`](docs/), built with Sphinx).

## License

GPL-3.0-or-later
