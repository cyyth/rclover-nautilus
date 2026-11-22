EXTDIR ?= $(shell pkg-config --variable=extensiondir libnautilus-extension-4)
ICONDIR ?= /usr/share/icons/hicolor/scalable/emblems

# Build as your user, install as root:  make && sudo make install
build:
	cargo build --release

test:
	cargo test

install:
	rm -f $(DESTDIR)$(EXTDIR)/libnuvolo-nautilus.so $(DESTDIR)$(ICONDIR)/emblem-nuvolo-*.svg  # the old name
	install -Dm755 target/release/librclover_nautilus.so $(DESTDIR)$(EXTDIR)/librclover-nautilus.so
	install -Dm644 -t $(DESTDIR)$(ICONDIR) icons/*.svg
	@echo "Restart GNOME Files with: nautilus -q"

uninstall:
	rm -f $(DESTDIR)$(EXTDIR)/librclover-nautilus.so $(DESTDIR)$(ICONDIR)/emblem-rclover-*.svg

docs:
	sphinx-build -W -b html docs docs/_build/html

.PHONY: build test install uninstall docs
