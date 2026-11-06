import { useCallback, useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import {
  ArrowDown,
  ArrowUp,
  Check,
  ChevronDown,
  Download,
  Compass,
  File,
  FilePlus2,
  Folder,
  HardDrive,
  Link2,
  LoaderCircle,
  Pause,
  Play,
  Plus,
  Radio,
  RotateCcw,
  Search,
  Settings2,
  Sun,
  Moon,
  X,
} from "lucide-react";
import { desktop, errorMessage } from "./bridge";
import { version } from "../package.json";
import type {
  DesktopApi,
  PreparedSelection,
  Selection,
  Snapshot,
  Torrent,
  TorrentState,
} from "./bridge";
import { bytes, displayPath } from "./format";
import { FileChoices } from "./FileChoices";
import { Discover } from "./Discover";
import { desktopSearch } from "./searchBridge";
import type { SearchApi } from "./searchBridge";
import { DownloadSettingsPanel } from "./DownloadSettingsPanel";

const labels: Record<TorrentState, string> = {
  checking: "Checking files",
  downloading: "Downloading",
  paused: "Paused",
  seeding: "Seeding",
  failed: "Needs attention",
};
type Filter = "all" | "downloading" | "paused" | "seeding";

export default function App({
  api = desktop,
  searchApi = desktopSearch,
}: {
  api?: DesktopApi;
  searchApi?: SearchApi;
}) {
  const [view, setView] = useState<"downloads" | "discover">("downloads");
  const [theme, setTheme] = useState<"dark" | "light">(() => {
    try {
      return localStorage.getItem("rustor-theme") === "light"
        ? "light"
        : "dark";
    } catch {
      return "dark";
    }
  });
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem("rustor-theme", theme);
    } catch {
      /* Appearance still works without storage. */
    }
  }, [theme]);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const settingsButton = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [magnet, setMagnet] = useState("");
  const [file, setFile] = useState<Selection | null>(null);
  const [destination, setDestination] = useState<Selection | null>(null);
  const [prepared, setPrepared] = useState<PreparedSelection | null>(null);
  const [selectedFiles, setSelectedFiles] = useState<Set<number>>(new Set());
  const [adding, setAdding] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const magnetInput = useRef<HTMLInputElement>(null);
  const mounted = useRef(false);
  const accept = useCallback((next: Snapshot) => {
    setSnapshot((current) =>
      !current || next.revision > current.revision ? next : current,
    );
  }, []);

  useEffect(() => {
    mounted.current = true;
    if (!api.available)
      return () => {
        mounted.current = false;
      };
    let active = true;
    let unsubscribe: (() => void) | undefined;
    void (async () => {
      try {
        const stop = await api.subscribe((next) => {
          if (active) accept(next);
        });
        if (!active) {
          stop();
          return;
        }
        unsubscribe = stop;
        const next = await api.snapshot();
        if (active) accept(next);
      } catch (reason) {
        if (active) setError(errorMessage(reason));
      }
    })();
    return () => {
      active = false;
      mounted.current = false;
      unsubscribe?.();
    };
  }, [api, accept]);

  useEffect(() => {
    if (open) magnetInput.current?.focus();
  }, [open]);

  useEffect(
    () => () => {
      if (prepared)
        void api.discardPrepared(prepared.id).catch(() => {
          /* Cache is bounded if cleanup fails. */
        });
    },
    [api, prepared],
  );

  function closeAdd() {
    setOpen(false);
    setPrepared(null);
  }

  function closeSettings() {
    setSettingsOpen(false);
    requestAnimationFrame(() => settingsButton.current?.focus());
  }

  const torrents = snapshot?.torrents ?? [];
  const ready = api.available && snapshot?.phase === "ready";
  const activeCount = torrents.filter(
    (torrent) =>
      torrent.state === "downloading" || torrent.state === "checking",
  ).length;
  const filtered = torrents.filter(
    (torrent) =>
      (filter === "all" ||
        (filter === "downloading"
          ? ["downloading", "checking"].includes(torrent.state)
          : torrent.state === filter)) &&
      (torrent.name ?? torrent.id)
        .toLowerCase()
        .includes(query.trim().toLowerCase()),
  );
  const down = torrents.reduce(
    (sum, torrent) => sum + torrent.download_bytes_per_second,
    0,
  );
  const up = torrents.reduce(
    (sum, torrent) => sum + torrent.upload_bytes_per_second,
    0,
  );

  async function choose(kind: "file" | "folder") {
    setChoosing(true);
    setError(null);
    try {
      const selection = await (kind === "file"
        ? api.pickTorrent()
        : api.pickDestination());
      if (mounted.current && selection) {
        if (kind === "file") {
          setPrepared(null);
          setFile(selection);
          setMagnet("");
        } else setDestination(selection);
      }
    } catch (reason) {
      if (mounted.current) setError(errorMessage(reason));
    } finally {
      if (mounted.current) setChoosing(false);
    }
  }

  async function add(event: FormEvent) {
    event.preventDefault();
    if (!ready || adding || (!file && !magnet.trim())) return;
    setAdding(true);
    setError(null);
    setNotice("");
    try {
      if (!prepared) {
        const preview = await api.prepare(
          file
            ? { kind: "file", id: file.id }
            : { kind: "magnet", value: magnet.trim() },
        );
        if (!mounted.current) {
          void api.discardPrepared(preview.id).catch(() => {});
          return;
        }
        setPrepared(preview);
        setSelectedFiles(
          new Set(
            preview.files
              .filter((file) => file.selected)
              .map((file) => file.index),
          ),
        );
        return;
      }
      if (selectedFiles.size === 0) return;
      const outcome = await api.add(
        prepared.id,
        [...selectedFiles].sort((a, b) => a - b),
        destination?.id ?? null,
      );
      if (!mounted.current) return;
      setNotice(
        outcome.already_managed
          ? "This torrent is already in your session."
          : "Torrent added. Your files are being checked.",
      );
      closeAdd();
      setFile(null);
      setMagnet("");
      setFilter("all");
      setQuery("");
      accept(await api.snapshot());
    } catch (reason) {
      if (mounted.current) {
        setError(errorMessage(reason));
        if (
          typeof reason === "object" &&
          reason !== null &&
          "code" in reason &&
          reason.code === "selection_expired"
        )
          setPrepared(null);
      }
    } finally {
      if (mounted.current) setAdding(false);
    }
  }

  async function control(torrent: Torrent, remove = false) {
    setError(null);
    setNotice("");
    setBusy((current) => new Set(current).add(torrent.id));
    try {
      if (remove) await api.remove(torrent.id);
      else if (torrent.state === "failed") await api.retry(torrent.id);
      else if (torrent.state === "paused") await api.resume(torrent.id);
      else await api.pause(torrent.id);
      if (mounted.current) {
        if (remove)
          setNotice(
            "Torrent stopped and removed. Files kept in your download folder.",
          );
        if (!remove && torrent.state === "failed")
          setNotice("Retry started. RusTor is checking the existing files.");
        accept(await api.snapshot());
      }
    } catch (reason) {
      if (mounted.current) setError(errorMessage(reason));
    } finally {
      if (mounted.current)
        setBusy((current) => {
          const next = new Set(current);
          next.delete(torrent.id);
          return next;
        });
    }
  }

  const filters: {
    id: Filter;
    label: string;
    icon: typeof Download;
    count: number;
  }[] = [
    {
      id: "all",
      label: "All transfers",
      icon: HardDrive,
      count: torrents.length,
    },
    {
      id: "downloading",
      label: "Downloading",
      icon: ArrowDown,
      count: activeCount,
    },
    {
      id: "paused",
      label: "Paused",
      icon: Pause,
      count: torrents.filter((t) => t.state === "paused").length,
    },
    {
      id: "seeding",
      label: "Seeding",
      icon: Radio,
      count: torrents.filter((t) => t.state === "seeding").length,
    },
  ];

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <a
          className="wordmark"
          onClick={() => setView("downloads")}
          href="#downloads"
          aria-label="RusTor downloads"
        >
          <svg viewBox="0 0 28 28" aria-hidden="true">
            <path d="M6 5h9a6 6 0 0 1 0 12H6V5Zm9 12 7 6M6 17v6" />
          </svg>
          <span>RusTor</span>
        </a>
        <nav aria-label="Library">
          <button
            className={`nav-item ${view === "discover" ? "selected" : ""}`}
            aria-current={view === "discover" ? "page" : undefined}
            onClick={() => setView("discover")}
          >
            <Compass size={18} aria-hidden="true" />
            <span>Discover</span>
          </button>
          <span className="nav-section-label">Transfers</span>
        </nav>
        <nav aria-label="Transfer filters">
          {filters.map(({ id, label, icon: Icon, count }) => (
            <button
              key={id}
              className={`nav-item ${view === "downloads" && filter === id ? "selected" : ""}`}
              aria-current={
                view === "downloads" && filter === id ? "page" : undefined
              }
              onClick={() => {
                setView("downloads");
                setFilter(id);
              }}
            >
              <Icon size={17} aria-hidden="true" />
              <span>{label}</span>
              <span className="nav-count">{count}</span>
            </button>
          ))}
        </nav>
      </aside>
      <main id="downloads" className="workspace">
        <header className="page-header">
          <div>
            <h1>{view === "downloads" ? "Downloads" : "Discover"}</h1>
            <p>
              {view === "downloads"
                ? "Your files, in motion."
                : "Find your next download."}
            </p>
          </div>
          <div className="header-actions">
            <button
              ref={settingsButton}
              className="icon-button settings-toggle"
              aria-label="Download settings"
              title="Download settings"
              aria-expanded={settingsOpen}
              disabled={!ready || adding || choosing || settingsOpen}
              onClick={() => setSettingsOpen(true)}
            >
              <Settings2 size={18} aria-hidden="true" />
            </button>
            <button
              className="icon-button theme-toggle"
              aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} mode`}
              title={`Switch to ${theme === "dark" ? "light" : "dark"} mode`}
              onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
            >
              {theme === "dark" ? (
                <Sun size={18} aria-hidden="true" />
              ) : (
                <Moon size={18} aria-hidden="true" />
              )}
            </button>
            <button
              className="button primary"
              disabled={!ready || adding || choosing}
              onClick={() => {
                setView("downloads");
                if (open && view === "downloads") closeAdd();
                else setOpen(true);
                setError(null);
              }}
            >
              <Plus size={17} aria-hidden="true" />
              Add torrent
            </button>
          </div>
        </header>
        {settingsOpen && snapshot && (
          <DownloadSettingsPanel
            api={api}
            settings={snapshot.settings}
            trayAvailable={snapshot.tray_available}
            startup={snapshot.startup}
            onClose={closeSettings}
            onSaved={(next) => {
              accept(next);
              closeSettings();
              setNotice(
                "Settings saved. Speed limits are active; new torrents will use the default folder.",
              );
            }}
          />
        )}
        {snapshot?.background_error && (
          <div className="error background-error" role="alert">
            {snapshot.background_error.message}
          </div>
        )}
        <div className="page-body" hidden={view !== "discover"}>
          <Discover
            api={searchApi}
            available={api.available}
            canAdd={ready && !adding && !choosing}
            managedIds={new Set(torrents.map((torrent) => torrent.id))}
            onChoose={(result) => {
              setPrepared(null);
              setFile(null);
              setMagnet(result.magnet);
              setDestination(null);
              setView("downloads");
              setOpen(true);
              setError(null);
              setNotice("");
            }}
          />
        </div>
        <div className="page-body download-page" hidden={view !== "downloads"}>
          <section
            className="transfer-strip"
            aria-label="Session transfer speeds"
          >
            <span>
              <ArrowDown size={15} aria-hidden="true" />
              <strong>{bytes(down)}</strong>
              <span>/s download</span>
            </span>
            <span>
              <ArrowUp size={15} aria-hidden="true" />
              <strong>{bytes(up)}</strong>
              <span>/s upload</span>
            </span>
            <span className="session-total">
              {activeCount} active<span className="dot-separator">·</span>
              {torrents.length} {torrents.length === 1 ? "torrent" : "torrents"}
            </span>
          </section>
          {!api.available && (
            <div className="notice">
              Open RusTor as a desktop app to add torrents and manage downloads.
              This browser view is a layout preview.
            </div>
          )}
          {(error || snapshot?.error) && (
            <div className="error" role="alert">
              {error ?? snapshot?.error?.message}
            </div>
          )}
          {notice && (
            <p className="success-notice" role="status">
              <Check size={15} aria-hidden="true" />
              {notice}
            </p>
          )}
          {open && (
            <form
              className="add-panel"
              onSubmit={add}
              onKeyDown={(event) => {
                if (event.key === "Escape" && !adding) closeAdd();
              }}
              aria-label="Add a torrent"
            >
              <div className="panel-heading">
                <h2>Add a torrent</h2>
                <button
                  type="button"
                  className="icon-button"
                  aria-label="Close add torrent"
                  disabled={adding}
                  onClick={closeAdd}
                >
                  <X size={17} />
                </button>
              </div>
              <label htmlFor="magnet">Magnet link or infohash</label>
              <div className="magnet-field">
                <Link2 size={17} aria-hidden="true" />
                <input
                  ref={magnetInput}
                  id="magnet"
                  value={magnet}
                  onChange={(event) => {
                    setPrepared(null);
                    setMagnet(event.target.value);
                    if (event.target.value) setFile(null);
                  }}
                  placeholder="magnet:?xt=urn:btih:…"
                  autoComplete="off"
                  spellCheck={false}
                  disabled={adding}
                />
              </div>
              <div className="file-selection">
                <button
                  type="button"
                  className="button secondary"
                  disabled={adding || choosing}
                  onClick={() => void choose("file")}
                >
                  <FilePlus2 size={16} aria-hidden="true" />
                  Choose .torrent file
                </button>
                {file && (
                  <span className="selected-file">
                    <File size={15} aria-hidden="true" />
                    {file.label}
                    <button
                      type="button"
                      className="icon-button small"
                      aria-label="Clear torrent file"
                      onClick={() => {
                        setFile(null);
                        setPrepared(null);
                      }}
                      disabled={adding}
                    >
                      <X size={14} />
                    </button>
                  </span>
                )}
              </div>
              <div className="destination-field">
                <div>
                  <span className="field-label">Save to</span>
                  <p
                    title={displayPath(
                      destination?.label ?? snapshot?.default_destination ?? "",
                    )}
                  >
                    <Folder size={15} aria-hidden="true" />
                    {displayPath(
                      destination?.label ?? snapshot?.default_destination ?? "",
                    )}
                  </p>
                </div>
                <button
                  type="button"
                  className="button secondary"
                  disabled={adding || choosing}
                  onClick={() => void choose("folder")}
                >
                  Choose folder
                </button>
              </div>
              {prepared && (
                <FileChoices
                  key={prepared.id}
                  files={prepared.files}
                  selected={selectedFiles}
                  onChange={setSelectedFiles}
                  disabled={adding}
                />
              )}
              <div className="form-footer">
                <p>
                  {prepared
                    ? "Downloading and seeding share your IP address with peers."
                    : "Load the file list before downloading. Magnet metadata may require contacting peers."}
                </p>
                <button
                  className="button primary"
                  disabled={
                    adding ||
                    choosing ||
                    (!file && !magnet.trim()) ||
                    (prepared !== null && selectedFiles.size === 0)
                  }
                >
                  {adding ? (
                    <LoaderCircle
                      className="spin"
                      size={16}
                      aria-hidden="true"
                    />
                  ) : (
                    <ArrowDown size={16} aria-hidden="true" />
                  )}
                  {adding
                    ? prepared
                      ? "Starting download…"
                      : "Loading files…"
                    : prepared
                      ? "Start download"
                      : "Load file list"}
                </button>
              </div>
            </form>
          )}
          <div className="list-toolbar">
            <h2>
              {filters.find((item) => item.id === filter)?.label}
              <span>{filtered.length}</span>
            </h2>
            <label className="filter-field">
              <Search size={15} aria-hidden="true" />
              <input
                aria-label="Filter transfers"
                placeholder="Filter transfers"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
            </label>
          </div>
          {filtered.length > 0 ? (
            <div className="torrent-list" aria-label="Torrents">
              {filtered.map((torrent) => (
                <TorrentRow
                  key={torrent.id}
                  torrent={torrent}
                  busy={busy.has(torrent.id)}
                  onControl={() => void control(torrent)}
                  onRemove={() => void control(torrent, true)}
                />
              ))}
            </div>
          ) : (
            <section className="empty-state">
              <div className="empty-symbol">
                <Download size={32} strokeWidth={1.4} aria-hidden="true" />
              </div>
              <h2>
                {!api.available
                  ? "Your downloads live here"
                  : snapshot?.phase === "failed"
                    ? "Session could not start"
                    : !snapshot || snapshot.phase === "starting"
                      ? "Starting your session"
                      : torrents.length > 0
                        ? "No matching transfers"
                        : "Ready when you are"}
              </h2>
              <p>
                {snapshot?.phase === "failed"
                  ? "Check the error above, then restart RusTor."
                  : torrents.length > 0
                    ? "Try another filter to find your torrent."
                    : "Add a magnet link or a .torrent file. RusTor will take care of the transfer."}
              </p>
              {torrents.length > 0 ? (
                <button
                  className="button secondary"
                  onClick={() => {
                    setFilter("all");
                    setQuery("");
                  }}
                >
                  Clear filters
                </button>
              ) : (
                snapshot?.phase !== "failed" && (
                  <span className="empty-caption">
                    Your session will be restored when you return.
                  </span>
                )
              )}
            </section>
          )}
        </div>
        <footer className="page-footer">
          <span>
            RusTor <span className="version">{version}</span>
          </span>
        </footer>
      </main>
    </div>
  );
}

function TorrentRow({
  torrent,
  busy,
  onControl,
  onRemove,
}: {
  torrent: Torrent;
  busy: boolean;
  onControl: () => void;
  onRemove: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const name = torrent.name || torrent.id;
  const ratio =
    torrent.total_bytes > 0
      ? Math.min(1, torrent.downloaded_bytes / torrent.total_bytes)
      : 0;
  return (
    <article className={`torrent-row state-${torrent.state}`}>
      <div className="torrent-main">
        <button
          className="torrent-identity"
          aria-expanded={expanded}
          onClick={() => setExpanded(!expanded)}
        >
          <span className="file-mark">
            <File size={19} aria-hidden="true" />
          </span>
          <span className="torrent-name">
            <strong title={name}>{name}</strong>
            <span>
              {bytes(torrent.downloaded_bytes)} of {bytes(torrent.total_bytes)}
            </span>
          </span>
          <ChevronDown
            size={15}
            className={expanded ? "chevron expanded" : "chevron"}
            aria-hidden="true"
          />
        </button>
        <div className="torrent-progress">
          <div>
            <span className="state-label">{labels[torrent.state]}</span>
            <span>{Math.round(ratio * 100)}%</span>
          </div>
          <div
            className="download-progress"
            role="progressbar"
            aria-valuenow={Math.round(ratio * 100)}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-label={`${name} progress`}
          >
            <span style={{ transform: `scaleX(${ratio})` }} />
          </div>
        </div>
        <div className="torrent-rate">
          <span>
            <ArrowDown size={13} aria-hidden="true" />
            {bytes(torrent.download_bytes_per_second)}/s
          </span>
          <span>
            <ArrowUp size={13} aria-hidden="true" />
            {bytes(torrent.upload_bytes_per_second)}/s
          </span>
        </div>
        <div className="torrent-actions">
          {torrent.state !== "failed" && (
            <button
              className="icon-button control-button"
              aria-label={`${torrent.state === "paused" ? "Resume" : "Pause"} ${name}`}
              disabled={busy}
              onClick={onControl}
            >
              {busy ? (
                <LoaderCircle className="spin" size={17} />
              ) : torrent.state === "paused" ? (
                <Play size={17} />
              ) : (
                <Pause size={17} />
              )}
            </button>
          )}
          <button
            className="icon-button remove-button"
            aria-label={`Stop and remove ${name} (keep files)`}
            title="Stop and remove torrent · keep files"
            disabled={busy}
            onClick={onRemove}
          >
            <X size={18} aria-hidden="true" />
          </button>
        </div>
      </div>
      {torrent.state === "failed" && (
        <section
          className="transfer-failure"
          aria-label={`${name} transfer failure`}
        >
          <div className="failure-heading">
            <div>
              <h3>Transfer stopped</h3>
              <p>
                Check that the download folder is available, writable, and has
                free space, then retry.
              </p>
            </div>
            <button
              className="button secondary retry-button"
              aria-label={`Retry ${name}`}
              disabled={busy}
              onClick={onControl}
            >
              {busy ? (
                <LoaderCircle className="spin" size={15} aria-hidden="true" />
              ) : (
                <RotateCcw size={15} aria-hidden="true" />
              )}
              {busy ? "Retrying…" : "Retry"}
            </button>
          </div>
          {torrent.error && (
            <details className="failure-details">
              <summary>Engine error details</summary>
              <p>{torrent.error}</p>
            </details>
          )}
        </section>
      )}
      {expanded && (
        <div className="torrent-details">
          <dl>
            <div>
              <dt>Download folder</dt>
              <dd>{displayPath(torrent.destination)}</dd>
            </div>
            <div>
              <dt>Infohash</dt>
              <dd className="infohash">{torrent.id}</dd>
            </div>
            <div>
              <dt>Connected peers</dt>
              <dd>{torrent.peers}</dd>
            </div>
            <div>
              <dt>Uploaded</dt>
              <dd>{bytes(torrent.uploaded_bytes)}</dd>
            </div>
          </dl>
        </div>
      )}
    </article>
  );
}
