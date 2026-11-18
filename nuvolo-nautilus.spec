%global debug_package %{nil}

Name:           nuvolo-nautilus
Version:        0.1.0
Release:        1%{?dist}
Summary:        GNOME Files extension for Nuvolo
License:        GPL-3.0-or-later
URL:            https://github.com/cyythanapong-netizen/nuvolo-nautilus
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  make
BuildRequires:  pkgconfig(libnautilus-extension-4)
Requires:       nautilus
Requires:       nuvolo

%description
Shows Nuvolo sync emblems on files in cloud drives and adds a Nuvolo menu to the
GNOME Files context menu: keep on device, free local cache, sync now, copy a share
link and open in the browser.

%prep
%autosetup -n %{name}-%{version}

%build
cargo build --release --locked

%install
make install DESTDIR=%{buildroot} EXTDIR=%{_libdir}/nautilus/extensions-4 ICONDIR=%{_datadir}/icons/hicolor/scalable/emblems

%files
%license LICENSE
%doc README.md
%{_libdir}/nautilus/extensions-4/libnuvolo-nautilus.so
%{_datadir}/icons/hicolor/scalable/emblems/emblem-nuvolo-*.svg

%changelog
* Wed Nov 18 2026 Thanapong Chuangyanyong - 0.1.0-1
- First package
