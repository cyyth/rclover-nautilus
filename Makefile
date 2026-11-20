EXTDIR ?= $(shell pkg-config --variable=extensiondir libnautilus-extension-4)
ICONDIR ?= /usr/share/icons/hicolor/scalable/emblems

# Build as your user, install as root:  make && sudo make install
build:
	cargo build --release

test:
	cargo test

install:
	install -Dm755 target/release/libnuvolo_nautilus.so $(DESTDIR)$(EXTDIR)/libnuvolo-nautilus.so
	install -Dm644 -t $(DESTDIR)$(ICONDIR) icons/*.svg
	@echo "Restart GNOME Files with: nautilus -q"

uninstall:
	rm -f $(DESTDIR)$(EXTDIR)/libnuvolo-nautilus.so $(DESTDIR)$(ICONDIR)/emblem-nuvolo-*.svg

docs:
	sphinx-build -W -b html docs docs/_build/html

.PHONY: build test install uninstall docs
