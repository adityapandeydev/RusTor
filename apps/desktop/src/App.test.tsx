import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "./App";
import type { DesktopApi, Snapshot, Torrent } from "./bridge";

afterEach(() => {
  cleanup();
  localStorage.clear();
});
const initial: Snapshot = {
  revision: 1,
  phase: "ready",
  torrents: [],
  default_destination: "C:/Downloads/RusTor",
  settings: {
    default_destination: "C:/Downloads/RusTor",
    download_limit_kib: 0,
    upload_limit_kib: 0,
  },
  error: null,
};
function api(snapshot = initial) {
  let receive: (snapshot: Snapshot) => void = () => {};
  const unsubscribe = vi.fn();
  const service: DesktopApi = {
    available: true,
    snapshot: vi.fn().mockResolvedValue(snapshot),
    subscribe: vi.fn(async (callback) => {
      receive = callback;
      return unsubscribe;
    }),
    pickTorrent: vi.fn().mockResolvedValue(null),
    pickDestination: vi.fn().mockResolvedValue(null),
    prepare: vi.fn().mockResolvedValue({
      id: "prepared-1",
      name: "Authorized collection",
      files: [
        { index: 0, path: "wanted.bin", length: 1024, selected: true },
        { index: 1, path: "extra.bin", length: 2048, selected: true },
      ],
    }),
    discardPrepared: vi.fn().mockResolvedValue(undefined),
    add: vi
      .fn()
      .mockResolvedValue({ id: "torrent-id", already_managed: false }),
    pause: vi.fn().mockResolvedValue(undefined),
    resume: vi.fn().mockResolvedValue(undefined),
    remove: vi.fn().mockResolvedValue(undefined),
    retry: vi.fn().mockResolvedValue(undefined),
    saveSettings: vi.fn().mockResolvedValue({ ...initial, revision: 2 }),
  };
  return { service, emit: (value: Snapshot) => receive(value), unsubscribe };
}
const torrent: Torrent = {
  id: "abc",
  name: "Authorized fixture.bin",
  destination: "C:/Downloads/RusTor",
  state: "downloading",
  downloaded_bytes: 1024,
  total_bytes: 4096,
  uploaded_bytes: 0,
  download_bytes_per_second: 128,
  upload_bytes_per_second: 0,
  peers: 1,
  error: null,
};

describe("Downloads desktop flow", () => {
  it("adds a magnet and the selected native destination", async () => {
    const { service } = api();
    vi.mocked(service.pickDestination).mockResolvedValue({
      id: "folder-1",
      label: "D:/Selected",
    });
    render(<App api={service} />);
    await userEvent.click(
      await screen.findByRole("button", { name: "Add torrent" }),
    );
    await userEvent.type(
      screen.getByLabelText("Magnet link or infohash"),
      "magnet:?xt=urn:btih:test",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Choose folder" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Load file list" }),
    );
    await screen.findByRole("checkbox", { name: "extra.bin" });
    expect(service.prepare).toHaveBeenCalledWith({
      kind: "magnet",
      value: "magnet:?xt=urn:btih:test",
    });
    expect(service.add).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("checkbox", { name: "extra.bin" }));
    await userEvent.click(
      screen.getByRole("button", { name: "Start download" }),
    );
    await waitFor(() =>
      expect(service.add).toHaveBeenCalledWith("prepared-1", [0], "folder-1"),
    );
  });

  it("passes the native file selection ID and preserves the form after an error", async () => {
    const { service } = api();
    vi.mocked(service.pickTorrent).mockResolvedValue({
      id: "file-1",
      label: "authorized.torrent",
    });
    vi.mocked(service.add).mockRejectedValue({
      code: "invalid_input",
      message: "Invalid torrent metadata.",
    });
    render(<App api={service} />);
    await userEvent.click(
      await screen.findByRole("button", { name: "Add torrent" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Choose .torrent file" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Load file list" }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: "Start download" }),
    );
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "Invalid torrent metadata.",
    );
    expect(service.prepare).toHaveBeenCalledWith({
      kind: "file",
      id: "file-1",
    });
    expect(service.add).toHaveBeenCalledWith("prepared-1", [0, 1], null);
    expect(screen.getByText("authorized.torrent")).toBeDefined();
  });

  it("pauses a real row and unsubscribes on unmount", async () => {
    const { service, unsubscribe } = api({ ...initial, torrents: [torrent] });
    const view = render(<App api={service} />);
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Pause Authorized fixture.bin",
      }),
    );
    expect(service.pause).toHaveBeenCalledWith("abc");
    view.unmount();
    expect(unsubscribe).toHaveBeenCalledTimes(1);
  });

  it("does not overwrite newer live state with a stale snapshot", async () => {
    const { service, emit } = api({
      ...initial,
      revision: 4,
      torrents: [torrent],
    });
    render(<App api={service} />);
    await screen.findByText("Authorized fixture.bin");
    emit({ ...initial, revision: 2 });
    await waitFor(() =>
      expect(screen.getByText("Authorized fixture.bin")).toBeDefined(),
    );
  });

  it("shows an honest browser-only state without invoking native commands", () => {
    const { service } = api();
    service.available = false;
    render(<App api={service} />);
    expect(screen.getByText(/Open RusTor as a desktop app/)).toBeDefined();
    expect(service.snapshot).not.toHaveBeenCalled();
  });
});

