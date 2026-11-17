# -*- coding: utf-8 -*-
"""One-shot: rename user-visible product name 灵岛 LingDao -> i岛 in source (UTF-8 safe)."""
import io, sys

def patch(path, repls):
    with io.open(path, "r", encoding="utf-8") as f:
        s = f.read()
    orig = s
    for old, new, want in repls:
        n = s.count(old)
        if n != want:
            print(f"  WARN {path}: expected {want} of {old!r}, found {n}")
        s = s.replace(old, new)
    if s != orig:
        with io.open(path, "w", encoding="utf-8", newline="") as f:
            f.write(s)
        print(f"  patched {path}")
    else:
        print(f"  unchanged {path}")

print("patching main.rs ...")
patch(r"D:\LingDao\src\main.rs", [
    ('.with_title("灵岛")', '.with_title("i岛·设置")', 1),
    ('platform::find_process_window("灵岛")', 'platform::find_process_window("i岛·设置")', 1),
    ('"关闭灵岛"', '"关闭 i岛"', 1),
    ('"灵岛"', '"i岛"', 2),                       # two painter texts (header + big title)
    ('"灵岛 LingDao"', '"i岛"', 2),               # viewport title + run_native app name
    ('# 灵岛 LingDao 配置', '# i岛 配置', 1),      # config.ini header
])

print("patching tray.rs ...")
patch(r"D:\LingDao\src\tray.rs", [
    ('.with_tooltip("灵岛 LingDao")', '.with_tooltip("i岛")', 1),
])
print("done")
