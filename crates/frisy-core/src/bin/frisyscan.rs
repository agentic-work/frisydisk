//! frisyscan PATH [--top N] [--advise [MODEL]]   scan and print totals
//! frisyscan --facts                             the machine report the advisor sees
//! frisyscan serve [--port N] [--ui DIR]         serve the UI and API on 127.0.0.1 for development

use frisy_core::advisor::{self, Message};
use frisy_core::settings::Settings;
use frisy_core::api::Api;
use frisy_core::view::{bytes, count};
use frisy_core::{facts, scan::Scan};
use std::io::Write;
use std::sync::atomic::AtomicBool;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut option = |name: &str| -> Option<String> {
        let i = args.iter().position(|a| a == name)?;
        args.remove(i);
        if i < args.len() && !args[i].starts_with("--") {
            Some(args.remove(i))
        } else {
            Some(String::new())
        }
    };
    let top: usize = option("--top").and_then(|v| v.parse().ok()).unwrap_or(15);
    let advise = option("--advise");
    let port: u16 = option("--port").and_then(|v| v.parse().ok()).unwrap_or(7878);
    let ui = option("--ui").unwrap_or_else(|| "ui".into());
    let show_facts = option("--facts").is_some();

    if show_facts {
        println!("{}", facts::machine_report());
        return;
    }
    if args.first().map(String::as_str) == Some("serve") {
        serve(port, &ui);
        return;
    }
    let Some(path) = args.first() else {
        eprintln!("usage: frisyscan PATH [--top N] [--advise [MODEL]] | --facts | serve [--port N] [--ui DIR]");
        std::process::exit(2);
    };

    let scan = Scan::run(path);
    let p = scan.progress();
    let tree = scan.tree.lock().unwrap();
    let root = &tree.nodes[0];
    println!("{}", scan.root_path);
    println!(
        "bytes={} files={} dirs={} inaccessible={} seconds={:.2}",
        root.size, root.files, p.directories, p.inaccessible, p.seconds
    );
    println!("total {} in {} files", bytes(root.size), count(root.files));
    for &c in root.children.iter().take(top) {
        let n = &tree.nodes[c as usize];
        println!("{:>12}  {}{}", bytes(n.size), n.name, if n.is_dir() { "/" } else { "" });
    }

    if let Some(model) = advise {
        // MODEL may be a model name or a provider: ollama, agenticwork, anthropic, openai.
        let settings = Settings::load();
        let all = advisor::backends(&settings);
        let backend = if model.is_empty() { all.into_iter().next() } else { all.into_iter().find(|b| b.model == model || b.kind == model) };
        let Some(backend) = backend else {
            eprintln!("no usable model: start Ollama, or add a provider key in the app's settings");
            std::process::exit(1);
        };
        println!("\n--- advisor ({}) ---", backend.label);
        let messages = vec![
            Message { role: "system".into(), text: advisor::system_prompt() },
            Message { role: "user".into(), text: advisor::user_prompt(&facts::machine_report(), Some(&facts::scan_report(&tree, 0))) },
        ];
        let mut answer = String::new();
        let result = advisor::stream(&settings, &messages, &backend, &AtomicBool::new(false), &mut |chunk| {
            answer.push_str(chunk);
            print!("{chunk}");
            let _ = std::io::stdout().flush();
        });
        println!();
        if let Err(e) = result {
            eprintln!("advisor failed: {e}");
            std::process::exit(1);
        }
        let risky = advisor::destructive_lines(&answer);
        if !risky.is_empty() {
            println!("\nWARNING: the model ignored the non-destructive rule. Skip these lines:");
            for line in risky {
                println!("  {line}");
            }
        }
    }
}

/// Serve the UI folder and the API over HTTP on localhost, so the interface
/// can be developed and tested in an ordinary browser against real scans.
fn serve(port: u16, ui: &str) {
    let server = tiny_http::Server::http(("127.0.0.1", port)).expect("could not bind port");
    let api = Api::new();
    println!("FrisyDisk dev server on http://127.0.0.1:{port} (UI from {ui})");
    let allowed_hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let ui_root = std::fs::canonicalize(ui).expect("UI folder not found");
    for mut request in server.incoming_requests() {
        let url = request.url().split('?').next().unwrap_or("/").to_string();
        // Only this machine's own pages may talk to the server: refuse requests
        // addressed to another host name (DNS rebinding) or sent by another site.
        let header = |name: &str| {
            request.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string())
        };
        let host_ok = header("Host").is_some_and(|h| allowed_hosts.contains(&h));
        let origin_ok = header("Origin").map_or(true, |o| allowed_hosts.iter().any(|h| o == format!("http://{h}")));
        if !host_ok || !origin_ok {
            let _ = request.respond(tiny_http::Response::from_string("forbidden").with_status_code(403));
            continue;
        }
        if let Some(cmd) = url.strip_prefix("/api/") {
            let mut body = String::new();
            let _ = std::io::Read::read_to_string(request.as_reader(), &mut body);
            let args: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let (status, payload) = match api.call(cmd, &args) {
                Ok(v) => (200, serde_json::json!({ "ok": v })),
                Err(e) => (400, serde_json::json!({ "error": e })),
            };
            let header = tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap();
            let _ = request.respond(tiny_http::Response::from_string(payload.to_string()).with_status_code(status).with_header(header));
            continue;
        }
        let rel = if url == "/" { "index.html" } else { url.trim_start_matches('/') };
        // Plain relative names only, and the resolved file must sit inside the UI folder.
        let plain = std::path::Path::new(rel).components().all(|c| matches!(c, std::path::Component::Normal(_)));
        let file = plain.then(|| ui_root.join(rel)).and_then(|f| std::fs::canonicalize(f).ok()).filter(|f| f.starts_with(&ui_root) && f.is_file());
        let Some(file) = file else {
            let _ = request.respond(tiny_http::Response::from_string("not found").with_status_code(404));
            continue;
        };
        let mime = match file.extension().and_then(|e| e.to_str()) {
            Some("html") => "text/html; charset=utf-8",
            Some("js") => "text/javascript; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("svg") => "image/svg+xml",
            Some("png") => "image/png",
            _ => "application/octet-stream",
        };
        let header = tiny_http::Header::from_bytes("Content-Type", mime).unwrap();
        let _ = request.respond(tiny_http::Response::from_file(std::fs::File::open(file).unwrap()).with_header(header));
    }
}
