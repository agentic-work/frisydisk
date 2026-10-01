//! Directory reader for Windows and non-macOS Unix, built on `std::fs`.
//! On Windows `DirEntry::metadata` comes straight from the directory listing,
//! so no extra system call is made per file.

use super::{Bounds, RawEntry, ReadError};
use std::path::Path;

pub(crate) fn bounds(root: &Path) -> Bounds {
    Bounds { devices: device_of(root).into_iter().collect(), skip_paths: Vec::new() }
}

#[cfg(unix)]
fn device_of(p: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| m.dev())
}

#[cfg(not(unix))]
fn device_of(_: &Path) -> Option<u64> {
    None
}

pub(crate) struct Reader;

impl Reader {
    pub fn new() -> Reader {
        Reader
    }

    pub fn read(&mut self, path: &Path, bounds: &Bounds) -> Result<Vec<RawEntry>, ReadError> {
        let listing = std::fs::read_dir(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ReadError::Skip,
            _ => ReadError::Denied,
        })?;
        let mut out = Vec::new();
        for entry in listing.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            let Some(info) = describe(&meta, bounds) else { continue };
            out.push(RawEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: info.is_dir,
                is_symlink: info.is_symlink,
                size: info.size,
                link: info.link,
            });
        }
        Ok(out)
    }
}

struct Info {
    is_dir: bool,
    is_symlink: bool,
    size: u64,
    link: Option<(u64, u64)>,
}

#[cfg(unix)]
fn describe(meta: &std::fs::Metadata, bounds: &Bounds) -> Option<Info> {
    use std::os::unix::fs::MetadataExt;
    let ft = meta.file_type();
    let is_dir = ft.is_dir();
    // A directory on another device is a mount point: leave it out.
    if is_dir && !bounds.devices.is_empty() && !bounds.devices.contains(&meta.dev()) {
        return None;
    }
    Some(Info {
        is_dir,
        is_symlink: ft.is_symlink(),
        size: if is_dir { 0 } else { meta.blocks() * 512 },
        link: (ft.is_file() && meta.nlink() > 1).then_some((meta.dev(), meta.ino())),
    })
}

#[cfg(windows)]
fn describe(meta: &std::fs::Metadata, _bounds: &Bounds) -> Option<Info> {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_OPEN: u32 = 0x4_0000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

    let attrs = meta.file_attributes();
    let reparse = attrs & REPARSE_POINT != 0;
    let cloud_only = attrs & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0;
    // Junctions, symlinks and mount points are listed but never entered.
    // Cloud placeholder folders (OneDrive) are reparse points too, and those are entered.
    let is_dir = meta.is_dir() && (!reparse || cloud_only);
    let is_link = reparse && !cloud_only;
    Some(Info {
        is_dir,
        is_symlink: is_link,
        // A cloud-only file takes no local space.
        size: if is_dir || is_link || cloud_only { 0 } else { meta.file_size() },
        link: None,
    })
}