it("removes a seeder from the session and explains that files are kept", async () => {
  const { service } = api({
    ...initial,
    torrents: [{ ...torrent, state: "seeding" }],
  });
  render(<App api={service} />);
  const remove = await screen.findByRole("button", {
    name: "Stop and remove Authorized fixture.bin (keep files)",
  });
  vi.mocked(service.snapshot).mockResolvedValue({ ...initial, revision: 2 });
  await userEvent.click(remove);
  await waitFor(() =>
    expect(screen.queryByText("Authorized fixture.bin")).toBeNull(),
  );
  expect(service.remove).toHaveBeenCalledWith("abc");
  expect(screen.getByRole("status").textContent).toContain("Files kept");
});

it("keeps a failed removal visible and allows retry", async () => {
  const { service } = api({ ...initial, torrents: [torrent] });
  vi.mocked(service.remove).mockRejectedValue({
    message: "Could not remove torrent. Try again.",
  });
  render(<App api={service} />);
  await userEvent.click(
    await screen.findByRole("button", { name: /Stop and remove Authorized/ }),
  );
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Could not remove",
  );
  expect(screen.getByText("Authorized fixture.bin")).toBeDefined();
});

it("restores a saved light theme across remounts", async () => {
  const { service } = api();
  const view = render(<App api={service} />);
  expect(document.documentElement.dataset.theme).toBe("dark");
  await userEvent.click(
    screen.getByRole("button", { name: "Switch to light mode" }),
  );
  expect(document.documentElement.dataset.theme).toBe("light");
  view.unmount();
  render(<App api={service} />);
  expect(document.documentElement.dataset.theme).toBe("light");
  await userEvent.click(
    screen.getByRole("button", { name: "Switch to dark mode" }),
  );
  expect(localStorage.getItem("rustor-theme")).toBe("dark");
});

it("requires a selected file and discards a preview when the source changes", async () => {
  const { service } = api();
  render(<App api={service} />);
  await userEvent.click(
    await screen.findByRole("button", { name: "Add torrent" }),
  );
  await userEvent.type(
    screen.getByLabelText("Magnet link or infohash"),
    "magnet:?xt=urn:btih:test",
  );
  await userEvent.click(screen.getByRole("button", { name: "Load file list" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "Clear selection" }),
  );
  expect(screen.getByRole("button", { name: "Start download" })).toHaveProperty(
    "disabled",
    true,
  );
  expect(service.add).not.toHaveBeenCalled();
  await userEvent.click(
    screen.getByRole("button", { name: "Select all files" }),
  );
  expect(screen.getByRole("button", { name: "Start download" })).toHaveProperty(
    "disabled",
    false,
  );
  await userEvent.type(screen.getByLabelText("Magnet link or infohash"), "x");
  await waitFor(() =>
    expect(service.discardPrepared).toHaveBeenCalledWith("prepared-1"),
  );
  expect(screen.queryByRole("checkbox")).toBeNull();
});

it("shows metadata failures without starting a torrent", async () => {
  const { service } = api();
  vi.mocked(service.prepare).mockRejectedValue({
    message: "Metadata could not be found. Try again.",
  });
  render(<App api={service} />);
  await userEvent.click(
    await screen.findByRole("button", { name: "Add torrent" }),
  );
  await userEvent.type(
    screen.getByLabelText("Magnet link or infohash"),
    "magnet:?xt=urn:btih:test",
  );
  await userEvent.click(screen.getByRole("button", { name: "Load file list" }));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Metadata could not be found",
  );
  expect(service.add).not.toHaveBeenCalled();
  expect(screen.getByLabelText("Magnet link or infohash")).toHaveProperty(
    "value",
    "magnet:?xt=urn:btih:test",
  );
});

