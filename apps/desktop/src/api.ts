import { invoke } from "@tauri-apps/api/core";

export interface Device {
  id: string;
  name: string;
  platform: string;
  address: string;
  port: number;
}

export interface QrPayload {
  payload: string;
  expires_at: number;
}

export interface TransferRecord {
  id: string;
  file_name: string;
  file_size: number;
  direction: "sent" | "received";
  device_name: string;
  status: "completed" | "failed" | "in_progress";
  created_at: number;
}

export interface ReceiveFolderInfo {
  path: string;
}

export const api = {
  listDevices: ()                          => invoke<Device[]>("list_devices"),
  getLocalId:  ()                          => invoke<{ device_id: string; device_name: string }>("get_local_id"),
  generateQr:  ()                          => invoke<QrPayload>("generate_qr"),
  pairWithCode:(code: string)              => invoke<void>("pair_with_code", { code }),
  sendFile:    (deviceId: string, path: string) => invoke<void>("send_file", { deviceId, path }),
  listTransfers: ()                        => invoke<TransferRecord[]>("list_transfers"),
  getReceiveFolder: ()                     => invoke<ReceiveFolderInfo>("get_receive_folder"),
  setReceiveFolder: (path: string)         => invoke<void>("set_receive_folder", { path }),
};
