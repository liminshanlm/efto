//! MS-SHLLINK（.lnk）二进制解析，只取启动器需要的字段。
//!
//! 策略：结构化解析优先（IDList 跳过 → LinkInfo 本地路径 → StringData 顺序读取），
//! 解析不出来时退回"在原始字节里找 UTF-16LE 绝对路径"的启发式，保证不丢条目。
//! 不依赖 COM，因此不需要 STA/COM 初始化，索引可以并行跑。

const HEADER_SIZE: usize = 76;
const LINK_CLSID: [u8; 16] = [
    0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

const F_HAS_IDLIST: u32 = 0x0000_0001;
const F_IS_UNICODE: u32 = 0x0000_0008;
// 解析器不依赖任何"可选块存在"标志位：StringData 各位在 MS-SHLLINK 与 IShellLink
// 头文件里编号不一致，LinkInfo 的 0x10 位在真机样本里还普遍置位。
// 改用块头特征 + 内容校验自证（见 parse 中的尽力读策略），错判只会退化成启发式。

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellLink {
    /// 目标绝对路径（LinkInfo 或 DOTS/启发式）
    pub target: Option<String>,
    /// 快捷方式显示名（StringData Name）
    pub name: Option<String>,
    pub working_dir: Option<String>,
    pub arguments: Option<String>,
    /// IconLocation 原值，可能是 `C:\a\b.exe,-101`
    pub icon_location: Option<String>,
    /// 结构化解析没拿到绝对路径（靠 VolumeID 补全或启发式兜底）
    pub degraded: bool,
    /// 诊断用：target 来源，方便统计真实还原率到底是哪条路径在起作用
    pub origin: TargetOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetOrigin {
    /// 还没尝试解析
    #[default]
    None,
    /// LinkInfo 的 LocalBasePath 本身就是绝对路径
    LinkInfo,
    /// LinkInfo 给的是卷根相对路径，用 VolumeID 的盘符补全
    LinkInfoWithVolume,
    /// 在原始字节里扫路径的兜底结果
    Heuristic,
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    fn u16(&mut self) -> Option<u16> {
        let v = u16::from_le_bytes(self.take(2)?.try_into().unwrap());
        Some(v)
    }
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_le_bytes(self.take(4)?.try_into().unwrap());
        Some(v)
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.p.checked_add(n)?;
        if end > self.b.len() {
            return None;
        }
        let s = &self.b[self.p..end];
        self.p = end;
        Some(s)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        let end = self.p.checked_add(n)?;
        if end > self.b.len() {
            return None;
        }
        self.p = end;
        Some(())
    }
}

fn utf16_to_string(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks(2)
        .filter_map(|c| c.try_into().ok().map(u16::from_le_bytes))
        .collect();
    // 截断到第一个 NUL
    let end = units.iter().position(|u| *u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn ansi_to_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// 判断一个 StringData 块是否像"给人看的快捷方式名"。
/// 规则刻意保守：宁可退回文件名（Windows 在无 Name 时也就是显示文件名）。
fn plausible_display_name(s: &str) -> bool {
    let t = s.trim();
    if t.chars().count() < 2 || t.chars().count() > 64 {
        return false;
    }
    if looks_like_path(t) {
        return false;
    }
    if t.contains('\\') || t.contains('/') || t.contains(':') {
        return false;
    }
    if t.starts_with('.') || t.starts_with('-') || t.starts_with('_') {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    for bad in [".exe", ".lnk", ".url", ".dll", ".msi", ".bat"] {
        if lower.ends_with(bad) {
            return false;
        }
    }
    // 至少含一个字母（汉字也算）：过滤 "@"、".."、纯标点
    t.chars().any(|c| c.is_alphabetic())
}

/// 粗略判断是否像一个可用的绝对路径。
fn looks_like_path(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 3 {
        return false;
    }
    let head: Vec<char> = t.chars().take(2).collect();
    let drive = head.len() == 2 && head[0].is_ascii_alphabetic() && head[1] == ':';
    let unc = t.starts_with("\\\\");
    let env = t.starts_with('%');
    let no_nul = !t.contains('\0');
    (drive || unc || env) && no_nul && t.chars().all(|c| !c.is_control())
}

/// 展开 `%NAME%` 环境变量。
pub fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        if let Some(end) = rest[start + 1..].find('%') {
            let name = &rest[start + 1..start + 1 + end];
            if name.is_empty() {
                out.push('%');
                rest = &rest[start + 2..];
                continue;
            }
            let val = std::env::var(name).unwrap_or_else(|_| format!("%{name}%"));
            out.push_str(&val);
            rest = &rest[start + 2 + end..];
        } else {
            out.push('%');
            rest = &rest[start + 1..];
        }
    }
    out.push_str(rest);
    out
}

pub fn parse(bytes: &[u8]) -> Option<ShellLink> {
    if bytes.len() < HEADER_SIZE + 2 {
        return None;
    }
    let mut r = Reader::new(bytes);
    let hdr = r.u32()?;
    if hdr != HEADER_SIZE as u32 {
        return None;
    }
    let clsid = r.take(16)?;
    if clsid != LINK_CLSID {
        return None;
    }
    let flags = r.u32()?;
    // 头部剩余字段：FileAttributes(4) + 3×FILETIME(24) + FileSize(4) + IconIndex(4)
    // + ShowCommand(4) + HotKey(2) + Reserved1(6) + Reserved2(8) = 52，读完正好到 76
    r.skip(52)?;

    let unicode = flags & F_IS_UNICODE != 0;

    // ---- LinkTargetIDList ----
    if flags & F_HAS_IDLIST != 0 {
        // 实测（204 个真实快捷方式全样本）：这里的 u16 是**其后 IDList 数据**的长度，
        // 不含自身 2 字节。按"含自身"少跳 2 字节会让 LinkInfo 定位整体偏移，
        // 导致结构化解析 100% 失败、只能靠启发式兜底。
        let size = r.u16()? as usize;
        if size < 2 || r.skip(size).is_none() {
            return None;
        }
    }

    let mut link = ShellLink::default();

    // ---- LinkInfo ----
    // 不用 flags 判断是否存在：真机样本里 0x10 位普遍置位（曾被当作 ForceNoLinkInfo），
    // 而块头特征本身已足够自证（BlockSize 合理 + HeaderSize 在已知集合内 + 能读出盘符路径），
    // 读不出就原样跳过，交给后面的启发式。
    if let Some(li) = try_read_link_info(bytes, r.p) {
        r.p += li.consumed;
        if let Some((target, origin)) = compose_target(&li) {
            link.target = Some(target);
            link.origin = origin;
        }
    }

    // ---- StringData：Name / RelativePath / WorkingDir / Arguments / IconLocation ----
    // 哪些块存在由若干 flag 位决定，而这些位在 MS-SHLLINK 与 IShellLink 之间编号并不一致，
    // 因此按固定顺序"尽力读"：任一块读不出合法文本就停止，不猜标志位。
    // 参数与工作目录读不到也不影响启动——我们把 .lnk 本身交给 ShellExecute，Windows 负责解析全部字段。
    let mut read_str = || -> Option<String> {
        let pos = r.p;
        let len = r.u16()? as usize;
        if len == 0 || len > 1024 {
            r.p = pos;
            return None;
        }
        let char_bytes = if unicode { 2 } else { 1 };
        let raw = r.take(len * char_bytes)?;
        let s = if unicode {
            utf16_to_string(raw)
        } else {
            ansi_to_string(raw)
        };
        if s.chars().any(|c| c.is_control() && c != '\t') {
            r.p = pos;
            return None;
        }
        Some(s)
    };

    let mut blocks: Vec<String> = Vec::with_capacity(5);
    while blocks.len() < 5 {
        match read_str() {
            Some(s) => blocks.push(s),
            None => break,
        }
    }
    // 只有一个块时无法区分它是 Name 还是 RelativePath：因此必须做合理性校验。
    // 实测本机 204 个快捷方式，"尽力读"会把 RelativePath/DOTS 字符串误当成 Name，
    // 产出 "@"、"E"、"..\..\..\YunShuLauncher\YunShuLauncher.exe" 这类标题——
    // 这些一律退回文件名（startmenu 用 file_stem 兜底），否则搜索结果的显示名就是垃圾。
    let name_candidate = blocks.first().cloned().unwrap_or_default();
    if plausible_display_name(&name_candidate) {
        link.name = Some(name_candidate);
    }
    if blocks.len() >= 3 && !blocks[2].trim().is_empty() {
        link.working_dir = Some(blocks[2].clone());
    }
    if blocks.len() >= 4 && !blocks[3].trim().is_empty() {
        link.arguments = Some(blocks[3].clone());
    }
    if blocks.len() >= 5 && !blocks[4].trim().is_empty() {
        link.icon_location = Some(blocks[4].clone());
    }

    // ---- 兜底启发式：从原始字节里扫所有候选绝对路径并排序 ----
    if link.target.is_none() {
        if let Some((p, exists)) = heuristic_target(bytes) {
            link.target = Some(p);
            link.origin = TargetOrigin::Heuristic;
            link.degraded = !exists;
        }
    }

    for slot in [
        &mut link.target,
        &mut link.working_dir,
        &mut link.icon_location,
    ] {
        if let Some(v) = slot {
            *v = expand_env(v);
        }
    }

    Some(link)
}

struct LinkInfoResult {
    /// LocalBasePath（可能是绝对路径，也可能是卷根相对路径 `\Users\x\a.exe`）
    local_path: String,
    /// CommonPathSuffix
    suffix: String,
    /// VolumeID 里的盘符根，例如 `C:` 或 `C:\`
    drive: Option<String>,
    consumed: usize,
}

/// LocalBasePath / CommonPathSuffix / VolumeID 三者组合出最终绝对路径。
fn compose_target(li: &LinkInfoResult) -> Option<(String, TargetOrigin)> {
    let local = li.local_path.trim();
    if local.is_empty() {
        return None;
    }

    // 1. LocalBasePath 本身就是绝对路径（最常见）
    if has_drive_prefix(local) {
        return Some((join_local_and_suffix(local, &li.suffix), TargetOrigin::LinkInfo));
    }

    // 2. 卷根相对路径 + VolumeID 盘符：`\Users\me\AppData\...\app.exe` + `C:`
    if local.starts_with('\\') {
        if let Some(drive) = &li.drive {
            let root = drive.trim_end_matches('\\');
            let joined = format!("{root}{local}");
            if has_drive_prefix(&joined) {
                return Some((joined, TargetOrigin::LinkInfoWithVolume));
            }
        }
        // 3. 只有 CommonPathSuffix 是绝对路径的情况
        if has_drive_prefix(&li.suffix) {
            return Some((li.suffix.clone(), TargetOrigin::LinkInfoWithVolume));
        }
    }
    None
}

fn has_drive_prefix(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic()) && chars.next() == Some(':')
}

/// VolumeID 的 DrivePath 偏移。
/// 布局（相对 VolumeID 起点）：VolumeStamp(16) Flags(4) 保留(12)
/// DrivePathLength(4)@32 DrivePathOffset(4)@36 PathBufferLength(4)@40 PathBufferOffset(4)@44
const DRIVE_PATH_LENGTH_OFF: usize = 32;
const DRIVE_PATH_OFFSET_OFF: usize = 36;

fn read_volume_drive(bytes: &[u8], volume_abs: usize) -> Option<String> {
    let len = r_u32_at(bytes, volume_abs.checked_add(DRIVE_PATH_LENGTH_OFF)?)? as usize;
    let off = r_u32_at(bytes, volume_abs.checked_add(DRIVE_PATH_OFFSET_OFF)?)? as usize;
    if !(2..=8).contains(&len) {
        return None;
    }
    let p = volume_abs.checked_add(off)?;
    if p >= bytes.len() {
        return None;
    }
    let drive = read_sz(bytes, p, false);
    // 形如 `C:` / `C:\` / `\\server\share`
    if has_drive_prefix(&drive) {
        return Some(drive);
    }
    if drive.starts_with("\\\\") {
        return Some(drive);
    }
    None
}

fn r_u32_at(bytes: &[u8], p: usize) -> Option<u32> {
    let end = p.checked_add(4)?;
    if end > bytes.len() {
        return None;
    }
    Some(u32::from_le_bytes(bytes[p..end].try_into().unwrap()))
}

fn try_read_link_info(bytes: &[u8], at: usize) -> Option<LinkInfoResult> {
    let mut r = Reader::new(bytes.get(at..)?);
    let block_size = r.u32()?;
    let header_size = r.u32()?;
    if !(28..=4096).contains(&block_size) {
        return None;
    }
    if !matches!(header_size, 0x1C | 0x20 | 0x24 | 0x28 | 0x2C) {
        return None;
    }
    // LinkInfo: flags(0x08) volumeOff(0x0C) localOff(0x10) commonOff(0x14)
    //           然后 header_size >= 0x24 时是 localOffUnicode(0x18) commonOffUnicode(0x1C)
    let li_flags = r.u32()?;
    let volume_off = r.u32()? as usize;
    let local_off = r.u32()? as usize;
    let common_off = r.u32()? as usize;
    let has_unicode = header_size >= 0x24;
    let local_uni_off = if has_unicode {
        r.u32().map(|v| v as usize)
    } else {
        None
    };
    let common_uni_off = if has_unicode {
        r.u32().map(|v| v as usize)
    } else {
        None
    };

    if li_flags & 0x1 == 0 {
        // 没有 VolumeIDAndLocalPath，只有公共路径后缀，无法还原绝对路径
        return None;
    }
    let abs = |off: usize| at.checked_add(off).filter(|p| *p < bytes.len());

    let mut local = abs(local_off).map(|p| read_sz(bytes, p, false))?;
    if let Some(uo) = local_uni_off {
        if let Some(p) = abs(uo) {
            let uni = read_sz(bytes, p, true);
            if !uni.is_empty() {
                local = uni;
            }
        }
    }
    if local.is_empty() || local.len() > 1024 {
        return None;
    }

    let mut suffix = String::new();
    if li_flags & 0x2 != 0 {
        if let Some(off) = common_uni_off.or(Some(common_off)).and_then(abs) {
            suffix = read_sz(bytes, off, common_uni_off.is_some());
        }
    }

    // VolumeID 里的盘符：LocalBasePath 是相对路径时靠它补全
    let drive = abs(volume_off).and_then(|p| read_volume_drive(bytes, p));

    Some(LinkInfoResult {
        local_path: local,
        suffix,
        drive,
        consumed: block_size as usize,
    })
}

/// 在 offset 处读一个以 NUL 结尾的 ANSI / UTF-16LE 字符串。
fn read_sz(bytes: &[u8], offset: usize, unicode: bool) -> String {
    if offset >= bytes.len() {
        return String::new();
    }
    if unicode {
        let mut units = Vec::new();
        let mut i = offset;
        while i + 1 < bytes.len() {
            let u = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
            if u == 0 {
                break;
            }
            units.push(u);
            i += 2;
            if units.len() > 1024 {
                break;
            }
        }
        String::from_utf16_lossy(&units)
    } else {
        let end = bytes[offset..]
            .iter()
            .position(|b| *b == 0)
            .map(|n| offset + n)
            .unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[offset..end]).into_owned()
    }
}

