//! Linux linker shim for machines that have the runtime X11/XKB shared libraries
//! (`libxcb.so.1`, `libxkbcommon.so.0`, `libxkbcommon-x11.so.0`) installed without
//! the unversioned `-dev` symlinks (`libxcb.so`, `libxkbcommon.so`, `libxkbcommon-x11.so`).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(target_os = "linux")]
    link_linux_system_libs();
}

#[cfg(target_os = "linux")]
fn link_linux_system_libs() {
    use std::path::{Path, PathBuf};

    let Some(out_dir) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    let search_dirs = [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib64",
        "/usr/lib",
        "/lib/x86_64-linux-gnu",
        "/lib/aarch64-linux-gnu",
    ];
    let needed = [
        ("libxcb.so", "libxcb.so.1"),
        ("libxkbcommon.so", "libxkbcommon.so.0"),
        ("libxkbcommon-x11.so", "libxkbcommon-x11.so.0"),
    ];

    let mut created_any = false;
    for (unversioned, soname) in needed {
        let already_present = search_dirs
            .iter()
            .any(|dir| Path::new(dir).join(unversioned).exists());
        if already_present {
            continue;
        }
        if let Some(target) = search_dirs
            .iter()
            .map(|dir| Path::new(dir).join(soname))
            .find(|candidate| candidate.exists())
        {
            let link_path = out_dir.join(unversioned);
            let _ = std::fs::remove_file(&link_path);
            if std::os::unix::fs::symlink(&target, &link_path).is_ok() {
                created_any = true;
            }
        }
    }

    if created_any {
        println!("cargo:rustc-link-search=native={}", out_dir.display());
    }
}
