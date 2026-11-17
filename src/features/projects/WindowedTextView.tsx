import React from "react";
import { ChevronFirst, ChevronLast, ChevronLeft, ChevronRight } from "lucide-react";
import { formatBytes } from "../../lib/formatBytes";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { IconButton } from "../../ui/IconButton";
import { Tooltip } from "../../ui/Tooltip";
import { Spinner } from "../../ui/Spinner";
import { PROJECT_FILE_LIMITS } from "./projectFileLimits";
import { readWindow, type TextWindow } from "./projectFileLoad";
import { describeProjectError } from "./projectFileText";
import { displayLine, longestLine } from "./fileLines";
import { VirtualLines } from "./VirtualLines";

type WindowState =
  | { status: "loading"; offset: number }
  | { status: "ready"; window: TextWindow }
  | { status: "error"; offset: number; message: string };

/**
 * A text file too large to hold whole, a window at a time (spec:
 * project-file-editing, "A local file opens at any size up to 10 GiB").
 *
 * Read-only: an edit needs the whole file in hand. Positions are bytes, not
 * lines — counting the lines of a multi-gigabyte file would mean reading all of
 * it before showing any of it.
 */
export function WindowedTextView({ projectId, path, size }: { projectId: string; path: string; size: number }) {
  const [state, setState] = React.useState<WindowState>({ status: "loading", offset: 0 });
  const [dragPercent, setDragPercent] = React.useState<number | null>(null);
  const request = React.useRef(0);
  const seekTimer = React.useRef<number | null>(null);

  const load = React.useCallback(
    (offset: number) => {
      // A button pressed while a drag is settling wins: the drag's read would
      // otherwise land after it and move the view back.
      if (seekTimer.current !== null) {
        window.clearTimeout(seekTimer.current);
        seekTimer.current = null;
        setDragPercent(null);
      }
      const current = ++request.current;
      setState({ status: "loading", offset });
      readWindow(projectId, path, size, offset).then(
        (window) => {
          if (current === request.current) setState({ status: "ready", window });
        },
        (cause: unknown) => {
          if (current === request.current) {
            setState({
              status: "error",
              offset,
              message: describeProjectError(cause, "This part of the file could not be read."),
            });
          }
        },
      );
    },
    [path, projectId, size],
  );

  React.useEffect(() => {
    load(0);
    return () => {
      request.current += 1;
      if (seekTimer.current !== null) window.clearTimeout(seekTimer.current);
    };
  }, [load]);

  const windowBytes = PROJECT_FILE_LIMITS.windowBytes;
  const shown = state.status === "ready" ? state.window : null;
  // Measured once per window, not on every drag of the slider.
  const longest = React.useMemo(() => (shown === null ? 0 : longestLine(shown.lines)), [shown]);
  const position = shown?.start ?? (state.status === "ready" ? 0 : state.offset);
  const lastStart = Math.max(0, size - windowBytes);
  // The slider's scale is the one `seek` reads back: 100% is the last window,
  // so a drag to the end stays at the end once that window arrives.
  const percent = lastStart === 0 ? 100 : Math.min(100, Math.round((position / lastStart) * 100));

  // A drag fires a change per pixel; only where it settles is read.
  const seek = (next: number) => {
    setDragPercent(next);
    if (seekTimer.current !== null) window.clearTimeout(seekTimer.current);
    seekTimer.current = window.setTimeout(() => {
      seekTimer.current = null;
      setDragPercent(null);
      load(Math.floor((next / 100) * lastStart));
    }, 150);
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="type-caption text-muted" role="status">
          {shown
            ? `Bytes ${shown.start.toLocaleString()}–${shown.end.toLocaleString()} of ${size.toLocaleString()} (${percent}%)`
            : `Reading from byte ${position.toLocaleString()}…`}
        </span>
        <div className="flex items-center gap-1">
          <NavButton disabled={position === 0} label="First page" onClick={() => load(0)}>
            <ChevronFirst aria-hidden size={14} />
          </NavButton>
          <NavButton
            disabled={position === 0}
            label="Previous page"
            onClick={() => load(Math.max(0, position - windowBytes))}
          >
            <ChevronLeft aria-hidden size={14} />
          </NavButton>
          <NavButton
            disabled={shown === null || shown.end >= size}
            label="Next page"
            onClick={() => shown && load(shown.end)}
          >
            <ChevronRight aria-hidden size={14} />
          </NavButton>
          <NavButton disabled={shown !== null && shown.end >= size} label="Last page" onClick={() => load(lastStart)}>
            <ChevronLast aria-hidden size={14} />
          </NavButton>
        </div>
      </div>
      <input
        aria-label="Position in file"
        aria-valuetext={`${percent}% of ${formatBytes(size)}`}
        className="w-full accent-accent"
        max={100}
        min={0}
        onChange={(event) => seek(Number(event.currentTarget.value))}
        step={1}
        type="range"
        value={dragPercent ?? percent}
      />
      <p className="type-caption text-muted">
        This file is {formatBytes(size)}, so it is shown {formatBytes(windowBytes)} at a time and cannot be edited here.
      </p>
      {shown?.replaced ? (
        <p className="type-caption text-muted">Some bytes here are not valid UTF-8 and show as �.</p>
      ) : null}
      {state.status === "error" ? (
        <Alert title="This part was not read" tone="danger">
          <p>{state.message}</p>
          <Button className="mt-2" onClick={() => load(state.offset)} size="sm" variant="secondary">
            Try again
          </Button>
        </Alert>
      ) : null}
      {shown ? (
        <VirtualLines
          count={shown.lines.length}
          firstLineNumber={null}
          // A new window starts at its top, not at the scroll offset the last
          // one was left at.
          key={shown.start}
          label={`${path}, bytes ${shown.start.toLocaleString()} to ${shown.end.toLocaleString()}`}
          maxLineColumns={longest}
          renderLine={(index) => displayLine(shown.lines[index] ?? "")}
        />
      ) : state.status === "loading" ? (
        <p className="type-body flex items-center gap-2 text-secondary">
          <Spinner decorative size={16} />
          Reading…
        </p>
      ) : null}
    </div>
  );
}

function NavButton({
  label,
  disabled,
  onClick,
  children,
}: {
  label: string;
  disabled: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <Tooltip label={label}>
      <IconButton aria-label={label} disabled={disabled} onClick={onClick} size="sm">
        {children}
      </IconButton>
    </Tooltip>
  );
}
