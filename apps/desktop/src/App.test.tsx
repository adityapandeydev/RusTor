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
    add: vi
      .fn()
      .mockResolvedValue({ id: "torrent-id", already_managed: false }),
    pause: vi.fn().mockResolvedValue(undefined),
    resume: vi.fn().mockResolvedValue(undefined),
    remove: vi.fn().mockResolvedValue(undefined),
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
      screen.getByRole("button", { name: "Start download" }),
    );
    await waitFor(() =>
      expect(service.add).toHaveBeenCalledWith(
        { kind: "magnet", value: "magnet:?xt=urn:btih:test" },
        "folder-1",
      ),
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
      screen.getByRole("button", { name: "Start download" }),
    );
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "Invalid torrent metadata.",
    );
    expect(service.add).toHaveBeenCalledWith(
      { kind: "file", id: "file-1" },
      null,
    );
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