it("allows an expired file list to be reloaded without re-entering the source", async () => {
  const { service } = api();
  vi.mocked(service.add).mockRejectedValue({
    code: "selection_expired",
    message: "This file list has expired. Load the files again.",
  });
  render(<App api={service} />);
  await userEvent.click(
    await screen.findByRole("button", { name: "Add torrent" }),
  );
  await userEvent.type(
    screen.getByLabelText("Magnet link or infohash"),
    "magnet:?xt=urn:btih:test",
  );
  await userEvent.click(screen.getByRole("button", { name: "Load file list" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "Start download" }),
  );
  await screen.findByRole("alert");
  expect(screen.getByRole("button", { name: "Load file list" })).toHaveProperty(
    "disabled",
    false,
  );
  expect(screen.getByLabelText("Magnet link or infohash")).toHaveProperty(
    "value",
    "magnet:?xt=urn:btih:test",
  );
});

it("shows failure guidance without expanding the row and retries the same torrent", async () => {
  const { service } = api({
    ...initial,
    torrents: [
      {
        ...torrent,
        state: "failed",
        error: "Disk write failed: access denied",
      },
    ],
  });
  render(<App api={service} />);
  expect(
    await screen.findByText(/Check that the download folder/),
  ).toBeDefined();
  expect(screen.getByText("Disk write failed: access denied")).toBeDefined();
  expect(
    screen.queryByRole("button", { name: "Pause Authorized fixture.bin" }),
  ).toBeNull();
  vi.mocked(service.snapshot).mockResolvedValue({
    ...initial,
    revision: 2,
    torrents: [{ ...torrent, state: "checking" }],
  });
  await userEvent.click(
    screen.getByRole("button", { name: "Retry Authorized fixture.bin" }),
  );
  expect(service.retry).toHaveBeenCalledWith("abc");
  expect(service.remove).not.toHaveBeenCalled();
  await screen.findByText("Checking files");
  expect(screen.queryByText(/Check that the download folder/)).toBeNull();
  expect(screen.getByRole("status").textContent).toContain("Retry started");
});

it("keeps failed transfers visible when retry fails and unlocks the retry button", async () => {
  const { service } = api({
    ...initial,
    torrents: [{ ...torrent, state: "failed", error: null }],
  });
  vi.mocked(service.retry).mockRejectedValue({
    message: "Could not retry. Check the folder and try again.",
  });
  render(<App api={service} />);
  await userEvent.click(
    await screen.findByRole("button", { name: "Retry Authorized fixture.bin" }),
  );
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Could not retry",
  );
  expect(
    screen.getByRole("button", { name: "Retry Authorized fixture.bin" }),
  ).toHaveProperty("disabled", false);
  expect(screen.getByText("Authorized fixture.bin")).toBeDefined();
  expect(screen.queryByText("Retry started")).toBeNull();
});

