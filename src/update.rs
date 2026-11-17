//! 应用内更新：检查 GitHub Release 的 latest.json、下载安装包、校验并启动安装器。
//!
//! 清单地址优先级：
//!   1. 环境变量 `LINGDAO_UPDATE_URL`（可为 http(s) 或本地文件路径，方便测试）
//!   2. 编译期 `LINGDAO_REPO`（CI 注入 `owner/repo`）→
//!      https://github.com/<owner>/<repo>/releases/latest/download/latest.json
//!
//! latest.json 结构：
//! { "version":"0.2.0", "notes":"...", "url":"https://.../LingDao-Setup-0.2.0.exe",
//!   "sha256":"...", "size":12345678, "mandatory":false, "pub_date":"2026-11-20" }

use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct Manifest {
    pub version: String,
    pub notes: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub mandatory: bool,
}

fn repo_slug() -> Option<String> {
    let r = std::env::var("LINGDAO_REPO")
        .ok()
        .or_else(|| option_env!("LINGDAO_REPO").map(|s| s.to_string()))?;
    let r = r.trim().to_string();
    if r.is_empty() || r.contains("YOUR_") {
        None
    } else {
        Some(r)
    }
}

/// 返回清单 URL；没有配置仓库时返回 None（开发版默认不联网检查）。
pub fn manifest_url() -> Option<String> {
    if let Ok(u) = std::env::var("LINGDAO_UPDATE_URL") {
        if !u.trim().is_empty() {
            return Some(u.trim().to_string());
        }
    }
    repo_slug().map(|r| format!("https://github.com/{}/releases/latest/download/latest.json", r))
}

fn read_text(url: &str) -> Result<String, String> {
    if let Some(p) = url.strip_prefix("file://") {
        return std::fs::read_to_string(p).map_err(|e| e.to_string());
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            .timeout_read(Duration::from_secs(6))
            .build();
        let resp = agent.get(url).call().map_err(|e| e.to_string())?;
        let mut s = String::new();
        resp.into_reader()
            .take(256 * 1024)
            .read_to_string(&mut s)
            .map_err(|e| e.to_string())?;
        return Ok(s);
    }
    std::fs::read_to_string(url).map_err(|e| e.to_string())
}

fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let text = text.trim_start_matches('\u{feff}');
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let version = v.get("version").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    if version.is_empty() {
        return Err("latest.json 缺少 version".into());
    }
    Ok(Manifest {
        version,
        notes: v.get("notes").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        url: v.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        sha256: v.get("sha256").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        size: v.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
        mandatory: v.get("mandatory").and_then(|x| x.as_bool()).unwrap_or(false),
    })
}

fn ver_key(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|x| x.parse::<u64>().unwrap_or(0))
        .collect()
}

/// a 是否比 b 新（按数字段比较；预发布后缀忽略）。
pub fn is_newer(a: &str, b: &str) -> bool {
    let (ka, kb) = (ver_key(a), ver_key(b));
    for i in 0..ka.len().max(kb.len()) {
        let x = *ka.get(i).unwrap_or(&0);
        let y = *kb.get(i).unwrap_or(&0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// 同步检查更新。返回 Ok(Some(m)) 表示有比当前更新的版本；
/// Ok(None) 表示已是最新（或未配置更新源）；Err 表示网络/格式错误。
pub fn check() -> Result<Option<Manifest>, String> {
    let url = match manifest_url() {
        Some(u) => u,
        None => return Ok(None),
    };
    let m = parse_manifest(&read_text(&url)?)?;
    if is_newer(&m.version, VERSION) {
        Ok(Some(m))
    } else {
        Ok(None)
    }
}

fn upd_dir() -> PathBuf {
    let d = crate::paths::data_dir().join("updates");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn sha256_file(path: &Path) -> Result<String, String> {
    // 用系统 certutil 计算 SHA256，避免额外依赖。
    let out = std::process::Command::new("certutil")
        .args(["-hashfile"])
        .arg(path)
        .arg("SHA256")
        .output()
        .map_err(|e| e.to_string())?;
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        let t: String = line.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if t.len() == 64 {
            return Ok(t.to_lowercase());
        }
    }
    Err("certutil 未返回 SHA256".into())
}

fn download(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(p) = url.strip_prefix("file://") {
        std::fs::copy(p, dest).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(60))
        .build();
    let resp = agent.get(url).call().map_err(|e| e.to_string())?;
    let mut r = resp.into_reader();
    let mut f = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    std::io::copy(&mut r, &mut f).map_err(|e| e.to_string())?;
    Ok(())
}

/// 下载安装包并校验；成功返回本地路径。`sha256` 为空时跳过校验（仅测试用）。
pub fn download_setup(m: &Manifest) -> Result<PathBuf, String> {
    if m.url.trim().is_empty() {
        return Err("清单缺少下载地址".into());
    }
    let name = m
        .url
        .rsplit('/')
        .next()
        .filter(|s| s.to_lowercase().ends_with(".exe"))
        .unwrap_or("LingDao-Setup.exe");
    let dest = upd_dir().join(format!("{}-{}", m.version, name));
    let tmp = dest.with_extension("part");
    download(&m.url, &tmp)?;
    if !m.sha256.trim().is_empty() {
        let got = sha256_file(&tmp)?;
        if got != m.sha256.trim().to_lowercase() {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("SHA256 不匹配（下载 {}）", &got[..8.min(got.len())]));
        }
    }
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

/// 启动安装器（Inno Setup 静默模式）。成功后本进程应尽快退出。
pub fn run_setup(path: &Path) -> Result<(), String> {
    std::process::Command::new(path)
        .args(["/SILENT", "/CLOSEAPPLICATIONS", "/RESTARTAPPLICATIONS", "/NORESTART"])
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.1.10", "0.1.9"));
    }

    #[test]
    fn parse_ok() {
        let m = parse_manifest(
            r#"{"version":"0.2.0","notes":"fix","url":"https://x/a.exe","sha256":"aa","size":5,"mandatory":true}"#,
        )
        .unwrap();
        assert_eq!(m.version, "0.2.0");
        assert!(m.mandatory);
        assert_eq!(m.size, 5);
    }

    #[test]
    fn parse_missing_version() {
        assert!(parse_manifest(r#"{"notes":"x"}"#).is_err());
    }
}
