//! 本机文件的读写：列盘符 / 系统文件夹、列目录、按块读写文件。
//!
//! 三方共用同一套代码：
//!
//! * 文件传输窗口里"本机"那一栏（`lib.rs` 的 `local_fs_*` 命令）；
//! * 文件传输的另一端——别的设备通过账号服务来读写这台电脑（`src/peer.rs`）；
//! * 它们都只做**单个文件**（不递归文件夹），没有目录树、没有断点续传的元数据：
//!   一个任务就是一个打开着的文件句柄（`id` → 句柄），对面按块读 / 按块写。
//!
//! 块读写是磁盘 IO，可能慢（机械盘、网络盘）：调用方在 `spawn_blocking` 里跑。

use std::collections::HashMap;
use std::sync::OnceLock;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// 文件传输里的一条目录项
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsEntry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    /// 修改时间（Unix 秒；0 = 不知道）
    pub modified: i64,
}

/// 能直接跳过去的一个"位置"：盘符、桌面、下载……
///
/// `kind` 给界面挑图标 / 翻译用（`drive`、`documents`、`downloads`……）；
/// `name` 只有盘有卷标时才有值（`Data`），系统文件夹的名字由界面翻译。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsPlace {
    pub name: String,
    pub path: String,
    pub kind: String,
}

/// 会话里打开着的文件
#[derive(Default)]
pub struct Jobs {
    open: HashMap<u32, File>,
}

impl Jobs {
    pub fn open(&mut self, id: u32, path: &str, write: bool) -> Result<u64, String> {
        if self.open.contains_key(&id) {
            return Err(format!("file {id} is already open"));
        }
        if path.trim().is_empty() {
            return Err("empty path".into());
        }
        let file = if write {
            // 覆盖写：控制端已经问过"是否替换"，走到这里就是用户选了替换
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .map_err(|e| e.to_string())?
        } else {
            File::open(path).map_err(|e| e.to_string())?
        };
        let size = if write {
            0
        } else {
            file.metadata().map(|m| m.len()).unwrap_or(0)
        };
        self.open.insert(id, file);
        Ok(size)
    }

    /// 读一块：返回（数据，是否到了文件末尾）
    pub fn read(&mut self, id: u32, offset: u64, len: u32) -> Result<(Vec<u8>, bool), String> {
        let file = self.open.get_mut(&id).ok_or_else(|| not_open(id))?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; len as usize];
        let mut filled = 0;
        while filled < buf.len() {
            match file.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        buf.truncate(filled);
        let eof = filled < len as usize;
        Ok((buf, eof))
    }

    /// 写一块（从 `offset` 开始，之前的都要写过）
    pub fn write(&mut self, id: u32, offset: u64, data: &[u8]) -> Result<(), String> {
        let file = self.open.get_mut(&id).ok_or_else(|| not_open(id))?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        file.write_all(data).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 现在打开着哪些文件（隧道断开时要全部关掉）
    pub fn ids(&self) -> Vec<u32> {
        self.open.keys().copied().collect()
    }

    pub fn close(&mut self, id: u32) {
        if let Some(mut file) = self.open.remove(&id) {
            // 写完的文件要落盘：控制端看到"已发送"，用户可能马上就要打开它
            let _ = file.flush();
        }
    }
}

fn not_open(id: u32) -> String {
    format!("file {id} isn't open")
}

/// 文件传输窗口里能直接跳过去的位置：各个盘 + 桌面 / 下载这些系统文件夹
/// （文件传输窗口打开时问一次，界面拿去画路径栏的下拉菜单）。
///
/// 盘符用 `GetLogicalDrives` 的位图，**不去挨个试探**：对拔掉的读卡器、离线的网络盘调
/// `Path::exists()` 会挂好几秒（文件传输窗口一打开就卡死，就是这么来的）。
pub fn roots() -> (Vec<FsPlace>, String) {
    let mut places = drive_places();
    places.extend(folder_places());
    let home = home_dir()
        .or_else(|| places.first().map(|p| p.path.clone()))
        .unwrap_or_else(|| "C:\\".into());
    (places, home)
}

/// 有哪些盘：`GetLogicalDrives` 的第 i 位就是 `A + i`，卷标顺带读出来（不碰盘，只问系统）
fn drive_places() -> Vec<FsPlace> {
    use windows::Win32::Storage::FileSystem::GetLogicalDrives;
    let mask = unsafe { GetLogicalDrives() };
    (0..26u32)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| {
            let path = format!("{}:\\", (b'A' + i as u8) as char);
            FsPlace {
                name: volume_label(&path),
                path,
                kind: "drive".into(),
            }
        })
        .collect()
}

