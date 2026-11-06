import { afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { UpdateControls } from "./UpdateControls";
import type { DesktopApi, UpdateStatus } from "./bridge";

afterEach(cleanup);
const initial: UpdateStatus = { revision: 0, phase: "idle", version: null, notes: null, downloaded: 0, total: null, error: null, retry: null, transfers_stopped: false };
function fixture(status = initial) {
  let emit: (status: UpdateStatus) => void = () => {};
  const unsubscribe = vi.fn();
  const api: Pick<DesktopApi, "updateStatus" | "subscribeUpdates" | "checkUpdate" | "downloadUpdate" | "installUpdate"> = {
    updateStatus: vi.fn().mockResolvedValue(status),
    subscribeUpdates: vi.fn(async (callback) => { emit = callback; return unsubscribe; }),
    checkUpdate: vi.fn().mockResolvedValue({ ...initial, revision: 1, phase: "available", version: "0.2.0" }),
    downloadUpdate: vi.fn().mockResolvedValue({ ...initial, revision: 2, phase: "ready", version: "0.2.0", downloaded: 100, total: 100 }),
    installUpdate: vi.fn().mockResolvedValue(undefined),
  };
  return { api, emit: (value: UpdateStatus) => emit(value), unsubscribe };
}
it("checks, downloads, then requires separate explicit install confirmation", async () => {
  const user = userEvent.setup();
  const { api } = fixture();
  render(<UpdateControls api={api} />);
  await user.click(await screen.findByRole("button", { name: "Check for updates" }));
  await user.click(await screen.findByRole("button", { name: "Download update" }));
  expect(api.installUpdate).not.toHaveBeenCalled();
  await user.click(await screen.findByRole("button", { name: "Install and restart" }));
  expect(screen.getByRole("button", { name: "Restart now" })).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Keep using RusTor" }));
  expect(api.installUpdate).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Install and restart" }));
  await user.click(screen.getByRole("button", { name: "Restart now" }));
  expect(api.installUpdate).toHaveBeenCalledTimes(1);
});
it("shows determinate and unknown-size progress without enabling installation", async () => {
  const { api } = fixture({ ...initial, phase: "downloading", downloaded: 50, total: 100 });
  const view = render(<UpdateControls api={api} />);
  expect((await screen.findByRole("progressbar", { name: "Update download" })).getAttribute("value")).toBe("50");
  expect(screen.queryByRole("button", { name: "Install and restart" })).toBeNull();
  view.unmount();
  render(<UpdateControls api={fixture({ ...initial, phase: "downloading", downloaded: 50 }).api} />);
  expect((await screen.findByRole("progressbar")).hasAttribute("value")).toBe(false);
});
it("shows signing-unavailable and no-update states honestly", async () => {
  const view = render(<UpdateControls api={fixture({ ...initial, phase: "unavailable", error: { code: "updater_not_configured", message: "Release signing is not configured." } }).api} />);
  expect(await screen.findByText("Release signing is not configured.")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Check for updates" })).toBeNull();
  view.unmount();
  render(<UpdateControls api={fixture({ ...initial, phase: "current" }).api} />);
  expect(await screen.findByText("You’re up to date.")).toBeTruthy();
});
it("keeps the download for an install retry and reports stopped transfers after installer failure", async () => {
  const user = userEvent.setup();
  const { api } = fixture({ ...initial, phase: "failed", retry: "install", transfers_stopped: true, error: { code: "update_install_failed", message: "Transfers stopped. Retry installation or reopen RusTor." } });
  render(<UpdateControls api={api} />);
  expect(await screen.findByText(/Transfers stopped/)).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Retry installation" }));
  await user.click(screen.getByRole("button", { name: "Restart now" }));
  expect(api.installUpdate).toHaveBeenCalledTimes(1);
  expect(api.downloadUpdate).not.toHaveBeenCalled();
});
it("surfaces failed checks and releases its subscription on unmount", async () => {
  const user = userEvent.setup();
  const { api, unsubscribe } = fixture();
  vi.mocked(api.checkUpdate).mockRejectedValue({ message: "Offline. Try again." });
  const view = render(<UpdateControls api={api} />);
  await user.click(await screen.findByRole("button", { name: "Check for updates" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Offline. Try again.");
  view.unmount();
  expect(unsubscribe).toHaveBeenCalledTimes(1);
});
