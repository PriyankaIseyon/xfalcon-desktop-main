import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ImageOff, LayoutDashboard } from "lucide-react";
import { cn } from "../../lib/cn";
import { Skeleton } from "../../ui/Skeleton";
import {
  peekStoredThumbnailUrl,
  peekThumbnailDocumentSrc,
  storedThumbnailUrl,
  thumbnailDocumentSrc,
  ThumbnailMissing,
} from "./storedThumbnails";

/** How long a card must stay on screen before its thumbnail is fetched. Opening
 * a report asks DA to assemble its page, so a fast scroll past a long library
 * must not start one fetch per card it flies over. */
const VISIBLE_DWELL_MS = 150;

/** How far beyond the visible part of the library a card counts as near: one
 * screen above and below. A card is brought in before it scrolls into view, and
 * a live drawing is let go only once it is that far away, so ordinary scrolling
 * back and forth never reloads one. */
const NEAR_MARGIN = "100% 0px";

/** A report's card picture. DA's stored image when it holds one: one small
 * request, cached, with nothing to run. Otherwise the report itself, drawn from
 * the same isolated origin as the viewer.
 *
 * A stored image, once shown, stays: it is a still picture, and taking it down
 * would only make it flash back in. A live drawing runs the dashboard's scripts,
 * so it is taken down once the card is well out of view. */
export function ReportThumbnail({
  reportId,
  updatedAt,
  thumbnailAt,
  live = true,
  className,
}: {
  reportId: string;
  updatedAt: bigint;
  /** The stored image's version, or `null` when DA holds none. */
  thumbnailAt: bigint | null;
  /** Whether a report with no stored image is drawn live. A small picture (a
   * table row's) shows a quiet dashboard tile instead. */
  live?: boolean;
  className?: string;
}) {
  const container = useRef<HTMLDivElement>(null);
  // A picture already in memory is shown from the first frame: a card that comes
  // back (a search cleared, a layout switched) is simply there, with no wait for
  // the dwell and no request.
  const [visible, setVisible] = useState(() =>
    thumbnailAt !== null
      ? peekStoredThumbnailUrl(reportId, thumbnailAt) !== null
      : live && peekThumbnailDocumentSrc(reportId, updatedAt) !== null,
  );
  const [width, setWidth] = useState(0);
  // The version DA said it held but then did not have: drawn live from then on.
  const [missing, setMissing] = useState<bigint | null>(null);
  const storedAt = thumbnailAt !== null && thumbnailAt !== missing ? thumbnailAt : null;
  // Read by the observer, which is set up once.
  const keepWhenAway = useRef(storedAt !== null);
  keepWhenAway.current = storedAt !== null;

  // Measured before the first paint, so a held live drawing is scaled from the
  // first frame rather than after the resize observer's first report.
  useLayoutEffect(() => {
    const element = container.current;
    if (element) setWidth(element.getBoundingClientRect().width);
  }, []);

  useEffect(() => {
    const element = container.current;
    if (!element) return;
    const resize = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    resize.observe(element);
    let dwell: number | undefined;
    const visibility = new IntersectionObserver(
      ([entry]) => {
        window.clearTimeout(dwell);
        if (entry.isIntersecting) dwell = window.setTimeout(() => setVisible(true), VISIBLE_DWELL_MS);
        else if (!keepWhenAway.current) setVisible(false);
      },
      // The library scrolls inside its own pane, so "near" is measured against it.
      { root: element.closest(".reports-workspace__scroll"), rootMargin: NEAR_MARGIN },
    );
    visibility.observe(element);
    return () => {
      window.clearTimeout(dwell);
      resize.disconnect();
      visibility.disconnect();
    };
  }, []);

  return (
    <div ref={container} className={cn(className ?? "report-card__thumbnail")} aria-hidden="true" inert>
      {storedAt === null && !live ? (
        <span className="report-thumbnail-tile">
          <LayoutDashboard size={18} strokeWidth={1.5} />
        </span>
      ) : !visible ? null : storedAt !== null ? (
        // An image fills its frame by CSS, so it needs no measured width.
        <StoredThumbnail reportId={reportId} version={storedAt} onMissing={() => setMissing(storedAt)} />
      ) : width > 0 ? (
        <ThumbnailDocument key={`${reportId}:${updatedAt}`} reportId={reportId} updatedAt={updatedAt} width={width} />
      ) : null}
    </div>
  );
}

