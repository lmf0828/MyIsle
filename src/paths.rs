//! 统一的数据目录。
//!
//! 发布版默认使用 `%APPDATA%\LingDao\data`（可写、卸载不误删、更新不覆盖）；
//! 开发/绿色版若在 exe 同目录已有 `data\`，继续用它（保持现有数据不搬家），
//! 也可用环境变量 `LINGDAO_DATA_DIR` 明确指定，或用 `LINGDAO_PORTABLE=1`
//! 强制使用 exe 同目录的 `data\`。
use std::path::PathBuf;

fn appdata_data() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("LingDao").join("data"))
}

pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("LINGDAO_DATA_DIR") {
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|x| x.to_path_buf()));
    let portable = std::env::var_os("LINGDAO_PORTABLE").is_some();
    if let Some(dir) = &exe_dir {
        let local = dir.join("data");
        // 开发/绿色版：exe 旁边已有 data 就沿用；PORTABLE=1 时强制用本地 data。
        if portable || local.is_dir() {
            return local;
        }
    }
    if let Some(d) = appdata_data() {
        return d;
    }
    if let Some(dir) = exe_dir {
        return dir.join("data");
    }
    PathBuf::from("data")
}

pub fn ensure() -> PathBuf {
    let d = data_dir();
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn file(name: &str) -> PathBuf {
    ensure().join(name)
}
