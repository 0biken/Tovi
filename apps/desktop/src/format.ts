export const PLATFORM_ICONS: Record<string, string> = {
  macos: "🍎",
  windows: "🪟",
  linux: "🐧",
  android: "🤖",
  ios: "📱",
};

export function platformIcon(platform: string) {
  return PLATFORM_ICONS[platform] ?? "💻";
}

export function formatBytes(bytes: number) {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(1)} MB`;
  if (bytes >= 1e3) return `${(bytes / 1e3).toFixed(0)} KB`;
  return `${bytes} B`;
}

export function formatSpeed(bytesPerSec: number) {
  return `${formatBytes(bytesPerSec)}/s`;
}

/** "45 s", "3 min", "1 h 05 min" */
export function formatDuration(seconds: number) {
  const s = Math.max(1, Math.round(seconds));
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.round(s / 60)} min`;
  const h = Math.floor(s / 3600);
  return `${h} h ${String(Math.round((s % 3600) / 60)).padStart(2, "0")} min`;
}

/** "just now", "5 min ago", "3 h ago", "2 days ago" */
export function timeAgo(unixSecs: number) {
  const s = Math.max(0, Date.now() / 1000 - unixSecs);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  const days = Math.floor(s / 86400);
  return `${days} day${days === 1 ? "" : "s"} ago`;
}

/** First 8 characters of a device ID, as shown elsewhere in TOVI */
export function shortId(id: string) {
  return id.slice(0, 8);
}
