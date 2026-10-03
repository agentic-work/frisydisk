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
/// elevated copy of the app (Windows asks for consent) on the folder that is
/// open now, then close this one. The folder comes from the app's own state,
/// and no shell or script interpreter is involved.
#[tauri::command]
fn relaunch_as_admin(app: tauri::AppHandle, state: tauri::State<'_, Api>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut params = String::new();
    if let Some(root) = state.scan_root() {
        if !std::path::Path::new(&root).is_dir() || root.contains('"') {
            return Err("The open folder cannot be reopened.".into());
        }
        params = format!("--scan {}", quote_arg(&root));
    }
    run_elevated(&exe, &params)?;
    app.exit(0);
    Ok(())
}

/// Quote one argument for the Windows command line: a trailing backslash would
/// otherwise escape the closing quote.
fn quote_arg(s: &str) -> String {
    let trailing = s.len() - s.trim_end_matches('\\').len();
    format!("\"{}{}\"", s, "\\".repeat(trailing))
}

#[cfg(windows)]
fn run_elevated(exe: &std::path::Path, params: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let verb = wide("runas".as_ref());
    let file = wide(exe.as_os_str());
    let args = wide(params.as_ref());
    let result = unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), args.as_ptr(), std::ptr::null(), SW_SHOWNORMAL)
    };
    // ShellExecute reports success with a value above 32.
    if result as isize > 32 {
        Ok(())
    } else {
        Err("Windows did not start FrisyDisk as administrator.".into())
    }
}

#[cfg(not(windows))]
fn run_elevated(_: &std::path::Path, _: &str) -> Result<(), String> {
    Err("Only needed on Windows.".into())
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
