import { useState } from "react";
import type { TorrentFile } from "./bridge";
import { bytes } from "./format";

const PAGE_SIZE = 100;

export function FileChoices({
  files,
  selected,
  onChange,
  disabled,
}: {
  files: TorrentFile[];
  selected: Set<number>;
  onChange: (selected: Set<number>) => void;
  disabled: boolean;
}) {
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const matching = files.filter((file) =>
    file.path.toLowerCase().includes(query.trim().toLowerCase()),
  );
  const pageCount = Math.max(1, Math.ceil(matching.length / PAGE_SIZE));
  const current = Math.min(page, pageCount - 1);
  const visible = matching.slice(
    current * PAGE_SIZE,
    (current + 1) * PAGE_SIZE,
  );
  const total = files.reduce(
    (sum, file) => sum + (selected.has(file.index) ? file.length : 0),
    0,
  );
  return (
    <section className="file-choices" aria-label="Files to download">
      <div className="file-choices-heading">
        <div>
          <h3>Files to download</h3>
          <p aria-live="polite">
            {selected.size} of {files.length} selected{" "}
            <span aria-hidden="true">·</span> {bytes(total)}
          </p>
        </div>
        <div className="file-choice-actions">
          <button
            type="button"
            className="text-button"
            disabled={disabled}
            onClick={() => onChange(new Set(files.map((file) => file.index)))}
          >
            Select all files
          </button>
          <button
            type="button"
            className="text-button"
            disabled={disabled}
            onClick={() => onChange(new Set())}
          >
            Clear selection
          </button>
        </div>
      </div>
      {files.length > PAGE_SIZE && (
        <input
          className="file-search"
          aria-label="Find files in torrent"
          placeholder="Find a file"
          value={query}
          disabled={disabled}
          onChange={(event) => {
            setQuery(event.target.value);
            setPage(0);
          }}
        />
      )}
      <div className="file-choice-list">
        {visible.map((file) => (
          <label className="file-choice" key={file.index}>
            <input
              type="checkbox"
              aria-label={file.path}
              checked={selected.has(file.index)}
              disabled={disabled}
              onChange={(event) => {
                const next = new Set(selected);
                if (event.target.checked) next.add(file.index);
                else next.delete(file.index);
                onChange(next);
              }}
            />
            <span className="file-choice-path">{file.path}</span>
            <span className="file-choice-size">{bytes(file.length)}</span>
          </label>
        ))}
        {matching.length === 0 && (
          <p className="file-choice-empty">No files match your search.</p>
        )}
      </div>
      {pageCount > 1 && (
        <div className="file-pagination">
          <button
            type="button"
            className="text-button"
            disabled={disabled || current === 0}
            onClick={() => setPage(current - 1)}
          >
            Previous files
          </button>
          <span>
            Page {current + 1} of {pageCount}
          </span>
          <button
            type="button"
            className="text-button"
            disabled={disabled || current + 1 === pageCount}
            onClick={() => setPage(current + 1)}
          >
            Next files
          </button>
        </div>
      )}
      <p className="file-choice-note">
        Choose at least one file. Shared pieces may include data from skipped
        files.
      </p>
    </section>
  );
}
