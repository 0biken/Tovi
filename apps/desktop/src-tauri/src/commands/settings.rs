use super::{message, CommandResult};
use serde::Serialize;
use std::path::Path;
use tauri::State;
use tovi_core::node::Node;

#[derive(Serialize)]
pub struct ReceiveFolderInfo {
    pub path: String,
}

/// Where received files are saved
#[tauri::command]
pub fn get_receive_folder(node: State<'_, Node>) -> ReceiveFolderInfo {
    ReceiveFolderInfo {
        path: node.receive_dir().to_string_lossy().into_owned(),
    }
}

/// Save a new receive folder; applies to the next transfer
#[tauri::command]
pub fn set_receive_folder(node: State<'_, Node>, path: String) -> CommandResult<()> {
    node.set_receive_dir(Path::new(&path)).map_err(message)
}

/// Whether files from paired devices are saved without asking
#[tauri::command]
pub fn get_auto_accept(node: State<'_, Node>) -> bool {
    node.auto_accept()
}

#[tauri::command]
pub fn set_auto_accept(node: State<'_, Node>, enabled: bool) -> CommandResult<()> {
    node.set_auto_accept(enabled).map_err(message)
}
