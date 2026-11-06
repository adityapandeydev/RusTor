import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { Folder, LoaderCircle, X } from "lucide-react";
import { errorMessage } from "./bridge";
import type {
  DesktopApi,
  DownloadSettings,
  Selection,
  Snapshot,
} from "./bridge";
import { displayPath } from "./format";

const MAX_RATE_KIB = 976_562;

export function DownloadSettingsPanel({
  api,
  settings,
  trayAvailable,
  onSaved,
  onClose,
}: {
  api: DesktopApi;
  settings: DownloadSettings;
  trayAvailable: boolean;
  onSaved: (snapshot: Snapshot) => void;
  onClose: () => void;
}) {
  const [folder, setFolder] = useState<Selection | null>(null);
  const [download, setDownload] = useState(String(settings.download_limit_kib));
  const [upload, setUpload] = useState(String(settings.upload_limit_kib));
  const [background, setBackground] = useState(settings.continue_in_background);
  const [saving, setSaving] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const title = useRef<HTMLHeadingElement>(null);
  const alive = useRef(false);
  useEffect(() => {
    alive.current = true;
    title.current?.focus();
    return () => {
      alive.current = false;
    };
  }, []);
  const busy = saving || choosing;

  async function chooseFolder() {
    setChoosing(true);
    setError(null);
    try {
      const selected = await api.pickDestination();
      if (alive.current && selected) setFolder(selected);
    } catch (reason) {
      if (alive.current) setError(errorMessage(reason));
    } finally {
      if (alive.current) setChoosing(false);
    }
  }
  async function save(event: FormEvent) {
    event.preventDefault();
    if (busy) return;
    const values = [download, upload].map((value) =>
      value.trim() === "" ? NaN : Number(value),
    );
    if (
      values.some(
        (value) =>
          !Number.isInteger(value) ||
          (value !== 0 && (value < 16 || value > MAX_RATE_KIB)),
      )
    ) {
      setError(
        "Use 0 for unlimited, or a whole number from 16 to 976,562 KiB/s.",
      );
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const snapshot = await api.saveSettings({
        destination_id: folder?.id ?? null,
        download_limit_kib: values[0],
        upload_limit_kib: values[1],
        continue_in_background: trayAvailable && background,
      });
      if (alive.current) onSaved(snapshot);
    } catch (reason) {
      if (alive.current) setError(errorMessage(reason));
    } finally {
      if (alive.current) setSaving(false);
    }
  }

  async function quit() {
    setError(null);
    try {
      await api.quit();
    } catch (reason) {
      if (alive.current) setError(errorMessage(reason));
    }
  }
  return (
    <form
      className="download-settings-panel"
      aria-label="Download settings"
      onSubmit={(event) => void save(event)}
      onKeyDown={(event) => {
        if (event.key === "Escape" && !busy) onClose();
      }}
    >
      <div className="panel-heading">
        <h2 ref={title} tabIndex={-1}>
          Download settings
        </h2>
        <button
          type="button"
          className="icon-button"
          aria-label="Close download settings"
          disabled={busy}
          onClick={onClose}
        >
          <X size={17} aria-hidden="true" />
        </button>
      </div>
      <p className="settings-intro">
        Speed limits cover all transfers. Folder changes apply only to new
        torrents.
      </p>
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      <div className="destination-field">
        <div>
          <span className="field-label">Default download folder</span>
          <p>
            <Folder size={15} aria-hidden="true" />
            {displayPath(folder?.label ?? settings.default_destination)}
          </p>
        </div>
        <button
          type="button"
          className="button secondary"
          disabled={busy}
          onClick={() => void chooseFolder()}
        >
          Choose default folder
        </button>
      </div>
      <div className="settings-rate-fields">
        <label>
          Download limit (KiB/s)
          <input
            type="number"
            min="0"
            max={MAX_RATE_KIB}
            step="1"
            required
            value={download}
            disabled={busy}
            onChange={(event) => setDownload(event.target.value)}
            aria-describedby="speed-limit-help"
          />
        </label>
        <label>
          Upload limit (KiB/s)
          <input
            type="number"
            min="0"
            max={MAX_RATE_KIB}
            step="1"
            required
            value={upload}
            disabled={busy}
            onChange={(event) => setUpload(event.target.value)}
            aria-describedby="speed-limit-help"
          />
        </label>
      </div>
      <p className="settings-help" id="speed-limit-help">
        0 means unlimited. Otherwise use at least 16 KiB/s. 1 KiB = 1,024 bytes.
      </p>
      <div className="settings-background">
        <label>
          <input
            type="checkbox"
            checked={background && trayAvailable}
            disabled={busy || !trayAvailable}
            onChange={(event) => setBackground(event.target.checked)}
            aria-describedby="background-help"
          />
          Continue running when closed
        </label>
        <p className="settings-help" id="background-help">
          {trayAvailable
            ? "Keep downloads and seeding running in the system tray. Use Open RusTor to return, or Quit RusTor to save your session and exit. On Linux, the window stays minimized for easy access. Sleep, logout and shutdown interrupt transfers."
            : "The system tray is unavailable. Keep the window open to continue transfers; closing it quits RusTor."}
        </p>
      </div>
      <div className="settings-actions">
        <button
          type="button"
          className="button secondary quit-app"
          disabled={busy}
          onClick={() => void quit()}
        >
          Quit RusTor
        </button>
        <button
          type="button"
          className="button secondary"
          disabled={busy}
          onClick={onClose}
          aria-label="Cancel settings"
        >
          Cancel
        </button>
        <button className="button primary" disabled={busy}>
          {saving && (
            <LoaderCircle className="spin" size={16} aria-hidden="true" />
          )}
          {saving ? "Saving…" : "Save settings"}
        </button>
      </div>
    </form>
  );
}
