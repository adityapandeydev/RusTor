import { useEffect, useRef, useState } from "react";
import { File } from "lucide-react";
import { invoke, isTauri } from "@tauri-apps/api/core";

export type ArtworkLoader = (infoHash: string) => Promise<string | null>;
const loadArtwork: ArtworkLoader = (infoHash) =>
  isTauri() ? invoke("get_artwork", { infoHash }) : Promise.resolve(null);

interface Props {
  infoHash: string;
  available?: boolean;
  compact?: boolean;
  load?: ArtworkLoader;
}

/** Decorative artwork; the adjacent torrent title supplies its accessible name. */
export function Artwork(props: Props) {
  return <ArtworkImage key={`${props.infoHash}:${props.available}`} {...props} />;
}

function ArtworkImage({ infoHash, available = true, compact = false, load = loadArtwork }: Props) {
  const frame = useRef<HTMLSpanElement>(null);
  const [source, setSource] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!available) return;
    let cancelled = false;
    let started = false;
    const start = () => {
      if (started) return;
      started = true;
      void load(infoHash).then((value) => {
        if (!cancelled && value && value.length <= 180_000 && /^data:image\/png;base64,[A-Za-z0-9+/=]+$/.test(value)) {
          setSource(value);
        }
      }).catch(() => { /* Optional artwork never interrupts search or transfers. */ });
    };
    const observer = typeof IntersectionObserver === "undefined" ? null :
      new IntersectionObserver((entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          observer?.disconnect();
          start();
        }
      }, { rootMargin: "160px" });
    if (observer && frame.current) observer.observe(frame.current);
    else start();
    return () => { cancelled = true; observer?.disconnect(); };
  }, [infoHash, available, load]);

  return <span ref={frame} className={`artwork${compact ? " artwork-compact" : ""}`} aria-hidden="true">
    <File size={compact ? 17 : 20} strokeWidth={1.4} />
    {source && !failed && <img src={source} alt="" decoding="async" draggable={false}
      className={visible ? "artwork-loaded" : ""}
      onLoad={() => setVisible(true)} onError={() => setFailed(true)} />}
  </span>;
}
