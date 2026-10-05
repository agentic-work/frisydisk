//! The I/O benchmark runs for real against a temp dir, stays bounded, and
//! cleans up after itself. No mocks: it times actual reads and writes.

use frisy_core::benchmark::{self, BenchOptions};

#[test]
fn benchmark_measures_read_write_and_random_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let before: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert!(before.is_empty());

    // Small, fast settings so the test is quick but still exercises every phase.
    let opts = BenchOptions { total_bytes: 4 * 1024 * 1024, block_bytes: 64 * 1024, random_ops: 64, write: true };
    let r = benchmark::run(dir.path(), &opts).unwrap();

    // Sequential write and read both ran and reported a positive throughput.
    assert!(r.write_mbps.unwrap() > 0.0, "{r:?}");
    assert!(r.read_mbps > 0.0, "{r:?}");
    // Random read reported latency and IOPS.
    assert!(r.random_iops > 0.0 && r.random_latency_us > 0.0, "{r:?}");
    assert_eq!(r.bytes, opts.total_bytes);

    // The scratch file is gone: the benchmark leaves nothing behind.
    let after: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert!(after.is_empty(), "benchmark left files behind: {} entries", after.len());
}

#[test]
fn read_only_benchmark_skips_the_write_number() {
    let dir = tempfile::tempdir().unwrap();
    let opts = BenchOptions { total_bytes: 2 * 1024 * 1024, block_bytes: 64 * 1024, random_ops: 32, write: false };
    let r = benchmark::run(dir.path(), &opts).unwrap();
    // Read-only: no write throughput, but reads and random still measured.
    assert!(r.write_mbps.is_none());
    assert!(r.read_mbps > 0.0 && r.random_iops > 0.0);
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
}

#[test]
fn a_bad_path_is_an_error_not_a_panic() {
    let opts = BenchOptions::default();
    let r = benchmark::run(std::path::Path::new("/no/such/dir/anywhere/frisy"), &opts);
    assert!(r.is_err());
}

#[test]
fn the_report_names_the_bottleneck_in_plain_text() {
    let dir = tempfile::tempdir().unwrap();
    let opts = BenchOptions { total_bytes: 2 * 1024 * 1024, block_bytes: 64 * 1024, random_ops: 32, write: true };
    let text = benchmark::report(dir.path(), &opts).unwrap();
    assert!(text.contains("I/O benchmark"));
    assert!(text.to_lowercase().contains("read") && text.to_lowercase().contains("random"));
    // Gives the model something to reason about: a throughput figure in MB/s.
    assert!(text.contains("MB/s"));
}
