/// Build script for bwp-controller.
///
/// On Windows, copies `WinDivert.dll` from the windivert-sys build output
/// directory to the final Cargo output directory so that `bwp-engine.exe`
/// can find it at runtime without any manual copying step.
fn main() {
    #[cfg(target_os = "windows")]
    copy_windivert_dll();
}

#[cfg(target_os = "windows")]
fn copy_windivert_dll() {
    use std::path::PathBuf;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    // OUT_DIR is: <workspace>/target/<profile>/build/<crate-hash>/out
    // Profile dir is 3 levels up from out_dir:
    //   out -> <hash> -> build -> <profile>
    let profile_dir = match out_dir.ancestors().nth(3) {
        Some(d) => d.to_path_buf(),
        None => return,
    };

    // Look for WinDivert.dll in any build subdir
    let build_dir = profile_dir.join("build");
    match std::fs::read_dir(&build_dir) {
        Ok(entries) => {
            for entry in entries.filter_map(|e| e.ok()) {
                let candidate = entry.path().join("out").join("WinDivert.dll");
                if candidate.exists() {
                    let dst = profile_dir.join("WinDivert.dll");
                    if !dst.exists() {
                        if let Err(e) = std::fs::copy(&candidate, &dst) {
                            println!("cargo:warning=WinDivert.dll copy failed: {}", e);
                        } else {
                            println!("cargo:warning=WinDivert.dll → {}", dst.display());
                        }
                    }
                    // Found it — also emit link search path
                    println!(
                        "cargo:rustc-link-search=native={}",
                        entry.path().join("out").display()
                    );
                    return;
                }
            }
            println!(
                "cargo:warning=WinDivert.dll not found in {}",
                build_dir.display()
            );
        }
        Err(e) => println!("cargo:warning=Cannot scan build dir: {}", e),
    }
}
