mod commands;
mod events;

use tauri::{Manager, RunEvent};
use tovi_core::node::{Node, NodeConfig};
use tovi_core::protocol::MAX_DEVICE_NAME_LEN;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // The app keeps its own identity and database, separate from the
            // CLI's, so the two on one PC are two different devices.
            // TOVI_DATA_DIR overrides it (testing, or two instances on one PC).
            let data_dir = match std::env::var_os("TOVI_DATA_DIR") {
                Some(dir) => dir.into(),
                None => app.path().app_data_dir()?,
            };
            let receive_dir = app
                .path()
                .download_dir()
                .map(|dir| dir.join("TOVI"))
                .unwrap_or_else(|_| data_dir.join("Received"));
            let config = NodeConfig::new(data_dir, device_name(), receive_dir);
            let (node, events) = tauri::async_runtime::block_on(Node::start(config))?;
            tauri::async_runtime::spawn(events::forward(app.handle().clone(), events));
            app.manage(node);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Devices
            commands::devices::list_devices,
            commands::devices::forget_device,
            // Pairing
            commands::pairing::get_local_id,
            commands::pairing::generate_qr,
            commands::pairing::pair_with_code,
            commands::pairing::respond,
            // Transfer
            commands::transfer::send_file,
            commands::transfer::list_transfers,
            commands::transfer::open_transfer_file,
            commands::transfer::reveal_transfer_file,
            // Settings
            commands::settings::get_receive_folder,
            commands::settings::set_receive_folder,
            commands::settings::get_auto_accept,
            commands::settings::set_auto_accept,
        ])
        .build(tauri::generate_context!())
        .expect("error while building TOVI desktop application");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            // Tell connected devices we're going, and close the database cleanly
            let node = app.state::<Node>().inner().clone();
            tauri::async_runtime::block_on(node.shutdown());
        }
    });
}

/// This computer's name, as other devices will see it
fn device_name() -> String {
    hostname::get()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "TOVI Desktop".into())
        .chars()
        .take(MAX_DEVICE_NAME_LEN)
        .collect()
}
