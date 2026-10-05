mod commands;

use commands::{discovery, pairing, settings, transfer};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            // Discovery
            discovery::list_devices,
            // Pairing
            pairing::get_local_id,
            pairing::generate_qr,
            pairing::pair_with_code,
            // Transfer
            transfer::send_file,
            transfer::list_transfers,
            // Settings
            settings::get_receive_folder,
            settings::set_receive_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TOVI desktop application");
}
