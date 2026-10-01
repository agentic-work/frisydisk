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

/// Lets scripted runs pass `--scan PATH --mode sankey --tab types`.
#[tauri::command]
fn launch_args() -> Vec<String> {
    std::env::args().skip(1).collect()
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Api::new())
        .invoke_handler(tauri::generate_handler![api, launch_args])
        .run(tauri::generate_context!())
        .expect("FrisyDisk could not start");
}
