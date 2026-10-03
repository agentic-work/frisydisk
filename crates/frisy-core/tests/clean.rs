//! Cleanup and hidden-space tests. Everything runs inside temp folders.

use frisy_core::clean::{self, Mode, Places};
use frisy_core::hidden::{self, Part};
use frisy_core::scan::Scan;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

fn write(p: &Path, bytes: usize) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![7u8; bytes]).unwrap();
}

fn make_old(p: &Path, days: u64) {
    let when = SystemTime::now() - Duration::from_secs(days * 86_400);
    fs::File::options().write(true).open(p).unwrap().set_modified(when).unwrap();
}

fn places(home: &Path) -> Places {
    Places { home: home.to_path_buf(), temp: home.join("tmp"), local_app_data: Some(home.join("AppData/Local")), windows_dir: None }
}

fn target(home: &Path, id: &str) -> clean::Target {
    clean::targets(&places(home), std::env::consts::OS).into_iter().find(|t| t.id == id).unwrap()
}

#[test]
fn recently_used_files_are_left_alone() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    write(&h.join("tmp/a.log"), 50_000);
    write(&h.join("tmp/b.log"), 50_000);
    // Both were just written, so the day-old rule leaves them alone.
    let t = target(h, "temp");
    assert_eq!(clean::measure(&t).count, 0);
    // Even with an old modified time: the files were created just now.
    make_old(&h.join("tmp/a.log"), 30);
    if !cfg!(target_os = "macos") {
        // (macOS moves the creation time back with the modified time.)
        assert_eq!(clean::measure(&t).count, 0);
    }
}

/// A target with no age limit, so tests do not depend on file timestamps.
fn any_age(home: &Path, id: &str) -> clean::Target {
    let mut t = target(home, id);
    t.min_age = Duration::ZERO;
    t
}

#[test]
fn temp_files_are_removed_but_the_folder_stays() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    write(&h.join("tmp/old.log"), 50_000);
    write(&h.join("tmp/sub/deep.bin"), 80_000);
    let found = clean::measure(&any_age(h, "temp"));
    assert_eq!(found.count, 2);
    assert!(found.bytes >= 130_000);
    let report = clean::clean(&[found], Mode::Delete);
    assert_eq!((report.removed, report.skipped), (2, 0));
    assert!(!h.join("tmp/old.log").exists() && !h.join("tmp/sub").exists());
    assert!(h.join("tmp").is_dir(), "the folder itself stays");
}

#[test]
fn caches_are_measured_and_deleted() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let (caches, id) = match std::env::consts::OS {
        "macos" => (h.join("Library/Caches"), "caches"),
        "windows" => (h.join("AppData/Local/CrashDumps"), "crash-dumps"),
        _ => (h.join(".cache"), "caches"),
    };
    write(&caches.join("app/blob.bin"), 200_000);
    write(&caches.join("other.bin"), 10_000);
    let found = clean::measure(&any_age(h, id));
    assert_eq!(found.count, 2);
    assert!(found.bytes >= 210_000);
    let report = clean::clean(&[found], Mode::Delete);
    assert_eq!(report.removed, 2);
    assert!(caches.is_dir() && fs::read_dir(&caches).unwrap().next().is_none());
}

#[test]
fn an_item_replaced_after_measuring_is_skipped() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    write(&h.join("tmp/swap.bin"), 1000);
    let found = clean::measure(&any_age(h, "temp"));
    assert_eq!(found.count, 1);
    // Replace it with a folder under the same name.
    fs::remove_file(h.join("tmp/swap.bin")).unwrap();
    fs::create_dir_all(h.join("tmp/swap.bin")).unwrap();
    let report = clean::clean(&[found], Mode::Delete);
    assert_eq!((report.removed, report.skipped), (0, 1));
    assert!(h.join("tmp/swap.bin").is_dir());
}

