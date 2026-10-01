//! Read-only facts about this machine's storage: mounted volumes, and the
//! plain-text reports the advisor sends to a model. Nothing here changes state.

use crate::tree::{NodeId, Tree};
use crate::view::{bytes, count, largest, types};
use serde::Serialize;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Serialize, Debug, Clone)]
pub struct Volume {
    pub mount: String,
    pub name: String,
    pub fs: String,
    pub source: String,
    pub total: u64,
    pub free: u64,
    pub network: bool,
    pub removable: bool,
}

/// Volumes a person would recognise: the system disk, other drives and
/// network shares. Internal helper volumes are left out.
#[cfg(target_os = "macos")]
pub fn volumes() -> Vec<Volume> {
    use std::ffi::CStr;
    let mut list: *mut libc::statfs = std::ptr::null_mut();
    let n = unsafe { libc::getmntinfo(&mut list, libc::MNT_NOWAIT) };
    let mut out = Vec::new();
    if n <= 0 || list.is_null() {
        return out;
    }
    for fs in unsafe { std::slice::from_raw_parts(list, n as usize) } {
        let text = |p: &[libc::c_char]| unsafe { CStr::from_ptr(p.as_ptr()) }.to_string_lossy().into_owned();
        let mount = text(&fs.f_mntonname);
        let kind = text(&fs.f_fstypename);
        let flags = fs.f_flags;
        if kind == "devfs" || kind == "autofs" {
            continue;
        }
        if flags & libc::MNT_DONTBROWSE as u32 != 0 && mount != "/" {
            continue;
        }
        let block = fs.f_bsize as u64;
        let local = flags & libc::MNT_LOCAL as u32 != 0;
        out.push(Volume {
            name: if mount == "/" { "Macintosh HD".into() } else { last_component(&mount) },
            mount,
            fs: kind,
            source: text(&fs.f_mntfromname),
            total: fs.f_blocks * block,
            free: fs.f_bavail * block,
            network: !local,
            removable: false,
        });
    }
    out
}

#[cfg(windows)]
pub fn volumes() -> Vec<Volume> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;

    let mut out = Vec::new();
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{letter}:\\");
        let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
        if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE && kind != DRIVE_REMOTE {
            continue;
        }
        let (mut free, mut total, mut total_free) = (0u64, 0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) } == 0 || total == 0 {
            continue;
        }
        let mut label = [0u16; 261];
        let mut fs = [0u16; 261];
        unsafe {
            GetVolumeInformationW(
                wide.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            );
        }
        let text = |b: &[u16]| String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]);
        let label = text(&label);
        out.push(Volume {
            name: if label.is_empty() { format!("Drive {letter}:") } else { format!("{label} ({letter}:)") },
            mount: root.clone(),
            fs: text(&fs),
            source: root,
            total,
            free,
            network: kind == DRIVE_REMOTE,
            removable: kind == DRIVE_REMOVABLE,
        });
    }
    out
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn volumes() -> Vec<Volume> {
    let mut out = Vec::new();
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    for line in mounts.lines() {
        let mut parts = line.split_whitespace();
        let (Some(source), Some(mount), Some(fs)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let network = matches!(fs, "nfs" | "nfs4" | "cifs" | "smb3" | "fuse.sshfs");
        if !(source.starts_with("/dev/") || network) || !seen.insert(source.to_string()) {
            continue;
        }
        let Ok(c) = std::ffi::CString::new(mount) else { continue };
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 || st.f_blocks == 0 {
            continue;
        }
        out.push(Volume {
            name: if mount == "/" { "System".into() } else { last_component(mount) },
            mount: mount.to_string(),
            fs: fs.to_string(),
            source: source.to_string(),
            total: st.f_blocks as u64 * st.f_frsize as u64,
            free: st.f_bavail as u64 * st.f_frsize as u64,
            network,
            removable: false,
        });
    }
    out
}

#[cfg(unix)]
fn last_component(path: &str) -> String {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path).to_string()
}

