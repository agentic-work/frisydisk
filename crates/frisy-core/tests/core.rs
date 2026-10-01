use frisy_core::advisor;
use frisy_core::api::Api;
use frisy_core::scan::Scan;
use frisy_core::tree::{Entry, Tree};
use frisy_core::view;
use serde_json::json;
use std::fs;
use std::path::Path;

/// Write `bytes` of incompressible data so the file system cannot store it sparsely.
fn write(root: &Path, rel: &str, bytes: usize) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let mut data = vec![0u8; bytes];
    let mut x = 0x9E37_79B9_7F4A_7C15u64 ^ bytes as u64;
    for b in data.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = x as u8;
    }
    fs::write(p, data).unwrap();
}

/// What the scanner should report for one file on this platform.
fn on_disk(root: &Path, rel: &str) -> u64 {
    let m = fs::symlink_metadata(root.join(rel)).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        m.blocks() * 512
    }
    #[cfg(not(unix))]
    {
        m.len()
    }
}

fn names(tree: &Tree, ids: &[u32]) -> Vec<String> {
    ids.iter().map(|&i| tree.nodes[i as usize].name.to_string()).collect()
}

#[test]
fn totals_match_sizes_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "a.bin", 100_000);
    write(r, "sub/b.bin", 300_000);
    write(r, "sub/deep/c.bin", 50_000);
    fs::create_dir(r.join("empty")).unwrap();

    let scan = Scan::run(r.to_str().unwrap());
    let tree = scan.tree.lock().unwrap();
    let root = &tree.nodes[0];
    assert_eq!(root.size, on_disk(r, "a.bin") + on_disk(r, "sub/b.bin") + on_disk(r, "sub/deep/c.bin"));
    assert_eq!(root.files, 3);
    let p = scan.progress();
    assert_eq!(p.directories, 4);
    assert!(!p.scanning);

    // Children are sorted largest first and directory totals include descendants.
    assert_eq!(names(&tree, &root.children), ["sub", "a.bin", "empty"]);
    let sub = root.children[0];
    assert_eq!(tree.nodes[sub as usize].size, on_disk(r, "sub/b.bin") + on_disk(r, "sub/deep/c.bin"));
    let b = tree.nodes[sub as usize].children[0];
    assert!(Path::new(&tree.path(b)).ends_with("sub/b.bin"));
    assert!(Path::new(&tree.path(b)).is_file());
}

#[test]
fn many_files_across_many_directories() {
    let dir = tempfile::tempdir().unwrap();
    for d in 0..40 {
        for f in 0..25 {
            write(dir.path(), &format!("d{d}/n{}/f{f}.dat", d % 3), 4096);
        }
    }
    let scan = Scan::run(dir.path().to_str().unwrap());
    let tree = scan.tree.lock().unwrap();
    assert_eq!(tree.nodes[0].files, 1000);
    assert_eq!(tree.nodes[0].size, 1000 * on_disk(dir.path(), "d0/n0/f0.dat"));
    assert_eq!(tree.nodes[0].children.len(), 40);
}

#[cfg(unix)]
#[test]
fn hard_links_are_counted_once() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "one.bin", 200_000);
    fs::hard_link(dir.path().join("one.bin"), dir.path().join("two.bin")).unwrap();
    let scan = Scan::run(dir.path().to_str().unwrap());
    let tree = scan.tree.lock().unwrap();
    assert_eq!(tree.nodes[0].size, on_disk(dir.path(), "one.bin"));
    assert_eq!(tree.nodes[0].files, 2);
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    write(other.path(), "big.bin", 500_000);
    write(dir.path(), "small.bin", 10_000);
    std::os::unix::fs::symlink(other.path(), dir.path().join("link")).unwrap();
    let scan = Scan::run(dir.path().to_str().unwrap());
    let tree = scan.tree.lock().unwrap();
    assert!(tree.nodes[0].size < 100_000);
    let link = tree.nodes[0].children.iter().map(|&c| &tree.nodes[c as usize]).find(|n| &*n.name == "link").unwrap();
    assert!(!link.is_dir() && link.flags & frisy_core::tree::FLAG_SYMLINK != 0);
}

#[cfg(unix)]
#[test]
fn unreadable_directory_is_reported_not_fatal() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "ok.bin", 10_000);
    write(dir.path(), "locked/secret.bin", 10_000);
    let locked = dir.path().join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0)).unwrap();
    let scan = Scan::run(dir.path().to_str().unwrap());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    // Running as root can read anything, so there is nothing to assert.
    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    assert_eq!(scan.progress().inaccessible, 1);
    let tree = scan.tree.lock().unwrap();
    assert_eq!(tree.nodes[0].files, 1);
    let n = tree.nodes[0].children.iter().map(|&c| &tree.nodes[c as usize]).find(|n| &*n.name == "locked").unwrap();
    assert!(n.flags & frisy_core::tree::FLAG_INACCESSIBLE != 0);
}

