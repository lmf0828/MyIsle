# WinIsle Rust 构建环境（winlibs MSVCRT mingw-w64 + 共用 cargo/rustup on D:）
# 用法： . D:\LingDao\scripts\genv.ps1
$env:CARGO_HOME='D:\rust\cargo'
$env:RUSTUP_HOME='D:\rust\rustup'
$env:PATH='D:\rust\toolchains\winlibs-msvcrt\mingw64\bin;D:\rust\cargo\bin;'+$env:PATH
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER='D:\rust\toolchains\winlibs-msvcrt\mingw64\bin\gcc.exe'
$env:CC_X86_64_PC_WINDOWS_GNU='D:\rust\toolchains\winlibs-msvcrt\mingw64\bin\gcc.exe'
$env:AR_X86_64_PC_WINDOWS_GNU='D:\rust\toolchains\winlibs-msvcrt\mingw64\bin\ar.exe'
Remove-Item Env:HTTPS_PROXY,Env:HTTP_PROXY,Env:ALL_PROXY,Env:CARGO_HTTP_PROXY -ErrorAction SilentlyContinue