it("saves native folder handles and session limits from the download settings panel", async () => {
  const { service } = api();
  vi.mocked(service.pickDestination).mockResolvedValue({
    id: "folder-settings",
    label: "D:/Saved downloads",
  });
  vi.mocked(service.saveSettings).mockResolvedValue({
    ...initial,
    revision: 2,
    default_destination: "D:/Saved downloads",
    settings: {
      default_destination: "D:/Saved downloads",
      download_limit_kib: 128,
      upload_limit_kib: 64,
    },
  });
  render(<App api={service} />);
  await userEvent.click(
    await screen.findByRole("button", { name: "Download settings" }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Choose default folder" }),
  );
  await userEvent.clear(screen.getByLabelText("Download limit (KiB/s)"));
  await userEvent.type(screen.getByLabelText("Download limit (KiB/s)"), "128");
  await userEvent.clear(screen.getByLabelText("Upload limit (KiB/s)"));
  await userEvent.type(screen.getByLabelText("Upload limit (KiB/s)"), "64");
  await userEvent.click(screen.getByRole("button", { name: "Save settings" }));
  await waitFor(() =>
    expect(service.saveSettings).toHaveBeenCalledWith({
      destination_id: "folder-settings",
      download_limit_kib: 128,
      upload_limit_kib: 64,
    }),
  );
  expect(screen.getByRole("status").textContent).toContain("Settings saved");
  expect(screen.queryByRole("form", { name: "Download settings" })).toBeNull();
  await userEvent.click(
    screen.getByRole("button", { name: "Download settings" }),
  );
  expect(screen.getByLabelText("Download limit (KiB/s)")).toHaveProperty(
    "value",
    "128",
  );
});

it("rejects limits below one chunk and cancels settings without saving", async () => {
  const { service } = api();
  render(<App api={service} />);
  await userEvent.click(
    await screen.findByRole("button", { name: "Download settings" }),
  );
  await userEvent.clear(screen.getByLabelText("Download limit (KiB/s)"));
  await userEvent.type(screen.getByLabelText("Download limit (KiB/s)"), "1");
  await userEvent.click(screen.getByRole("button", { name: "Save settings" }));
  expect((await screen.findByRole("alert")).textContent).toContain("16");
  expect(service.saveSettings).not.toHaveBeenCalled();
  await userEvent.click(
    screen.getByRole("button", { name: "Cancel settings" }),
  );
  expect(screen.queryByRole("form", { name: "Download settings" })).toBeNull();
});

it("preserves settings edits after a save error without claiming success", async () => {
  const { service } = api();
  vi.mocked(service.saveSettings).mockRejectedValue({
    message: "Settings could not be saved. Try again.",
  });
  render(<App api={service} />);
  await userEvent.click(
    await screen.findByRole("button", { name: "Download settings" }),
  );
  await userEvent.clear(screen.getByLabelText("Upload limit (KiB/s)"));
  await userEvent.type(screen.getByLabelText("Upload limit (KiB/s)"), "64");
  await userEvent.click(screen.getByRole("button", { name: "Save settings" }));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Settings could not be saved",
  );
  expect(screen.getByLabelText("Upload limit (KiB/s)")).toHaveProperty(
    "value",
    "64",
  );
  expect(screen.queryByRole("status")).toBeNull();
});

it("opens Discover results in the existing file-selection flow without starting a transfer", async () => {
  const { service } = api();
  const result = {
    info_hash: "a".repeat(40),
    name: "Authorized search fixture",
    magnet: "magnet:?xt=urn:btih:" + "a".repeat(40),
    size_bytes: 1024,
    seeders: 2,
    leechers: 0,
    added_unix_ms: null,
    providers: ["nyaa"],
    has_unknown_health: false,
    trackers_truncated: false,
  };
  const searchApi = {
    start: vi.fn().mockResolvedValue({
      id: 1,
      query: "fixture",
      phase: "completed",
      providers: [],
      results: [result],
      partial: false,
      truncated: false,
    }),
    snapshot: vi.fn(),
    cancel: vi.fn().mockResolvedValue(true),
  };
  render(<App api={service} searchApi={searchApi} />);
  await userEvent.click(screen.getByRole("button", { name: "Discover" }));
  await userEvent.type(
    screen.getByRole("textbox", { name: "Search torrents" }),
    "fixture",
  );
  await userEvent.click(screen.getByRole("button", { name: "Search" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "Choose files" }),
  );
  expect(
    (screen.getByLabelText("Magnet link or infohash") as HTMLInputElement)
      .value,
  ).toBe(result.magnet);
  expect(service.prepare).not.toHaveBeenCalled();
  expect(service.add).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Load file list" }));
  expect(service.prepare).toHaveBeenCalledWith({
    kind: "magnet",
    value: result.magnet,
  });
  expect(
    await screen.findByRole("button", { name: "Start download" }),
  ).toBeTruthy();
});

it("opens settings from Discover and allows closing, navigating and reopening", async () => {
  const { service } = api();
  render(<App api={service} />);
  await userEvent.click(screen.getByRole("button", { name: "Discover" }));
  const settings = await screen.findByRole("button", {
    name: "Download settings",
  });
  await waitFor(() => expect(settings.hasAttribute("disabled")).toBe(false));
  await userEvent.click(settings);
  expect(screen.getByRole("form", { name: "Download settings" })).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: /All transfers/ }));
  expect(screen.getByRole("form", { name: "Download settings" })).toBeTruthy();
  await userEvent.click(
    screen.getByRole("button", { name: "Close download settings" }),
  );
  expect(screen.queryByRole("form", { name: "Download settings" })).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "Discover" }));
  await userEvent.click(settings);
  expect(screen.getByRole("form", { name: "Download settings" })).toBeTruthy();
});
