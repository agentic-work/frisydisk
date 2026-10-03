// No console window behind the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use frisy_core::api::Api;
use serde_json::Value;

/// The whole interface: the UI sends a command name and JSON arguments.
/// Runs off the main thread so a long call never freezes the window.
#[tauri::command(async)]
fn api(state: tauri::State<'_, Api>, cmd: String, args: Value) -> Result<Value, String> {
    state.call(&cmd, &args)
}

/// Windows only lets administrators read some system folders. Start a new,
/// elevated copy of the app on the same folder (Windows asks for consent),
/// then close this one.
#[tauri::command]
fn relaunch_as_admin(app: tauri::AppHandle, path: String) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Only needed on Windows.".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let mut script = format!("Start-Process -FilePath {} -Verb RunAs", quote(&exe.to_string_lossy()));
    if !path.is_empty() {
        script.push_str(&format!(" -ArgumentList {}", quote(&format!("--scan \"{path}\""))));
    }
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Windows did not start FrisyDisk as administrator.".into());
    }
    app.exit(0);
    Ok(())
}

/// Lets scripted runs pass `--scan PATH --mode sankey --tab types`.
#[tauri::command]
fn launch_args() -> Vec<String> {
    std::env::args().skip(1).collect()
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Api::new())
        .invoke_handler(tauri::generate_handler![api, launch_args, relaunch_as_admin])
        .run(tauri::generate_context!())
        .expect("FrisyDisk could not start");
}