fn join_local_and_suffix(local: &str, suffix: &str) -> String {
    if suffix.is_empty() || suffix.starts_with('\\') || suffix.contains(':') {
        return local.trim_end_matches('\\').to_string();
    }
    let sep = if local.ends_with('\\') { "" } else { "\\" };
    format!("{local}{sep}{suffix}")
}

/// 兜底：扫描字节里所有像绝对路径的字符串（UTF-16LE 与 ANSI），按可信度排序取最佳。
/// 返回 (路径, 是否存在于磁盘)。
///
/// 为什么要排序：IDList 里常混着 `.lnk` 自身所在目录、已知文件夹目录（如
/// `C:\Users\me\AppData\Roaming\Microsoft\Windows\Start Menu\Programs`），
/// 而真正的目标往往在 DOTS 块或 LinkInfo 之后。取"第一个像路径的串"会经常拿到目录。
fn heuristic_target(bytes: &[u8]) -> Option<(String, bool)> {
    let mut candidates: Vec<(u32, String)> = Vec::new();

    // UTF-16LE：'C' 0 ':' 0 '\' 0
    let mut i = 0;
    while i + 6 <= bytes.len() {
        if bytes[i].is_ascii_alphabetic()
            && bytes[i + 1] == 0
            && bytes[i + 2] == b':'
            && bytes[i + 3] == 0
            && bytes[i + 4] == b'\\'
            && bytes[i + 5] == 0
        {
            if let Some(s) = read_utf16_path(&bytes[i..]) {
                push_candidate(&mut candidates, s);
            }
        }
        i += 2;
    }
    // ANSI
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i].is_ascii_alphabetic() && bytes[i + 1] == b':' && bytes[i + 2] == b'\\' {
            if let Some(s) = read_ansi_path(&bytes[i..]) {
                push_candidate(&mut candidates, s);
            }
        }
        i += 1;
    }

    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let best = candidates.first().map(|(score, path)| {
        let exists = std::path::Path::new(path).exists();
        // 存在性已经进了打分，这里再兜一次，保证"存在"优先于"看着像"
        (path.clone(), exists || *score >= 200)
    })?;
    tracing::trace!(
        "lnk 启发式候选 {} 个，选中 {}（分 {}）",
        candidates.len(),
        best.0,
        candidates[0].0
    );
    Some(best)
}

