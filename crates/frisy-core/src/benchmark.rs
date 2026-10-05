//! A small, bounded disk I/O benchmark. It writes a scratch file, reads it
//! back, and does scattered random reads, timing each phase, then deletes the
//! file. It never touches existing data and never fills the disk: the total
//! size is capped and the scratch file is always removed, even on error.

use serde::Serialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct BenchOptions {
    /// Total bytes written and read back. Capped at 512 MiB for safety.
    pub total_bytes: u64,
    /// Size of each sequential read/write block.
    pub block_bytes: u64,
    /// Number of random-offset reads for the latency/IOPS phase.
    pub random_ops: u32,
    /// Run the write phase. When false, only an existing-or-created file is read.
    pub write: bool,
}

impl Default for BenchOptions {
    fn default() -> BenchOptions {
        BenchOptions { total_bytes: 64 * 1024 * 1024, block_bytes: 1024 * 1024, random_ops: 1024, write: true }
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct BenchResult {
    pub path: String,
    pub bytes: u64,
    /// Sequential write throughput in MB/s (decimal MB), absent in read-only runs.
    pub write_mbps: Option<f64>,
    /// Sequential read throughput in MB/s.
    pub read_mbps: f64,
    /// Random-read operations per second.
    pub random_iops: f64,
    /// Mean random-read latency in microseconds.
    pub random_latency_us: f64,
}

/// Deletes the scratch file when it goes out of scope, so a panic or early
/// return still cleans up.
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

const MAX_TOTAL: u64 = 512 * 1024 * 1024;

pub fn run(dir: &Path, opts: &BenchOptions) -> Result<BenchResult, String> {
    if !dir.is_dir() {
        return Err(format!("{} is not a writable directory", dir.display()));
    }
    let total = opts.total_bytes.min(MAX_TOTAL).max(opts.block_bytes);
    let block = opts.block_bytes.max(4096).min(total) as usize;
    let path = dir.join(format!(".frisy-iobench-{}", std::process::id()));
    let _scratch = Scratch(path.clone());

    // A fixed, non-zero pattern so compressing filesystems cannot cheat the write.
    let mut buf = vec![0u8; block];
    for (i, b) in buf.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(31).wrapping_add(7);
    }

    let write_mbps = if opts.write {
        let mut f = File::create(&path).map_err(|e| format!("create {}: {e}", path.display()))?;
        let start = Instant::now();
        let mut written = 0u64;
        while written < total {
            let n = block.min((total - written) as usize);
            f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            written += n as u64;
        }
        f.flush().map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?; // force to the device, not just the page cache
        Some(mbps(total, start.elapsed().as_secs_f64()))
    } else {
        // Read-only run still needs a file to read; create it without timing.
        let mut f = File::create(&path).map_err(|e| format!("create {}: {e}", path.display()))?;
        let mut written = 0u64;
        while written < total {
            let n = block.min((total - written) as usize);
            f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            written += n as u64;
        }
        f.sync_all().map_err(|e| e.to_string())?;
        None
    };

    drop_cache(&path);

    // Sequential read.
    let mut f = File::open(&path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let start = Instant::now();
    let mut read = 0u64;
    while read < total {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        read += n as u64;
    }
    let read_mbps = mbps(read, start.elapsed().as_secs_f64());

    // Random reads: seek to block-aligned offsets and read one block each.
    let blocks = (total / block as u64).max(1);
    let mut rng = Rng(0x9E3779B97F4A7C15 ^ std::process::id() as u64);
    let ops = opts.random_ops.max(1);
    let start = Instant::now();
    for _ in 0..ops {
        let b = rng.next() % blocks;
        f.seek(SeekFrom::Start(b * block as u64)).map_err(|e| e.to_string())?;
        let _ = f.read(&mut buf[..4096.min(block)]).map_err(|e| e.to_string())?;
    }
    let elapsed = start.elapsed().as_secs_f64().max(1e-9);
    let random_iops = ops as f64 / elapsed;
    let random_latency_us = elapsed / ops as f64 * 1e6;

    Ok(BenchResult {
        path: dir.display().to_string(),
        bytes: total,
        write_mbps,
        read_mbps,
        random_iops,
        random_latency_us,
    })
}

/// A plain-text summary for the advisor, naming the likely bottleneck.
pub fn report(dir: &Path, opts: &BenchOptions) -> Result<String, String> {
    let r = run(dir, opts)?;
    let mut s = format!(
        "## I/O benchmark of {}\nTested {:.0} MB. Sequential read: {:.0} MB/s",
        r.path,
        r.bytes as f64 / 1e6,
        r.read_mbps,
    );
    if let Some(w) = r.write_mbps {
        s.push_str(&format!(", sequential write: {w:.0} MB/s"));
    }
    s.push_str(&format!(
        ".\nRandom read: {:.0} IOPS at {:.0} us mean latency.\n",
        r.random_iops, r.random_latency_us
    ));
    s.push_str(&format!("Likely profile: {}.", classify(&r)));
    Ok(s)
}

/// A rough read on what the numbers imply, to seed the model's reasoning.
pub fn classify(r: &BenchResult) -> String {
    let seq = r.read_mbps;
    let lat = r.random_latency_us;
    let media = if seq > 1500.0 && lat < 200.0 {
        "fast NVMe SSD"
    } else if seq > 300.0 && lat < 1000.0 {
        "SATA SSD"
    } else if lat > 3000.0 || seq < 150.0 {
        "spinning disk or a network share (high latency dominates)"
    } else {
        "mixed or external storage"
    };
    format!("{media}; {}", if lat > 2000.0 { "random access is the bottleneck, not bandwidth" } else { "bandwidth-bound for large files" })
}

fn mbps(bytes: u64, secs: f64) -> f64 {
    if secs <= 0.0 {
        return 0.0;
    }
    bytes as f64 / 1e6 / secs
}

/// Best-effort: ask the OS to forget this file's pages so the read phase hits
/// the device. Platform calls are advisory; failure is fine.
fn drop_cache(path: &Path) {
    #[cfg(target_os = "macos")]
    unsafe {
        if let Ok(f) = File::open(path) {
            use std::os::unix::io::AsRawFd;
            // F_NOCACHE = 48: turn off caching for this descriptor.
            libc::fcntl(f.as_raw_fd(), 48, 1);
        }
    }
    #[cfg(target_os = "linux")]
    unsafe {
        if let Ok(f) = File::open(path) {
            use std::os::unix::io::AsRawFd;
            libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
    }
}

/// A tiny deterministic PRNG (SplitMix64), enough to scatter read offsets.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
}