#[test]
fn nothing_outside_a_cleanup_folder_can_be_removed() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    write(&h.join("tmp/old.log"), 1000);
    write(&h.join("precious.txt"), 1000);

    let mut found = clean::measure(&any_age(h, "temp"));
    // Tamper with the measured list, as a bug or a race might.
    found.items.push((h.join("precious.txt"), 1000));
    found.items.push((h.join("tmp/../precious.txt"), 1000));
    found.items.push((h.join("tmp"), 1000));
    let report = clean::clean(&[found], Mode::Delete);
    assert_eq!(report.removed, 1);
    assert_eq!(report.skipped, 3);
    assert!(h.join("precious.txt").exists());
    assert!(h.join("tmp").is_dir());
}

#[cfg(unix)]
#[test]
fn symlinks_are_removed_not_followed() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    write(&h.join("keep/important.txt"), 1000);
    fs::create_dir_all(h.join("tmp")).unwrap();
    std::os::unix::fs::symlink(h.join("keep"), h.join("tmp/link")).unwrap();
    let found = clean::measure(&any_age(h, "temp"));
    assert_eq!(found.count, 1);
    let report = clean::clean(&[found], Mode::Delete);
    assert_eq!(report.removed, 1);
    assert!(!h.join("tmp/link").exists());
    assert!(h.join("keep/important.txt").exists(), "the link's target is untouched");
}

#[test]
fn unused_build_folders_are_found_only_in_stale_projects() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    for (project, old) in [("old-app", true), ("new-app", false)] {
        write(&r.join(project).join("package.json"), 100);
        write(&r.join(project).join("node_modules/dep/big.js"), 2_000_000);
        if old {
            make_old(&r.join(project).join("package.json"), 200);
        }
    }
    // A Rust project, and a `target` folder that is not a Rust build.
    write(&r.join("rusty/Cargo.toml"), 100);
    write(&r.join("rusty/target/debug/app"), 3_000_000);
    make_old(&r.join("rusty/Cargo.toml"), 200);
    write(&r.join("photos/target/keep.jpg"), 3_000_000);
    make_old(&r.join("photos/target/keep.jpg"), 400);

    let scan = Scan::run(r.to_str().unwrap());
    let tree = scan.tree.lock().unwrap();
    let found = clean::stale_build_folders(&tree, 90);
    let mut names: Vec<String> = found.examples.iter().map(|p| {
        let p = Path::new(p);
        format!("{}/{}", p.parent().unwrap().file_name().unwrap().to_string_lossy(), p.file_name().unwrap().to_string_lossy())
    }).collect();
    names.sort();
    assert_eq!(names, ["old-app/node_modules", "rusty/target"]);

    let report = clean::clean(&[found], Mode::Delete);
    assert_eq!(report.removed, 2);
    assert!(!r.join("old-app/node_modules").exists());
    assert!(r.join("old-app/package.json").exists());
    assert!(r.join("new-app/node_modules").exists());
    assert!(r.join("photos/target/keep.jpg").exists());
}

#[test]
fn hidden_space_is_split_and_capped() {
    assert_eq!(hidden::split(100_000, 99_900, vec![]), None, "under half a percent is noise");
    let h = hidden::split(1000, 600, vec![
        Part { name: "Swap".into(), bytes: 150 },
        Part { name: "Recovery".into(), bytes: 300 },
        Part { name: "Empty".into(), bytes: 0 },
    ]).unwrap();
    assert_eq!(h.bytes, 400);
    assert_eq!(h.parts.iter().map(|p| (p.name.as_str(), p.bytes)).collect::<Vec<_>>(), [("Recovery", 300), ("Swap", 100)]);
    assert_eq!(h.other, 0);
    let h = hidden::split(1000, 500, vec![Part { name: "Swap".into(), bytes: 100 }]).unwrap();
    assert_eq!((h.bytes, h.other), (500, 400));
    assert_eq!(hidden::split(500, 900, vec![]), None);
}

#[test]
fn every_platform_has_targets_and_none_is_the_home_folder() {
    let home = tempfile::tempdir().unwrap();
    for os in ["macos", "windows", "linux"] {
        let t = clean::targets(&places(home.path()), os);
        assert!(t.iter().any(|x| x.id == "temp"), "{os}");
        assert!(t.iter().all(|x| x.roots.iter().all(|r| r != home.path() && r.parent().is_some())), "{os}");
    }
}
