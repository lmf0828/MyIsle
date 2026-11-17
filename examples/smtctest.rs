// 诊断 v2：后台线程 STA + 消息泵 + 轮询 Status。
use windows::Foundation::{AsyncStatus, IAsyncInfo};
use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(pv: *const core::ffi::c_void, coinit: u32) -> i32;
}

#[repr(C)]
struct MSG { hwnd: isize, message: u32, wparam: usize, lparam: isize, time: u32, pt_x: i32, pt_y: i32 }
#[link(name = "user32")]
extern "system" {
    fn PeekMessageW(msg: *mut MSG, hwnd: isize, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const MSG) -> i32;
    fn DispatchMessageW(msg: *const MSG) -> isize;
}
fn pump() {
    unsafe {
        let mut m: MSG = std::mem::zeroed();
        while PeekMessageW(&mut m, 0, 0, 0, 1) != 0 {
            let _ = TranslateMessage(&m);
            let _ = DispatchMessageW(&m);
        }
    }
}

fn main() {
    unsafe {
        let hr = CoInitializeEx(std::ptr::null(), 0x2); // STA
        println!("CoInitializeEx STA hr={hr:#x}");
        let op = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync() {
            Ok(o) => o,
            Err(e) => { println!("RequestAsync ERR {}", e.message()); return; }
        };
        println!("RequestAsync ok; polling Status with pump...");
        let start = std::time::Instant::now();
        loop {
            pump();
            let st = op.Status().unwrap_or(AsyncStatus::Started);
            if st != AsyncStatus::Started {
                println!("Status={:?} after {:?}", st, start.elapsed());
                if st == AsyncStatus::Completed {
                    match op.GetResults() {
                        Ok(m) => {
                            println!("manager OK");
                            match m.GetCurrentSession() {
                                Ok(s) => {
                                    let app = s.SourceAppUserModelId().map(|x| x.to_string()).unwrap_or_default();
                                    let title = match s.TryGetMediaPropertiesAsync() {
                                        Ok(p) => { let mut t = "propsPending".to_string();
                                            for _ in 0..50 { pump(); if p.Status().unwrap_or(AsyncStatus::Started)==AsyncStatus::Completed { t = p.GetResults().ok().and_then(|pr| pr.Title().ok()).map(|x| x.to_string()).unwrap_or_default(); break; } std::thread::sleep(std::time::Duration::from_millis(100)); }
                                            t }
                                        Err(e) => format!("tryPropsErr {}", e.message()),
                                    };
                                    println!("SESSION app='{}' title='{}'", app, title);
                                }
                                Err(e) => println!("no current session {}", e.message()),
                            }
                        }
                        Err(e) => println!("GetResults ERR {}", e.message()),
                    }
                }
                break;
            }
            if start.elapsed().as_secs() > 10 {
                println!("TIMEOUT still Started after 10s");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    }
}
