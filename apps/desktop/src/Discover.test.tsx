import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Discover } from "./Discover";
import type { SearchApi, SearchResult, SearchSnapshot } from "./searchBridge";
afterEach(cleanup);
const row: SearchResult = {
  info_hash: "a".repeat(40),
  name: "Authorized collection",
  magnet: "magnet:?xt=urn:btih:" + "a".repeat(40),
  size_bytes: 1024,
  seeders: 0,
  leechers: null,
  added_unix_ms: null,
  providers: ["nyaa"],
  has_unknown_health: false,
  trackers_truncated: false,
};
const done: SearchSnapshot = {
  id: 1,
  query: "collection",
  phase: "completed",
  providers: [
    {
      descriptor: { id: "nyaa", label: "Nyaa" },
      status: { state: "complete", accepted: 1, rejected: 0, truncated: false },
    },
  ],
  results: [row],
  partial: false,
  truncated: false,
};
function setup(overrides: Partial<SearchApi> = {}, snapshot = done) {
  const api: SearchApi = {
    start: vi.fn().mockResolvedValue(snapshot),
    snapshot: vi.fn().mockResolvedValue(snapshot),
    cancel: vi.fn().mockResolvedValue(true),
    ...overrides,
  };
  const choose = vi.fn();
  const view = render(
    <Discover
      api={api}
      available
      canAdd
      managedIds={new Set()}
      onChoose={choose}
    />,
  );
  return { api, choose, ...view };
}
async function submit() {
  await userEvent.type(
    screen.getByRole("textbox", { name: "Search torrents" }),
    "collection",
  );
  await userEvent.click(screen.getByRole("button", { name: "Search" }));
}
describe("Discover", () => {
  it("searches only on submit and chooses files without starting a transfer", async () => {
    const { api, choose } = setup();
    expect(api.start).not.toHaveBeenCalled();
    await submit();
    expect(api.start).toHaveBeenCalledWith("collection", null);
    await userEvent.click(
      await screen.findByRole("button", { name: "Choose files" }),
    );
    expect(choose).toHaveBeenCalledWith(row);
    expect(screen.queryByText("Unknown size", { exact: true })).toBeNull();
  });
  it("retains all category filters and explains categories without a provider", async () => {
    const { api } = setup();
    await submit();
    await userEvent.click(screen.getByRole("button", { name: "Movies" }));
    expect(api.start).toHaveBeenLastCalledWith("collection", "movies");
    expect(screen.getByText("Movies sources are coming next")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Choose files" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "TV" }));
    expect(screen.getByText("TV sources are coming next")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Games" })).toBeTruthy();
    await userEvent.click(
      screen.getByRole("button", { name: "Explore Anime" }),
    );
    expect(api.start).toHaveBeenLastCalledWith("collection", "anime");
  });
  it("preserves unknown health when hiding dead torrents and keeps the filter reversible", async () => {
    setup(
      {},
      {
        ...done,
        results: [
          row,
          {
            ...row,
            info_hash: "b".repeat(40),
            name: "Unknown health",
            seeders: null,
            has_unknown_health: true,
          },
        ],
      },
    );
    await submit();
    await userEvent.click(
      screen.getByRole("checkbox", { name: "Hide zero seeders" }),
    );
    expect(screen.queryByText(row.name)).toBeNull();
    expect(screen.getByText("Unknown health", { exact: true })).toBeTruthy();
    await userEvent.click(
      screen.getByRole("checkbox", { name: "Hide zero seeders" }),
    );
    expect(screen.getByText(row.name)).toBeTruthy();
  });
  it("shows source failure and retry without inventing results", async () => {
    const { api } = setup(
      {},
      {
        ...done,
        results: [],
        partial: true,
        providers: [
          {
            descriptor: { id: "nyaa", label: "Nyaa" },
            status: {
              state: "failed",
              failure: { code: "rate_limited", retry_after_seconds: 60 },
            },
          },
        ],
      },
    );
    await submit();
    expect(screen.getByText(/Rate limited/)).toBeTruthy();
    expect(screen.getByText("No results to show")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Search" }));
    expect(api.start).toHaveBeenCalledTimes(2);
  });
  it("stops a running search and keeps partial results", async () => {
    const { api } = setup(
      { snapshot: vi.fn().mockResolvedValue({ ...done, phase: "cancelled" }) },
      { ...done, phase: "running" },
    );
    await submit();
    await userEvent.click(screen.getByRole("button", { name: "Stop search" }));
    expect(api.cancel).toHaveBeenCalledWith(1);
    await screen.findByText("Search stopped");
    expect(screen.getByText(row.name)).toBeTruthy();
  });
  it("ignores a late response from a replaced search", async () => {
    let resolveOld!: (value: SearchSnapshot) => void;
    const snapshot = vi.fn().mockImplementationOnce(
      () =>
        new Promise<SearchSnapshot>((resolve) => {
          resolveOld = resolve;
        }),
    );
    const start = vi
      .fn()
      .mockResolvedValueOnce({ ...done, phase: "running" })
      .mockResolvedValueOnce({
        ...done,
        id: 2,
        results: [{ ...row, name: "New result" }],
      });
    setup({ start, snapshot });
    await submit();
    await waitFor(() => expect(snapshot).toHaveBeenCalledWith(1));
    await userEvent.click(screen.getByRole("button", { name: "Search" }));
    await screen.findByText("New result");
    resolveOld(done);
    await waitFor(() => expect(screen.queryByText(row.name)).toBeNull());
    expect(screen.getByText("New result")).toBeTruthy();
  });
});