#[cfg(unix)]
extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

fn file(name: &str, size: u64) -> Entry {
    Entry { name: name.into(), is_dir: false, is_symlink: false, size }
}

fn dir(name: &str) -> Entry {
    Entry { name: name.into(), is_dir: true, is_symlink: false, size: 0 }
}

/// root(100) = big(60: x.mov 40, y.zip 20) + mid.mov(30) + ten 1-byte .txt files.
fn sample() -> Tree {
    let mut t = Tree::new("/r");
    let mut entries = vec![dir("big"), file("mid.mov", 30)];
    entries.extend((0..10).map(|i| file(&format!("s{i}.txt"), 1)));
    let dirs = t.attach(0, entries);
    t.attach(dirs[0], vec![file("x.mov", 40), file("y.zip", 20)]);
    t.sort_all();
    t
}

#[test]
fn tree_totals_paths_and_collector_moves() {
    let mut t = sample();
    assert_eq!((t.nodes[0].size, t.nodes[0].files), (100, 13));
    let big = t.nodes[0].children[0];
    assert_eq!(&*t.nodes[big as usize].name, "big");
    let x = t.nodes[big as usize].children[0];
    assert_eq!(t.path(x), format!("/r{0}big{0}x.mov", std::path::MAIN_SEPARATOR));
    assert_eq!(t.display_name(0), "r");
    assert!(t.is_descendant(x, big) && !t.is_descendant(big, x));

    assert!(t.detach(big));
    assert!(!t.detach(big), "already detached");
    assert!(!t.detach(0), "the root cannot be detached");
    assert_eq!((t.nodes[0].size, t.nodes[0].files), (40, 11));
    assert!(t.reattach(big));
    assert_eq!((t.nodes[0].size, t.nodes[0].files), (100, 13));
    assert_eq!(t.nodes[0].children[0], big, "goes back in size order");
}

#[test]
fn view_groups_small_items_and_keeps_totals() {
    let t = sample();
    let v = view::view(&t, 0, 6, 0.05);
    assert_eq!(v.children.len(), 3); // big, mid.mov, group of ten
    assert_eq!(v.children[0].name, "big");
    assert_eq!(v.children[0].children.len(), 2);
    let group = &v.children[2];
    assert!(group.id.is_none());
    assert_eq!((group.group, group.size), (10, 10));
    assert_eq!(v.children.iter().map(|c| c.size).sum::<u64>(), v.size);

    let shallow = view::view(&t, 0, 1, 0.0);
    assert!(shallow.children[0].children.is_empty());
    assert_eq!(shallow.children.len(), 12);
}

#[test]
fn largest_search_types_and_formatting() {
    let t = sample();
    let big: Vec<String> = view::largest(&t, 0, 3).into_iter().map(|r| r.name).collect();
    assert_eq!(big, ["x.mov", "mid.mov", "y.zip"]);
    let hits: Vec<String> = view::search(&t, 0, "MOV", 50).into_iter().map(|r| r.name).collect();
    assert_eq!(hits, ["x.mov", "mid.mov"]);
    let types = view::types(&t, 0);
    assert_eq!((types[0].category.as_str(), types[0].bytes), ("Video", 70));
    assert_eq!(types.iter().find(|b| b.category == "Documents").unwrap().count, 10);
    assert_eq!(view::category(".gitignore"), "No extension");
    assert_eq!(view::bytes(1_500_000_000), "1.50 GB");
    assert_eq!(view::bytes(999), "999 bytes");
    assert_eq!(view::count(1_234_567), "1,234,567");
}

#[test]
fn advisor_prompt_is_non_destructive_and_answers_are_checked() {
    let prompt = advisor::system_prompt();
    assert!(prompt.contains("NON-DESTRUCTIVE") && prompt.contains("Never recommend deleting"));
    let user = advisor::user_prompt("## Mounted volumes\n- NAS", Some("## Disk usage scan of /x"));
    assert!(user.contains("- NAS") && user.contains("/x"));
    let report = frisy_core::facts::scan_report(&sample(), 0);
    assert!(report.contains("big/: 60 bytes") && report.contains("Video"));

    let answer = "cp -av a /Volumes/nas/\n  rm -rf ~/work/a\nln -s /Volumes/nas/a ~/work/a\nsudo rm /x\n\
rsync -a --delete a b\nformat the report\nls && rm b\n# Optional: rm later\nRemove-Item C:\\old -Recurse\n\
robocopy C:\\a \\\\nas\\a /E\nrobocopy C:\\a \\\\nas\\a /MIR\ndel /s C:\\temp\nThe model was told never to delete anything.";
    assert_eq!(
        advisor::destructive_lines(answer),
        ["rm -rf ~/work/a", "sudo rm /x", "rsync -a --delete a b", "ls && rm b", "Remove-Item C:\\old -Recurse",
         "robocopy C:\\a \\\\nas\\a /MIR", "del /s C:\\temp"]
    );
}

