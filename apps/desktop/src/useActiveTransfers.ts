import { useMemo, useSyncExternalStore } from "react";
import { Direction, on } from "./api";

export interface ActiveTransfer {
  id: string;
  direction: Direction;
  deviceName: string;
  fileName: string;
  done: number;
  total: number;
  /** Bytes per second; 0 until the first progress event */
  speed: number;
  reconnecting: boolean;
}

// One app-wide list, fed by backend events from startup, so a page opened
// mid-transfer still sees what is already in flight
let active: ActiveTransfer[] = [];
const listeners = new Set<() => void>();

function update(f: (all: ActiveTransfer[]) => ActiveTransfer[]) {
  active = f(active);
  listeners.forEach((l) => l());
}

function patch(id: string, changes: Partial<ActiveTransfer>) {
  update((all) => all.map((t) => (t.id === id ? { ...t, ...changes } : t)));
}

on("transfer:started", (e) =>
  update((all) => [
    ...all.filter((t) => t.id !== e.transfer_id),
    {
      id: e.transfer_id,
      direction: e.direction,
      deviceName: e.device_name,
      fileName: e.file_name,
      done: 0,
      total: e.file_size,
      speed: 0,
      reconnecting: false,
    },
  ]),
);
on("transfer:progress", (e) =>
  patch(e.transfer_id, { done: e.done, total: e.total, speed: e.bytes_per_sec, reconnecting: false }),
);
on("transfer:reconnecting", (e) => patch(e.transfer_id, { reconnecting: true }));
on("transfer:finished", (e) => update((all) => all.filter((t) => t.id !== e.transfer_id)));

function subscribeStore(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Every transfer in flight, in either direction */
export function useActiveTransfers(): ActiveTransfer[] {
  return useSyncExternalStore(subscribeStore, () => active);
}

/**
 * IDs of the transfers in flight. Changes only when one starts or finishes,
 * so callers don't re-render on every progress event.
 */
export function useActiveTransferIds(): Set<string> {
  const key = useSyncExternalStore(subscribeStore, () => active.map((t) => t.id).join(","));
  return useMemo(() => new Set(key ? key.split(",") : []), [key]);
}