fn push_candidate(list: &mut Vec<(u32, String)>, s: String) {
    let Some(score) = score_path_candidate(&s) else {
        return;
    };
    // 同一候选可能同时被 UTF-16 与 ANSI 扫到，去重保留高分
    if let Some(existing) = list.iter_mut().find(|(_, p)| p.eq_ignore_ascii_case(&s)) {
        if existing.0 < score {
            existing.0 = score;
        }
        return;
    }
    list.push((score, s));
}

fn score_path_candidate(raw: &str) -> Option<u32> {
    let t = raw.trim().trim_end_matches('"').to_string();
    if t.len() < 4 || !looks_like_path(&t) {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower.ends_with(".lnk") || lower.ends_with(".url") {
        // 快捷方式指向快捷方式极少见，且通常是 IDList 里的自身位置
        return None;
    }
    let mut score: u32 = 10;
    const EXEC: &[&str] = &[
        ".exe", ".msi", ".bat", ".cmd", ".com", ".scr", ".cpl", ".msc", ".vbs", ".ps1",
    ];
    if EXEC.iter().any(|e| lower.ends_with(e)) {
        score += 200;
    } else if lower.ends_with(".html") || lower.ends_with(".pdf") || lower.ends_with(".txt") {
        score += 60;
    } else if !lower.contains('.') {
        // 没有扩展名的大概率是目录
        score = score.saturating_sub(40);
    }
    if std::path::Path::new(&t).exists() {
        score += 120;
    }
    for marker in [
        "\\program files",
        "\\program files (x86)",
        "\\appdata\\local\\programs",
        "\\windows\\system32",
        "\\appdata\\roaming\\microsoft\\windows\\start menu",
    ] {
        if lower.contains(marker) {
            score += 30;
            break;
        }
    }
    // 自身所在目录（Start Menu 里的 .lnk 位置）即使存在也不该当成 target
    if lower.contains("\\start menu\\programs") && !EXEC.iter().any(|e| lower.ends_with(e)) {
        score = score.saturating_sub(150);
    }
    score += (t.len() as u32).min(40);
    Some(score)
}

fn read_utf16_path(bytes: &[u8]) -> Option<String> {
    let mut units = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let u = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        if u == 0 {
            break;
        }
        // 路径里不会出现控制符
        if u < 0x20 {
            return None;
        }
        units.push(u);
        i += 2;
        if units.len() > 512 {
            break;
        }
    }
    if units.len() < 4 {
        return None;
    }
    Some(String::from_utf16_lossy(&units))
}

