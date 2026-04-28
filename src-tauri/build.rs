fn main() {
    tauri_build::build();
    add_libmpv_link_search_paths();
}

fn add_libmpv_link_search_paths() {
    #[cfg(target_os = "macos")]
    {
        use std::path::Path;

        println!("cargo:rerun-if-env-changed=LIBMPV_LIB_DIR");

        if let Ok(path) = std::env::var("LIBMPV_LIB_DIR") {
            if !path.trim().is_empty() {
                println!("cargo:rustc-link-search=native={path}");
            }
        }

        for candidate in [
            "/opt/homebrew/opt/mpv/lib",
            "/opt/homebrew/lib",
            "/usr/local/opt/mpv/lib",
            "/usr/local/lib",
        ] {
            if Path::new(candidate).exists() {
                println!("cargo:rustc-link-search=native={candidate}");
            }
        }
    }
}