#[test]
fn volumes_include_the_system_disk() {
    let vols = frisy_core::facts::volumes();
    assert!(!vols.is_empty());
    assert!(vols.iter().all(|v| v.total > 0 && v.free <= v.total));
    #[cfg(windows)]
    assert!(vols.iter().any(|v| v.mount.ends_with(":\\")));
    #[cfg(target_os = "macos")]
    assert!(vols.iter().any(|v| v.mount == "/"));
}

/// The same calls the UI makes, end to end, including moving a file to the Trash.
#[test]
fn api_scan_view_collect_and_trash() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "videos/big.mov", 400_000);
    write(r, "videos/clip.mp4", 150_000);
    write(r, "docs/report.pdf", 80_000);
    write(r, "trash-me.bin", 50_000);

    let api = Api::new();
    assert_eq!(api.call("status", &json!({})).unwrap()["open"], false);
    assert!(api.call("view", &json!({ "focus": 0 })).is_err());
    assert!(api.call("start_scan", &json!({ "path": r.join("nope").to_str().unwrap() })).is_err());
    api.call("start_scan", &json!({ "path": r.to_str().unwrap() })).unwrap();
    while api.call("status", &json!({})).unwrap()["progress"]["scanning"] == true {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let v = api.call("view", &json!({ "focus": 0, "depth": 4, "min_fraction": 0.0 })).unwrap();
    assert_eq!(v["tree"]["files"], 4);
    assert_eq!(v["rows"][0]["name"], "videos");
    assert_eq!(v["crumbs"].as_array().unwrap().len(), 1);
    let videos = v["rows"][0]["id"].as_u64().unwrap();
    let inner = api.call("view", &json!({ "focus": videos })).unwrap();
    assert_eq!(inner["crumbs"].as_array().unwrap().len(), 2);
    assert_eq!(inner["rows"][0]["name"], "big.mov");
    assert_eq!(api.call("types", &json!({ "focus": 0 })).unwrap()[0]["category"], "Video");
    assert_eq!(api.call("largest", &json!({ "focus": 0, "limit": 1 })).unwrap()[0]["name"], "big.mov");
    assert_eq!(api.call("search", &json!({ "focus": 0, "query": "report" })).unwrap()[0]["name"], "report.pdf");

    let total = v["tree"]["size"].as_u64().unwrap();
    let victim = v["rows"].as_array().unwrap().iter().find(|row| row["name"] == "trash-me.bin").unwrap();
    let (victim_id, victim_size) = (victim["id"].as_u64().unwrap(), victim["size"].as_u64().unwrap());

    // Staging changes the picture but not the disk, and can be undone.
    let staged = api.call("stage", &json!({ "id": victim_id })).unwrap();
    assert_eq!(staged["collector"]["items"].as_array().unwrap().len(), 1);
    assert!(api.call("stage", &json!({ "id": victim_id })).is_err());
    assert!(api.call("stage", &json!({ "id": 0 })).is_err());
    let after = api.call("view", &json!({ "focus": 0 })).unwrap();
    assert_eq!(after["tree"]["size"].as_u64().unwrap(), total - victim_size);
    assert!(r.join("trash-me.bin").exists());
    api.call("unstage", &json!({})).unwrap();
    assert_eq!(api.call("view", &json!({ "focus": 0 })).unwrap()["tree"]["size"].as_u64().unwrap(), total);

    // Moving to the Trash needs a desktop session; skip that part on headless CI runners.
    api.call("stage", &json!({ "id": victim_id })).unwrap();
    let result = api.call("trash_collector", &json!({})).unwrap();
    if result["moved"] == 1 {
        assert!(!r.join("trash-me.bin").exists());
        assert_eq!(result["freed"].as_u64().unwrap(), victim_size);
        assert_eq!(api.call("view", &json!({ "focus": 0 })).unwrap()["tree"]["size"].as_u64().unwrap(), total - victim_size);
    } else {
        // A failed move puts the item back in the chart.
        assert_eq!(result["failed"].as_array().unwrap().len(), 1);
        assert_eq!(api.call("view", &json!({ "focus": 0 })).unwrap()["tree"]["size"].as_u64().unwrap(), total);
    }
    assert!(r.join("videos/big.mov").exists());

    assert_eq!(api.call("advisor_poll", &json!({})).unwrap()["started"], false);
    assert!(api.call("nonsense", &json!({})).is_err());
}