/// Run a read-only command; `None` if it is missing, fails or takes too long.
pub(crate) fn run(tool: &str, args: &[&str], timeout: Duration, max_chars: usize) -> Option<String> {
    let mut cmd = Command::new(tool);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().ok()? {
            Some(s) => break s,
            None if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let out = reader.join().ok()?;
    if !status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out).trim().to_string();
    (!text.is_empty()).then(|| text.chars().take(max_chars).collect())
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn keep_lines(text: &str, needles: &[&str], limit: usize) -> Vec<String> {
    text.lines()
        .filter(|l| needles.iter().any(|n| l.contains(n)))
        .take(limit)
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

/// A plain-text description of the machine's storage, for the advisor prompt.
pub fn machine_report() -> String {
    let mut lines: Vec<String> = Vec::new();
    #[cfg(target_os = "macos")]
    let secs = Duration::from_secs(8);
    lines.push("## Machine".into());
    lines.push(format!("Operating system: {}", std::env::consts::OS));

    #[cfg(target_os = "macos")]
    {
        if let Some(s) = run("/usr/sbin/sysctl", &["-n", "hw.model", "machdep.cpu.brand_string", "hw.ncpu", "hw.memsize"], secs, 400) {
            let v: Vec<&str> = s.lines().collect();
            if v.len() == 4 {
                let ram = v[3].trim().parse::<u64>().unwrap_or(0);
                lines.push(format!("Model: {}, CPU: {}, {} cores, RAM: {}", v[0], v[1], v[2], bytes(ram)));
            }
        }
        if let Some(s) = run("/usr/sbin/sysctl", &["-n", "vm.swapusage"], secs, 200) {
            lines.push(format!("Swap: {s}"));
        }
    }
    #[cfg(windows)]
    {
        let ps = |script: &str| run("powershell", &["-NoProfile", "-NonInteractive", "-Command", script], Duration::from_secs(15), 2500);
        if let Some(s) = ps("Get-CimInstance Win32_ComputerSystem | Select-Object Manufacturer,Model,TotalPhysicalMemory | Format-List") {
            lines.extend(s.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()));
        }
        if let Some(s) = ps("Get-PhysicalDisk | Select-Object FriendlyName,MediaType,BusType,HealthStatus,Size | Format-Table -AutoSize | Out-String -Width 200") {
            lines.push("\n## Physical disks".into());
            lines.push(s);
        }
        if let Some(s) = ps("Get-SmbConnection | Select-Object ServerName,ShareName,Dialect | Format-Table -AutoSize | Out-String -Width 200") {
            lines.push("\n## SMB connections".into());
            lines.push(s);
        }
    }

    lines.push("\n## Mounted volumes".into());
    for v in volumes() {
        let used = if v.total > 0 { 100 - (v.free * 100 / v.total) } else { 0 };
        let kind = if v.network { "NETWORK share" } else if v.removable { "removable" } else { "local" };
        lines.push(format!(
            "- {} at {}: {kind}, {}, source {}, {} total, {} free ({used}% used)",
            v.name, v.mount, v.fs, v.source, bytes(v.total), bytes(v.free)
        ));
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(d) = run("/usr/sbin/diskutil", &["info", "/"], secs, 4000) {
            let picked = keep_lines(
                &d,
                &["Device / Media Name", "Solid State", "Protocol", "SMART Status", "File System Personality",
                  "Container Total Space", "Container Free Space", "FileVault"],
                12,
            );
            if !picked.is_empty() {
                lines.push("\n## Startup disk (diskutil info /)".into());
                lines.extend(picked);
            }
        }
        if let Some(s) = run("/usr/bin/tmutil", &["listlocalsnapshots", "/"], secs, 4000) {
            let n = s.lines().filter(|l| l.contains("com.apple")).count();
            lines.push(format!("\n## Time Machine\nLocal APFS snapshots on startup disk: {n}"));
        }
        if let Some(s) = run("/usr/bin/tmutil", &["destinationinfo"], secs, 2000) {
            lines.push("Time Machine destinations:".into());
            lines.extend(keep_lines(&s, &["Name", "Kind", "URL"], 9));
        }
        if let Some(s) = run("/usr/bin/smbutil", &["statshares", "-a"], secs, 6000) {
            let picked = keep_lines(&s, &["SERVER_NAME", "SMB_VERSION", "SIGNING_ON", "SIGNING_REQUIRED", "ENCRYPTION", "MULTICHANNEL"], 30);
            if !picked.is_empty() {
                lines.push("\n## SMB session attributes (smbutil statshares -a)".into());
                lines.extend(picked);
            }
        }
    }
    lines.join("\n")
}

/// A plain-text summary of a scan: biggest folders, file types and files.
pub fn scan_report(tree: &Tree, focus: NodeId) -> String {
    let n = &tree.nodes[focus as usize];
    let mut lines = vec![
        format!("## Disk usage scan of {}", tree.path(focus)),
        format!("Total: {} in {} files", bytes(n.size), count(n.files)),
        "\nLargest items (with their largest children):".to_string(),
    ];
    for c in tree.sorted_children(focus).into_iter().take(12) {
        let cn = &tree.nodes[c as usize];
        if cn.size == 0 {
            continue;
        }
        lines.push(format!("- {}{}: {}", cn.name, if cn.is_dir() { "/" } else { "" }, bytes(cn.size)));
        for g in tree.sorted_children(c).into_iter().take(5) {
            let gn = &tree.nodes[g as usize];
            if gn.size > cn.size / 50 {
                lines.push(format!("    - {}: {}", gn.name, bytes(gn.size)));
            }
        }
    }
    lines.push("\nBy file type:".into());
    for b in types(tree, focus).into_iter().take(10) {
        lines.push(format!("- {}: {} in {} files", b.category, bytes(b.bytes), count(b.count)));
    }
    lines.push("\nLargest single files:".into());
    for r in largest(tree, focus, 15) {
        lines.push(format!("- {}: {}", tree.path(r.id), bytes(r.size)));
    }
    lines.join("\n")
}
