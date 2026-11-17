//! Embed the 'MyIsle' icon + version info into the exe (RT_GROUP_ICON / VERSIONINFO).
//! Uses the mingw-w64 `windres` that ships with the toolchain configured in genv.ps1,
//! so no Windows SDK / rc.exe is required.
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn find_windres() -> Option<PathBuf> {
    // 1) derive from the configured GNU linker driver (genv.ps1 sets gcc.exe path)
    if let Ok(gcc) = env::var("CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER") {
        let cand = PathBuf::from(gcc).with_file_name("windres.exe");
        if cand.exists() {
            return Some(cand);
        }
    }
    // 2) well-known bundled toolchain location
    let p = PathBuf::from(r"D:\rust\toolchains\winlibs-msvcrt\mingw64\bin\windres.exe");
    if p.exists() {
        return Some(p);
    }
    None
}

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/myisle.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Some(windres) = find_windres() else {
        println!("cargo:warning=windres not found, skipping icon resource");
        return;
    };
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("app.res.o");
    let status = Command::new(&windres)
        .arg("-I").arg(".")
        .arg("-O").arg("coff")
        .arg("-o").arg(&out)
        .arg("app.rc")
        .status();
    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg={}", out.display());
        }
        _ => println!("cargo:warning=windres failed to compile app.rc"),
    }
}
