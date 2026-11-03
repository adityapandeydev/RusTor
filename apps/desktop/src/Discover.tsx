import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import {
  ArrowDown,
  ArrowDownToLine,
  Compass,
  LoaderCircle,
  Search,
  Square,
} from "lucide-react";
import { errorMessage } from "./bridge";
import { bytes } from "./format";
import type {
  SearchApi,
  SearchCategory,
  SearchResult,
  SearchSnapshot,
  ProviderStatus,
} from "./searchBridge";

const categories: { id: SearchCategory | null; label: string }[] = [
  { id: null, label: "All" },
  { id: "movies", label: "Movies" },
  { id: "tv", label: "TV" },
  { id: "games", label: "Games" },
  { id: "anime", label: "Anime" },
];
function providerMessage(status: ProviderStatus): string {
  switch (status.state) {
    case "pending":
      return "Waiting";
    case "running":
      return "Searching…";
    case "complete":
      return `${status.accepted} results${status.rejected ? ` · ${status.rejected} invalid entries skipped` : ""}`;
    case "failed":
      if (status.failure.code === "rate_limited")
        return `Rate limited${status.failure.retry_after_seconds ? ` · try again in ${status.failure.retry_after_seconds}s` : " · try again later"}`;
      return status.failure.code === "invalid_response"
        ? "Feed could not be read. Try again later."
        : "Unavailable. Try again later.";
    case "timed_out":
    case "deadline_exceeded":
      return "Search timed out. Try again.";
    case "cancelled":
      return "Stopped";
  }
}
export function Discover({
  api,
  available,
  canAdd,
  managedIds,
  onChoose,
}: {
  api: SearchApi;
  available: boolean;
  canAdd: boolean;
  managedIds: Set<string>;
  onChoose: (result: SearchResult) => void;
}) {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<SearchCategory | null>(null);
  const [snapshot, setSnapshot] = useState<SearchSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [sort, setSort] = useState("relevance");
  const [hideDead, setHideDead] = useState(false);
  const [page, setPage] = useState(0);
  const generation = useRef(0);
  const activeId = useRef<number | null>(null);
  useEffect(
    () => () => {
      generation.current++;
      if (activeId.current !== null)
        void api.cancel(activeId.current).catch(() => {});
    },
    [api],
  );
  const searchId = snapshot?.id;
  const searchPhase = snapshot?.phase;
  useEffect(() => {
    if (searchId === undefined || searchPhase !== "running") return;
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    const id = searchId;
    async function poll() {
      try {
        const next = await api.snapshot(id);
        if (!live || activeId.current !== id) return;
        setSnapshot(next);
        if (next.phase === "running")
          timer = setTimeout(() => void poll(), 250);
      } catch (reason) {
        if (live && activeId.current === id) setError(errorMessage(reason));
      }
    }
    timer = setTimeout(() => void poll(), 250);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [api, searchId, searchPhase]); // Poll one bounded snapshot at a time.
  async function search(event?: FormEvent, nextCategory = category) {
    event?.preventDefault();
    if (!available || starting) return;
    const ticket = ++generation.current;
    setStarting(true);
    setError(null);
    setPage(0);
    try {
      const next = await api.start(query, nextCategory);
      if (generation.current !== ticket) {
        void api.cancel(next.id).catch(() => {});
        return;
      }
      activeId.current = next.id;
      setSnapshot(next);
    } catch (reason) {
      if (generation.current === ticket) setError(errorMessage(reason));
    } finally {
      if (generation.current === ticket) setStarting(false);
    }
  }
  async function cancel() {
    if (!snapshot) return;
    try {
      await api.cancel(snapshot.id);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }
  const running = starting || (snapshot?.phase === "running" && !error);
  const unsupported = category !== null && category !== "anime";
  const rows = (unsupported ? [] : (snapshot?.results ?? [])).filter(
    (row) => !hideDead || row.seeders !== 0 || row.has_unknown_health,
  );
  if (sort !== "relevance")
    rows.sort((a, b) => {
      const left =
        sort === "size"
          ? a.size_bytes
          : sort === "added"
            ? a.added_unix_ms
            : a.seeders;
      const right =
        sort === "size"
          ? b.size_bytes
          : sort === "added"
            ? b.added_unix_ms
            : b.seeders;
      return left === null
        ? right === null
          ? 0
          : 1
        : right === null
          ? -1
          : right - left;
    });
  const lastPage = Math.max(0, Math.ceil(rows.length / 25) - 1);
  const currentPage = Math.min(page, lastPage);
  return (
    <section className="discover" aria-label="Discover torrents">
      <form
        className="discovery-search"
        onSubmit={(event) => void search(event)}
      >
        <Search size={21} aria-hidden="true" />
        <input
          aria-label="Search torrents"
          placeholder="Search by title or keyword"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          maxLength={256}
          disabled={!available}
        />
        <button className="button primary" disabled={!available || starting}>
          {starting ? (
            <LoaderCircle size={16} className="spin" aria-hidden="true" />
          ) : (
            <Search size={16} aria-hidden="true" />
          )}{" "}
          Search
        </button>
      </form>
      <div className="discover-controls">
        <div
          className="category-tabs"
          role="group"
          aria-label="Search category"
        >
          {categories.map((item) => (
            <button
              type="button"
              key={item.label}
              aria-pressed={category === item.id}
              disabled={starting}
              onClick={() => {
                setCategory(item.id);
                setPage(0);
                if (snapshot) void search(undefined, item.id);
              }}
            >
              {item.label}
            </button>
          ))}
        </div>
        <span className="source-note">Source: Nyaa RSS</span>
      </div>
      {error && (
        <div className="error" role="alert">
          {error}{" "}
          <button className="text-button" onClick={() => void search()}>
            Try again
          </button>
        </div>
      )}
      {unsupported ? (
        <div className="discover-empty">
          <Compass size={28} strokeWidth={1.5} aria-hidden="true" />
          <h2>
            {categories.find((item) => item.id === category)?.label} sources are
            coming next
          </h2>
          <p>
            This category is ready for providers. The current source, Nyaa, is
            listed under Anime.
          </p>
          <button
            className="button secondary"
            onClick={() => {
              setCategory("anime");
              if (snapshot) void search(undefined, "anime");
            }}
          >
            Explore Anime
          </button>
        </div>
      ) : (
        <>
          {snapshot && (
            <div className="search-summary">
              <p role="status">
                {running
                  ? "Searching sources…"
                  : snapshot.phase === "cancelled"
                    ? "Search stopped"
                    : snapshot.phase === "timed_out"
                      ? "Search timed out"
                      : `${rows.length} ${rows.length === 1 ? "result" : "results"}`}
                {snapshot.query && <span> for “{snapshot.query}”</span>}
              </p>
              {running && (
                <button
                  className="button secondary"
                  onClick={() => void cancel()}
                >
                  <Square size={12} aria-hidden="true" /> Stop search
                </button>
              )}
            </div>
          )}
          {snapshot?.providers.map((provider) => (
            <p
              className={`provider-status ${provider.status.state === "failed" ? "provider-failed" : ""}`}
              key={provider.descriptor.id}
            >
              {provider.status.state === "running" && (
                <LoaderCircle size={13} className="spin" aria-hidden="true" />
              )}
              <strong>{provider.descriptor.label}</strong>{" "}
              {providerMessage(provider.status)}
            </p>
          ))}
          {snapshot?.truncated && (
            <p className="provider-status">
              Showing a limited set of results. Refine your search to narrow it
              down.
            </p>
          )}
          {(snapshot?.results.length ?? 0) > 0 && (
            <div className="result-toolbar">
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={hideDead}
                  onChange={(event) => {
                    setHideDead(event.target.checked);
                    setPage(0);
                  }}
                />{" "}
                Hide zero seeders
              </label>
              <label>
                Sort by{" "}
                <select
                  value={sort}
                  onChange={(event) => {
                    setSort(event.target.value);
                    setPage(0);
                  }}
                >
                  <option value="relevance">Relevance</option>
                  <option value="seeders">Most seeders</option>
                  <option value="size">Largest size</option>
                  <option value="added">Newest</option>
                </select>
              </label>
            </div>
          )}
          {rows.length > 0 ? (
            <>
              <div className="result-head" aria-hidden="true">
                <span>Torrent</span>
                <span>Size</span>
                <span>Seeders</span>
                <span>Action</span>
              </div>
              <div className="search-results" aria-label="Search results">
                {rows
                  .slice(currentPage * 25, currentPage * 25 + 25)
                  .map((row) => (
                    <article className="search-result" key={row.info_hash}>
                      <div className="result-identity">
                        <h2>
                          <bdi>{row.name}</bdi>
                        </h2>
                        <p>
                          {row.providers.join(" · ")}
                          <span>·</span>
                          {row.added_unix_ms === null
                            ? "Date unavailable"
                            : new Date(row.added_unix_ms).toLocaleDateString()}
                          {row.trackers_truncated
                            ? " · Tracker list limited"
                            : ""}
                        </p>
                      </div>
                      <span className="result-size">
                        {row.size_bytes === null
                          ? "Unknown size"
                          : bytes(row.size_bytes)}
                      </span>
                      <span
                        className="result-seeders"
                        title={
                          row.seeders === null
                            ? "Swarm health is unknown"
                            : `${row.seeders} reported seeders`
                        }
                      >
                        <ArrowDown size={13} aria-hidden="true" />
                        {row.seeders ?? "Unknown"}
                      </span>
                      <button
                        className="button secondary result-action"
                        disabled={!canAdd || managedIds.has(row.info_hash)}
                        onClick={() => onChoose(row)}
                      >
                        <ArrowDownToLine size={15} aria-hidden="true" />
                        {managedIds.has(row.info_hash)
                          ? "In downloads"
                          : "Choose files"}
                      </button>
                    </article>
                  ))}
              </div>
              {lastPage > 0 && (
                <div className="file-pagination">
                  <span>
                    Page {currentPage + 1} of {lastPage + 1}
                  </span>
                  <div>
                    <button
                      className="button secondary"
                      disabled={currentPage === 0}
                      onClick={() => setPage(currentPage - 1)}
                    >
                      Previous
                    </button>
                    <button
                      className="button secondary"
                      disabled={currentPage === lastPage}
                      onClick={() => setPage(currentPage + 1)}
                    >
                      Next
                    </button>
                  </div>
                </div>
              )}
            </>
          ) : (
            <div className="discover-empty">
              {running ? (
                <LoaderCircle size={28} className="spin" aria-hidden="true" />
              ) : (
                <Compass size={32} strokeWidth={1.4} aria-hidden="true" />
              )}
              <h2>
                {running
                  ? "Looking for your next download"
                  : snapshot
                    ? "No results to show"
                    : "Find a torrent. Make it yours."}
              </h2>
              <p>
                {!available
                  ? "Open the desktop app to search live sources."
                  : running
                    ? "Results appear as each source responds. You can stop at any time."
                    : snapshot
                      ? "Try a different title or a shorter search. Source status is shown above."
                      : "Search a title, compare swarm health, then choose the files you want to download."}
              </p>
              {!snapshot && (
                <span className="empty-caption">
                  Nyaa is the first connected source. More providers are on the
                  way.
                </span>
              )}
            </div>
          )}
        </>
      )}
      <p className="discovery-footnote">
        Search covers the latest RSS page. Availability and seed counts come
        from the source.
      </p>
    </section>
  );
}