fn read_ansi_path(bytes: &[u8]) -> Option<String> {
    let end = bytes
        .iter()
        .position(|b| *b == 0 || *b < 0x20)
        .unwrap_or(bytes.len());
    if end < 4 {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(flags: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&76u32.to_le_bytes());
        v.extend_from_slice(&LINK_CLSID);
        v.extend_from_slice(&flags.to_le_bytes());
        v.resize(HEADER_SIZE, 0);
        v
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"not a lnk at all, longer than header").is_none());
        assert!(parse(&[0u8; 4]).is_none());
    }

    #[test]
    fn heuristic_finds_utf16_path() {
        let mut v = header(0);
        let p = "C:\\Program Files\\Demo\\demo.exe";
        for c in p.encode_utf16() {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v.extend_from_slice(&0u16.to_le_bytes());
        let link = parse(&v).unwrap();
        assert_eq!(link.target.as_deref(), Some(p));
    }

    /// 手工构造一个带 LinkInfo 的 lnk，验证结构化解析路径（不依赖启发式兜底）。
    #[test]
    fn parses_structured_link_info() {
        let local = "C:\\Program Files\\Demo\\demo.exe";
        let suffix = "\\demo.exe";
        let mut li: Vec<u8> = vec![0u8; 0x1C];
        // volume 条目占位（解析器不读它）
        li.extend_from_slice(&[0u8; 8]);
        let local_off = li.len();
        li.extend_from_slice(local.as_bytes());
        li.push(0);
        let common_off = li.len();
        li.extend_from_slice(suffix.as_bytes());
        li.push(0);
        let block_size = li.len() as u32;
        li[0..4].copy_from_slice(&block_size.to_le_bytes());
        li[4..8].copy_from_slice(&0x1Cu32.to_le_bytes()); // HeaderSize
        li[8..12].copy_from_slice(&0x3u32.to_le_bytes()); // volume+common present
        li[12..16].copy_from_slice(&0x1Cu32.to_le_bytes()); // volume off
        li[16..20].copy_from_slice(&(local_off as u32).to_le_bytes());
        li[20..24].copy_from_slice(&(common_off as u32).to_le_bytes());

        let mut v = header(F_IS_UNICODE);
        v.extend_from_slice(&li);
        let link = parse(&v).unwrap();
        assert_eq!(link.target.as_deref(), Some(local), "degraded={}", link.degraded);
        assert!(!link.degraded);
    }

    /// 真实机器上的开始菜单快捷方式。
    ///
    /// 只断言**精确率**：凡是解析出 target 的，必须真的存在于磁盘上（说明我们没把
    /// IDList 里的目录、DOTS 里的乱码当成目标）。
    /// 召回率不做硬断言：shell 命名空间项（Magnify/Narrator/控制面板）本来就没有文件系统
    /// 目标，属于"正确地为 None"，而且 startmenu 仍会索引 .lnk 本身，启动能力不丢。
    #[test]
    fn parses_real_start_menu_links() {
        let dirs = crate::apps::startmenu::start_menu_dirs();
        if dirs.is_empty() {
            return; // 非 Windows 或无开始菜单
        }
        let mut total = 0usize;
        let mut parsed = 0usize;
        let mut exists = 0usize;
        let mut structured = 0usize;
        let mut misses: Vec<String> = Vec::new();
        let mut phantom: Vec<String> = Vec::new();

        for dir in dirs {
            for entry in walkdir::WalkDir::new(dir)
                .max_depth(4)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let p = entry.path();
                if !p.to_string_lossy().to_ascii_lowercase().ends_with(".lnk") {
                    continue;
                }
                let Ok(bytes) = std::fs::read(p) else { continue };
                let Some(link) = parse(&bytes) else {
                    panic!("真实 lnk 解析失败: {}", p.display());
                };
                total += 1;
                match &link.target {
                    Some(t) => {
                        parsed += 1;
                        if matches!(
                            link.origin,
                            TargetOrigin::LinkInfo | TargetOrigin::LinkInfoWithVolume
                        ) {
                            structured += 1;
                        }
                        if std::path::Path::new(t).exists() {
                            exists += 1;
                        } else if phantom.len() < 6 {
                            phantom.push(format!("{} -> {t}", p.display()));
                        }
                    }
                    None => {
                        if misses.len() < 6 {
                            misses.push(p.display().to_string());
                        }
                    }
                }
                if total >= 400 {
                    break;
                }
            }
        }
        if total == 0 {
            return;
        }
        let precision = if parsed > 0 {
            exists as f64 / parsed as f64
        } else {
            0.0
        };
        eprintln!(
            "lnk 真机样本 {total}：产出 target {parsed}（结构化 {structured}，启发式 {}），其中存在 {exists}，精确率 {precision:.2}",
            parsed - structured
        );
        eprintln!("  无 target（多为 shell 命名空间项，.lnk 本身仍可启动）示例: {misses:?}");
        if !phantom.is_empty() {
            eprintln!("  target 不存在（卸载残留或移动过）示例: {phantom:?}");
        }
        assert!(
            precision > 0.90,
            "解析出的 target 精确率过低: {exists}/{parsed} = {precision:.2}"
        );
        // 结构化解析必须真的生效，否则说明又退化成纯启发式了
        assert!(
            structured as f64 / total as f64 > 0.5,
            "LinkInfo 结构化解析覆盖过少: {structured}/{total}（IDList 长度语义可能又变了）"
        );
    }

    #[test]
    fn expand_env_basic() {
        std::env::set_var("EFTO_RS_TESTVAR", "Z");
        assert_eq!(expand_env("%EFTO_RS_TESTVAR%\\app.exe"), "Z\\app.exe");
        assert_eq!(expand_env("C:\\a\\b.exe"), "C:\\a\\b.exe");
        assert_eq!(expand_env("%MISSING_EFTO_X%y"), "%MISSING_EFTO_X%y");
    }

    #[test]
    fn looks_like_path_rules() {
        assert!(looks_like_path("C:\\x\\y.exe"));
        assert!(looks_like_path("\\\\srv\\share\\a.exe"));
        assert!(!looks_like_path("notepad"));
        assert!(!looks_like_path("ab"));
    }

    /// 回归：真机上"尽力读"会把相对路径/DOTS 串当成 Name，标题就成了 @ / E / ..\..\x.exe
    #[test]
    fn name_plausibility_filters_garbage() {
        assert!(plausible_display_name("Google Chrome"));
        assert!(plausible_display_name("访问互联网"));
        assert!(plausible_display_name("微信"));
        assert!(plausible_display_name("TeamViewer 15"));
        assert!(!plausible_display_name("@"));
        assert!(!plausible_display_name("E"));
        assert!(!plausible_display_name(""));
        assert!(!plausible_display_name("a"));
        assert!(!plausible_display_name("..\\..\\..\\YunShuLauncher\\YunShuLauncher.exe"));
        assert!(!plausible_display_name("C:\\Windows\\notepad.exe"));
        assert!(!plausible_display_name("unins000.exe"));
        assert!(!plausible_display_name("--flag"));
        assert!(!plausible_display_name(".."));
    }
}
