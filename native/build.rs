fn main() {
    // Luau and mpv's Lua are different C ABIs with overlapping symbol names.
    // Keep vendored static-library symbols local; export only our Rust bridge.
    println!("cargo:rustc-link-arg=-Wl,--exclude-libs,ALL");
}
