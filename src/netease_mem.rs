//! 从网易云音乐 PC 客户端本地数据里解析「当前播放歌曲的 song id」。
//!
//! 目的：不再靠「歌名搜索」去网易云找歌词（会被同名歌曲/翻唱/伴奏错配，e.g. 把
//! Laleh《roses》贴到 KAVO《Keep the Roses》上）。网易云客户端会把当前播放队列
//! 持久化到 `%LOCALAPPDATA%\Netease\CloudMusic\webdata\file\playingList`（UTF-8 JSON，
//! 每项含 `track.id / name / artists[].name / duration(ms)`）。用 SMTC 拿到的
//! 歌名+歌手+时长去这份队列里精确命中，即可拿到**确切的 song id**，再按 id 调
//! `/api/song/lyric?id=...` 取歌词——同一 id，绝不会串歌。

use serde_json::Value;
use std::path::PathBuf;

fn key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn playing_list_path() -> Option<PathBuf> {
    let base = std::env::var("LOCALAPPDATA").ok()?;
    let p = PathBuf::from(base)
        .join("Netease")
        .join("CloudMusic")
        .join("webdata")
        .join("file")
        .join("playingList");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

fn queue_path() -> Option<PathBuf> {
    let base = std::env::var("LOCALAPPDATA").ok()?;
    let p = PathBuf::from(base)
        .join("Netease")
        .join("CloudMusic")
        .join("webdata")
        .join("file")
        .join("queue");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// 在一份「track 数组」JSON 里找与 (title, artist, expected_dur) 精确匹配的 song id。
fn scan_array(v: &Value, tk: &str, ak: &str, expected_dur: f64) -> Option<i64> {
    let arr = v.as_array()?;
    let mut loose: Option<i64> = None;
    for item in arr {
        let track = item.get("track").unwrap_or(item);
        let name = track.get("name").and_then(|x| x.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let dur = track
            .get("duration")
            .or_else(|| track.get("dt"))
            .and_then(|x| x.as_i64())
            .map(|ms| ms as f64 / 1000.0)
            .unwrap_or(0.0);
        let artists: String = track
            .get("artists")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        let id = track
            .get("id")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .or_else(|| track.get("id").and_then(|x| x.as_i64()));
        let Some(id) = id else { continue };

        let name_ok = key(name) == tk;
        if !name_ok {
            continue;
        }
        let artist_ok = ak.is_empty()
            || key(&artists).contains(ak)
            || ak.contains(&key(&artists));
        // 时长一致（SMTC 时长与歌曲时长差 <= 5s）才算强匹配。
        let dur_ok = expected_dur < 5.0 || (dur - expected_dur).abs() <= 5.0;
        if artist_ok && dur_ok {
            return Some(id);
        }
        // 歌名精确、但时长/歌手对不上：先记下，作为弱兜底。
        if loose.is_none() {
            loose = Some(id);
        }
    }
    loose
}

/// 返回当前网易云在播歌曲的 song id（从客户端本地队列文件里解析）。
pub fn current_song_id(title: &str, artist: &str, expected_dur: f64) -> Option<i64> {
    if title.trim().is_empty() {
        return None;
    }
    let tk = key(title);
    if tk.is_empty() {
        return None;
    }
    let ak = key(artist);

    for p in [playing_list_path(), queue_path()].into_iter().flatten() {
        let Ok(txt) = std::fs::read_to_string(&p) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&txt) else { continue };
        // playingList 形如 {"list":[...]}；queue 形如 [...]
        let arr = v.get("list").unwrap_or(&v);
        if let Some(id) = scan_array(arr, &tk, &ak, expected_dur) {
            return Some(id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore]
    fn probe_netease_env() {
        let title = std::env::var("NE_TITLE").unwrap_or_default();
        let artist = std::env::var("NE_ARTIST").unwrap_or_default();
        let dur: f64 = std::env::var("NE_DUR")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let id = super::current_song_id(&title, &artist, dur);
        eprintln!("netease_mem id for {:?} - {:?} dur={} => {:?}", title, artist, dur, id);
    }
}
