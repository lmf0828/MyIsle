import io,re,sys
p=r'D:\LingDao\src\lyrics.rs'
s=io.open(p,encoding='utf-8').read()

# 1) scoring block
old_score = '''        let mut sc = 0i32;
        if name == ctitle {
            sc += 6;
        } else if !ctitle.is_empty() && (name.contains(&ctitle) || ctitle.contains(name)) {
            sc += 3;
        }
        if let Some(ars) = s.get("artists").and_then(|v| v.as_array()) {
            if ars.iter().any(|a| {
                a.get("name")
                    .and_then(|v| v.as_str())
                    .map(|n| !artist.is_empty() && (artist.contains(n) || n.contains(artist)))
                    .unwrap_or(false)
            }) {
                sc += 4;
            }
        }
        scored.push((sc, id, dur_s));'''
new_score = '''        // 归一化比较：只保留字母数字（含中文），忽略空格/标点/大小写；
        // 歌名必须“完全相同”才算命中（候选带 (Live)/(cover) 等后缀会被排除）。
        let norm = |x: &str| -> String {
            x.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect::<String>()
        };
        let nt = norm(&ctitle);
        let nm = norm(name);
        let na = norm(artist);
        let mut sc = 0i32;
        if !nt.is_empty() && nm == nt {
            sc += 10; // 歌名完全一致
        } else if !nt.is_empty() && (nm.contains(&nt) || nt.contains(&nm)) {
            sc += 4; // 仅包含（可能是 cover/remix，低权重）
        }
        if let Some(ars) = s.get("artists").and_then(|v| v.as_array()) {
            let artist_ok = ars.iter().any(|a| {
                a.get("name").and_then(|v| v.as_str()).map(|n| {
                    let nn = norm(n);
                    !na.is_empty() && !nn.is_empty() && (nn == na || nn.contains(&na) || na.contains(&nn))
                }).unwrap_or(false)
            });
            if artist_ok {
                sc += 6; // 歌手一致
            }
        }
        scored.push((sc, id, dur_s));'''
assert old_score in s, 'score block not found'
s=s.replace(old_score,new_score,1)

# 2) pool construction replaces idxs init
old_idxs='''    // 版本优先：时长与"当前播放器实际时长"最接近的候选排前面（同名现场/录音室版本歌词时间轴不同）
    let mut idxs: Vec<usize> = (0..cands.len()).collect();'''
new_idxs='''    // 精确度优先：先用"歌名完全一致 + 歌手一致"(>=16) 的候选；
    // 没有再用"歌名完全一致"(>=10)；再没有才退到"仅包含"(>=4)。绝不接受只靠歌手匹配的其它歌。
    let mut pool: Vec<usize> = (0..cands.len()).filter(|&i| cands[i].0 >= 16).collect();
    if pool.is_empty() {
        pool = (0..cands.len()).filter(|&i| cands[i].0 >= 10).collect();
    }
    if pool.is_empty() {
        pool = (0..cands.len()).filter(|&i| cands[i].0 >= 4).collect();
    }
    // 版本优先：同一池内，时长与"当前播放器实际时长"最接近的排前面
    let mut idxs: Vec<usize> = pool;'''
assert old_idxs in s, 'idxs block not found'
s=s.replace(old_idxs,new_idxs,1)

# tighten class threshold 12 -> 8
s=s.replace('let class_i = if expected_dur > 5.0 && ci > 12.0 { 1 } else { 0 };',
            'let class_i = if expected_dur > 5.0 && ci > 8.0 { 1 } else { 0 };')
s=s.replace('let class_j = if expected_dur > 5.0 && cj > 12.0 { 1 } else { 0 };',
            'let class_j = if expected_dur > 5.0 && cj > 8.0 { 1 } else { 0 };')

# 3) loops: remove sc threshold (pool already filtered)
s=s.replace('''    for &ci in idxs.iter().take(12) {
        let (sc, id, _cd) = cands[ci];
        if sc < 6 { continue; }''','''    for &ci in idxs.iter().take(12) {
        let (sc, id, _cd) = cands[ci];
        let _ = sc;''',1)
s=s.replace('''        for &ci in idxs.iter().take(12) {
            let (sc, id, _cd) = cands[ci];
            if sc < 6 { continue; }''','''        for &ci in idxs.iter().take(12) {
            let (sc, id, _cd) = cands[ci];
            let _ = sc;''',1)

io.open(p,'w',encoding='utf-8',newline='').write(s)
print('ok')
