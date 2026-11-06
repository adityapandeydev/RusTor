import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type TorrentState =
  "checking" | "downloading" | "paused" | "seeding" | "failed";
export interface Torrent {
  id: string;
  name: string | null;
  destination: string;
  state: TorrentState;
  downloaded_bytes: number;
  total_bytes: number;
  uploaded_bytes: number;
  download_bytes_per_second: number;
  upload_bytes_per_second: number;
  peers: number;
  error: string | null;
}
export interface CommandError {
  code: string;
  message: string;
}
export interface DownloadSettings {
  default_destination: string;
  download_limit_kib: number;
  upload_limit_kib: number;
  continue_in_background: boolean;
  start_at_login: boolean;
  start_minimized: boolean;
}
export interface SettingsUpdate {
  destination_id: string | null;
  download_limit_kib: number;
  upload_limit_kib: number;
  continue_in_background: boolean;
  start_at_login: boolean;
  start_minimized: boolean;
}
export interface Snapshot {
  revision: number;
  phase: "starting" | "ready" | "failed";
  torrents: Torrent[];
  default_destination: string;
  settings: DownloadSettings;
  error: CommandError | null;
  tray_available: boolean;
  background_error: CommandError | null;
  startup: { enabled: boolean | null; error: CommandError | null };
}
export interface Selection {
  id: string;
  label: string;
}
export interface UpdateStatus {
  revision: number;
  phase: "unavailable" | "idle" | "checking" | "current" | "available" | "downloading" | "ready" | "installing" | "failed";
  version: string | null;
  notes: string | null;
  downloaded: number;
  total: number | null;
  error: CommandError | null;
  retry: "check" | "download" | "install" | null;
  transfers_stopped: boolean;
}
export type AddSource =
  { kind: "magnet"; value: string } | { kind: "file"; id: string };
export interface TorrentFile {
  index: number;
  path: string;
  length: number;
  selected: boolean;
}
export interface PreparedSelection {
  id: string;
  name: string | null;
  files: TorrentFile[];
}
export interface DesktopApi {
  available: boolean;
  snapshot(): Promise<Snapshot>;
  subscribe(callback: (snapshot: Snapshot) => void): Promise<() => void>;
  pickTorrent(): Promise<Selection | null>;
  pickDestination(): Promise<Selection | null>;
  prepare(source: AddSource): Promise<PreparedSelection>;
  discardPrepared(id: string): Promise<void>;
  add(
    preparedId: string,
    files: number[],
    destinationId: string | null,
  ): Promise<{ id: string; already_managed: boolean }>;
  pause(id: string): Promise<void>;
  resume(id: string): Promise<void>;
  remove(id: string): Promise<void>;
  retry(id: string): Promise<void>;
  saveSettings(update: SettingsUpdate): Promise<Snapshot>;
  quit(): Promise<void>;
  updateStatus(): Promise<UpdateStatus>;
  subscribeUpdates(callback: (status: UpdateStatus) => void): Promise<() => void>;
  checkUpdate(): Promise<UpdateStatus>;
  downloadUpdate(): Promise<UpdateStatus>;
  installUpdate(): Promise<void>;
}

export const desktop: DesktopApi = {
  available: isTauri(),
  snapshot: () => invoke("get_snapshot"),
  subscribe: (callback) =>
    listen<Snapshot>("torrent-snapshot", (event) => callback(event.payload)),
  pickTorrent: () => invoke("pick_torrent"),
  pickDestination: () => invoke("pick_destination"),
  prepare: (source) => invoke("prepare_torrent", { source }),
  discardPrepared: (id) => invoke("discard_prepared", { id }),
  add: (preparedId, files, destinationId) =>
    invoke("start_prepared", {
      request: {
        prepared_id: preparedId,
        files,
        destination_id: destinationId,
      },
    }),
  pause: (id) => invoke("pause_torrent", { id }),
  resume: (id) => invoke("resume_torrent", { id }),
  remove: (id) => invoke("remove_torrent", { id }),
  retry: (id) => invoke("retry_torrent", { id }),
  saveSettings: (request) => invoke("save_settings", { request }),
  quit: () => invoke("quit_app"),
  updateStatus: () => invoke("get_update_status"),
  subscribeUpdates: (callback) => listen<UpdateStatus>("update-status", (event) => callback(event.payload)),
  checkUpdate: () => invoke("check_update"),
  downloadUpdate: () => invoke("download_update"),
  installUpdate: () => invoke("install_update"),
};

export function errorMessage(error: unknown): string {
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  )
    return error.message;
  return "The operation could not be completed. Please try again.";
}
