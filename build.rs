fn main() {
    pkg_config::probe_library("libnautilus-extension-4").expect("libnautilus-extension-4 development files (nautilus-devel)");
}