/**
 * The stored image. One already held in memory is painted at once, with no
 * placeholder and no fade, so a card that comes back into view is simply there.
 * A new version replaces the old in place: the old picture stays until the new
 * one is ready, so an update never blanks the card.
 */
function StoredThumbnail({
  reportId,
  version,
  onMissing,
}: {
  reportId: string;
  version: bigint;
  onMissing: () => void;
}) {
  const [state, setState] = useState<{ src: string | null; failed: boolean }>(() => ({
    src: peekStoredThumbnailUrl(reportId, version),
    failed: false,
  }));
  // Whether a picture has been painted here. Only the first one fades in.
  const [painted, setPainted] = useState(() => state.src !== null);
  useEffect(() => {
    const held = peekStoredThumbnailUrl(reportId, version);
    if (held !== null) {
      setState((current) => (current.src === held ? current : { src: held, failed: false }));
      return;
    }
    let current = true;
    storedThumbnailUrl(reportId, version)
      .then((src) => {
        if (current) setState({ src, failed: false });
      })
      .catch((cause: unknown) => {
        if (!current) return;
        if (cause instanceof ThumbnailMissing) onMissing();
        else setState({ src: null, failed: true });
      });
    return () => {
      current = false;
    };
    // `onMissing` is a fresh closure each render and changes nothing it does.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reportId, version]);

  return (
    <>
      {state.failed ? <PreviewUnavailable /> : null}
      {!state.failed && !painted ? <Skeleton className="absolute inset-0 h-full w-full" /> : null}
      {state.src ? (
        <img
          alt=""
          src={state.src}
          decoding="async"
          draggable={false}
          className="report-card__thumbnail-image"
          data-painted={painted}
          onLoad={() => setPainted(true)}
          onError={() => setState({ src: null, failed: true })}
        />
      ) : null}
    </>
  );
}

/** The quiet stand-in for a picture that cannot be shown. */
function PreviewUnavailable() {
  return (
    <span className="report-card__preview-unavailable type-caption text-muted">
      <ImageOff aria-hidden size={18} strokeWidth={1.5} />
      Preview unavailable
    </span>
  );
}

/** A report drawn live, from the document opened for its saved version. One
 * opened before is drawn again from the local preview store without asking DA. */
function ThumbnailDocument({ reportId, updatedAt, width }: { reportId: string; updatedAt: bigint; width: number }) {
  const [state, setState] = useState<{ src: string | null; failed: boolean }>(() => ({
    src: peekThumbnailDocumentSrc(reportId, updatedAt),
    failed: false,
  }));
  const [loaded, setLoaded] = useState(false);
  useEffect(() => {
    if (peekThumbnailDocumentSrc(reportId, updatedAt) !== null) return;
    let current = true;
    thumbnailDocumentSrc(reportId, updatedAt)
      .then((src) => {
        if (current) setState({ src, failed: false });
      })
      .catch(() => {
        if (current) setState({ src: null, failed: true });
      });
    return () => {
      current = false;
    };
  }, [reportId, updatedAt]);

  return (
    <>
      {state.failed ? <PreviewUnavailable /> : null}
      {!state.failed && !loaded ? <Skeleton className="absolute inset-0 h-full w-full" /> : null}
      {state.src ? (
        <iframe
          title="Report thumbnail"
          src={state.src}
          sandbox="allow-scripts"
          referrerPolicy="no-referrer"
          tabIndex={-1}
          scrolling="no"
          className="report-card__thumbnail-frame"
          style={{ transform: `scale(${width / 1200})`, opacity: loaded ? 1 : 0 }}
          onLoad={() => setLoaded(true)}
        />
      ) : null}
    </>
  );
}
