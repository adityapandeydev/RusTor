import { invoke } from "@tauri-apps/api/core";

export type SearchCategory = "movies" | "tv" | "games" | "anime";
export interface SearchResult {
  info_hash: string;
  name: string;
  magnet: string;
  size_bytes: number | null;
  seeders: number | null;
  leechers: number | null;
  added_unix_ms: number | null;
  providers: string[];
  has_unknown_health: boolean;
  trackers_truncated: boolean;
}
export type ProviderStatus =
  | {
      state:
        | "pending"
        | "running"
        | "timed_out"
        | "deadline_exceeded"
        | "cancelled";
    }
  | {
      state: "complete";
      accepted: number;
      rejected: number;
      truncated: boolean;
    }
  | {
      state: "failed";
      failure: {
        code: "unavailable" | "invalid_response" | "panicked" | "rate_limited";
        retry_after_seconds?: number | null;
      };
    };
export interface SearchSnapshot {
  id: number;
  query: string;
  phase: "running" | "completed" | "cancelled" | "timed_out";
  providers: {
    descriptor: { id: string; label: string };
    status: ProviderStatus;
  }[];
  results: SearchResult[];
  partial: boolean;
  truncated: boolean;
}
export interface SearchApi {
  start(
    query: string,
    category: SearchCategory | null,
  ): Promise<SearchSnapshot>;
  snapshot(id: number): Promise<SearchSnapshot>;
  cancel(id: number): Promise<boolean>;
}
export const desktopSearch: SearchApi = {
  start: (query, category) => invoke("start_search", { query, category }),
  snapshot: (id) => invoke("get_search_snapshot", { id }),
  cancel: (id) => invoke("cancel_search", { id }),
};
