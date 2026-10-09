import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

// ---------------------------------------------------------------- commands

/** A paired device */
export interface Device {
  id: string;
  name: string;
  platform: string;
  /** Unix seconds */
  paired_at: number;
  last_seen: number | null;
  /** Best known address, e.g. "192.168.1.20:48210" */
  address: string | null;
}

export interface QrPayload {
  /** The `tovi://pair/...` link */
  payload: string;
  /** Unix seconds */
  expires_at: number;
}

export type Direction = "sent" | "received";

export interface TransferRecord {
  id: string;
  file_name: string;
  file_size: number;
  direction: Direction;
  device_id: string;
  device_name: string;
  status: "completed" | "failed" | "in_progress";
  /** Unix seconds */
  created_at: number;
  finished_at: number | null;
  /** Source file (sent) or saved file (received) */
  path: string | null;
  error: string | null;
}

export interface ReceiveFolderInfo {
  path: string;
}

export interface SendResult {
  transfer_id: string;
  size: number;
}

export const api = {
  getLocalId: () => invoke<{ device_id: string; device_name: string }>("get_local_id"),
  listDevices: () => invoke<Device[]>("list_devices"),
  forgetDevice: (deviceId: string) => invoke<boolean>("forget_device", { deviceId }),
  generateQr: () => invoke<QrPayload>("generate_qr"),
  pairWithCode: (code: string) => invoke<Device>("pair_with_code", { code }),
  /** Answer a pairing request or an incoming file */
  respond: (requestId: number, allow: boolean) => invoke<boolean>("respond", { requestId, allow }),
  sendFile: (deviceId: string, path: string) => invoke<SendResult>("send_file", { deviceId, path }),
  listTransfers: (limit?: number) => invoke<TransferRecord[]>("list_transfers", { limit }),
  openTransferFile: (transferId: string) => invoke<void>("open_transfer_file", { transferId }),
  revealTransferFile: (transferId: string) =>
    invoke<void>("reveal_transfer_file", { transferId }),
  getReceiveFolder: () => invoke<ReceiveFolderInfo>("get_receive_folder"),
  setReceiveFolder: (path: string) => invoke<void>("set_receive_folder", { path }),
  getAutoAccept: () => invoke<boolean>("get_auto_accept"),
  setAutoAccept: (enabled: boolean) => invoke<void>("set_auto_accept", { enabled }),
};

// ------------------------------------------------------------------ events

export interface PairingRequestEvent {
  request_id: number;
  device_id: string;
  device_name: string;
  platform: string;
}

export interface PairedEvent {
  device_id: string;
  device_name: string;
  platform: string;
}

export interface PairingFailedEvent {
  device_id: string;
  short_id: string;
  error: string;
}

export interface IncomingEvent {
  request_id: number;
  transfer_id: string;
  device_name: string;
  file_name: string;
  file_size: number;
}

export interface StartedEvent {
  transfer_id: string;
  direction: Direction;
  device_id: string;
  device_name: string;
  file_name: string;
  file_size: number;
  resuming: boolean;
}

export interface ProgressEvent {
  transfer_id: string;
  direction: Direction;
  done: number;
  total: number;
  bytes_per_sec: number;
}

export interface ReconnectingEvent {
  transfer_id: string;
  error: string;
}

export interface FinishedEvent {
  transfer_id: string;
  direction: Direction;
  device_id: string;
  device_name: string;
  file_name: string;
  status: "completed" | "failed" | "interrupted";
  path: string | null;
  size: number | null;
  resumed_bytes: number | null;
  error: string | null;
}

interface EventMap {
  "pairing:request": PairingRequestEvent;
  "pairing:done": PairedEvent;
  "pairing:failed": PairingFailedEvent;
  "request:expired": { request_id: number };
  "transfer:incoming": IncomingEvent;
  "transfer:started": StartedEvent;
  "transfer:progress": ProgressEvent;
  "transfer:reconnecting": ReconnectingEvent;
  "transfer:finished": FinishedEvent;
  "devices:changed": null;
}

/** Subscribe to a backend event; returns the unsubscribe function */
export function on<K extends keyof EventMap>(
  name: K,
  handler: (payload: EventMap[K]) => void,
): Promise<UnlistenFn> {
  return listen<EventMap[K]>(name, (event) => handler(event.payload));
}

/**
 * React-friendly subscription: listens until the returned cleanup runs, even
 * if that happens before `listen` has resolved
 */
export function subscribe<K extends keyof EventMap>(
  name: K,
  handler: (payload: EventMap[K]) => void,
): () => void {
  let unlisten: UnlistenFn | null = null;
  let cancelled = false;
  on(name, handler).then((fn) => {
    if (cancelled) fn();
    else unlisten = fn;
  });
  return () => {
    cancelled = true;
    unlisten?.();
  };
}