/// 盘的卷标（`Data` 这种）；读不出来就是空字符串（界面只显示 `(D:)`）
fn volume_label(root: &str) -> String {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;
    let wide = HSTRING::from(root);
    // 缓冲区要给够：卷标最多 MAX_PATH+1 个字符、文件系统名（`NTFS`）也是，
    // 给小了 `GetVolumeInformationW` 直接失败（所有盘都变成没有名字的 `C:`）
    let mut name = [0u16; 261];
    let mut fs = [0u16; 261];
    let mut serial = 0u32;
    let mut max_len = 0u32;
    let mut flags = 0u32;
    let ok = unsafe {
        GetVolumeInformationW(
            &wide,
            Some(&mut name),
            Some(&mut serial),
            Some(&mut max_len),
            Some(&mut flags),
            Some(&mut fs),
        )
    };
    if ok.is_err() {
        return String::new();
    }
    let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
    String::from_utf16_lossy(&name[..len])
}

/// 桌面 / 文档 / 下载这些系统文件夹（按用户的设置走，可能被重定向到别的盘）
fn folder_places() -> Vec<FsPlace> {
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos,
        FOLDERID_Documents, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
    };
    let known: [(&str, &windows::core::GUID); 6] = [
        ("desktop", &FOLDERID_Desktop),
        ("documents", &FOLDERID_Documents),
        ("downloads", &FOLDERID_Downloads),
        ("pictures", &FOLDERID_Pictures),
        ("videos", &FOLDERID_Videos),
        ("music", &FOLDERID_Music),
    ];
    known
        .into_iter()
        .filter_map(|(kind, id)| {
            let raw = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok()?;
            let path = unsafe { raw.to_string().ok() }?;
            // 释放 CoTaskMemAlloc 出来的字符串
            unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(raw.0 as *const _)) };
            Some(FsPlace {
                name: String::new(),
                path,
                kind: kind.into(),
            })
        })
        .collect()
}

/// 用户的默认目录（`USERPROFILE`，没有就 `HOME`）。只读环境变量、不碰磁盘
fn home_dir() -> Option<String> {
    static HOME: OnceLock<Option<String>> = OnceLock::new();
    HOME.get_or_init(|| {
        for key in ["USERPROFILE", "HOME"] {
            if let Ok(v) = std::env::var(key)
                && !v.trim().is_empty()
            {
                return Some(v);
            }
        }
        None
    })
    .clone()
}

/// 列一个目录：返回（解析好的绝对路径，上级目录，条目）。`path` 为空 = 默认目录
pub fn list(path: &str) -> Result<(String, Option<String>, Vec<FsEntry>), String> {
    let raw = if path.trim().is_empty() {
        home_dir().unwrap_or_else(|| "C:\\".into())
    } else {
        path.to_owned()
    };
    let dir = std::fs::canonicalize(&raw).map_err(|e| e.to_string())?;
    if !dir.is_dir() {
        return Err("not a directory".into());
    }
    let mut entries = Vec::new();
    for item in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let Ok(item) = item else { continue };
        let Ok(meta) = item.metadata() else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        entries.push(FsEntry {
            name,
            dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified,
        });
    }
    // 目录在前，同一类按名字排（不区分大小写，和资源管理器一样）
    entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let parent = dir.parent().map(display_path);
    Ok((display_path(&dir), parent, entries))
}

pub fn exists(path: &str) -> bool {
    !path.trim().is_empty() && Path::new(path).is_file()
}

/// 显示用的路径：去掉 `canonicalize` 加上的 `\\?\` 前缀
fn display_path(p: &Path) -> String {
    let text = p.to_string_lossy();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}

/// 目录是否可写（界面上"发送到这里"能不能点）
pub fn writable(path: &str) -> bool {
    let p = PathBuf::from(path);
    p.is_dir() && {
        let probe = p.join(".nife-remote-probe");
        let ok = File::create(&probe).is_ok();
        let _ = std::fs::remove_file(&probe);
        ok
    }
}
