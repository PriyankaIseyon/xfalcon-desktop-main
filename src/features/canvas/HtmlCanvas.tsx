import { useCallback, useEffect, useRef, useState } from "react";
import { Copy, Download, RotateCw } from "lucide-react";
import { Button } from "../../ui/Button";
import { Skeleton } from "../../ui/Skeleton";
import { documentExportSave, previewDocumentRegister, previewDocumentSrc } from "../../lib/native";
import { toast } from "../../ui/toastStore";
import { writeClipboardText } from "../chat/clipboard";
import type { HtmlArtifact } from "./canvasStore";

export interface HtmlCanvasProps {
  artifact: HtmlArtifact;
}

type FrameState = { status: "registering" } | { status: "ready"; src: string } | { status: "failed" };

/**
 * One HTML or SVG artifact, rendered from the `xf-preview` origin
 * (spec: dynamic-canvas-artifacts).
 *
 * The frame fills the panel at whatever width the panel has. There is no
 * Desktop/Tablet/Mobile picker: it framed a question nobody reading their own
 * dashboard asks, and simulating a phone inside a desktop window tells you
 * about the simulation, not the document. Panel width is the size control, and
 * the header's expand toggle is how it gets bigger — the same options
 * xfalcon-web's canvas offers.
 *
 * Isolation contract, carried across from `reports/ReportViewer.tsx` and
 * `chat/richContent/cards.tsx` so the reason travels with the third consumer:
 * the document loads from its own non-app origin under a per-response CSP that
 * allows its inline styles and scripts while denying ALL network
 * (spec: preview-protocol). `sandbox="allow-scripts"` and NEVER
 * `allow-same-origin` — even on its own origin the document gets no DOM,
 * storage, or Tauri IPC reach into the app — and `referrerPolicy="no-referrer"`
 * so a frame that somehow reached the network could not name where it came from.
 *
 * The document is re-registered on every mount and the id is never reused from
 * the artifact, for the reason `ReportViewer` states: the preview store is
 * in-memory, bounded, and evicts least-recently-served, so an id is not durable
 * and is gone after a restart. The frame cannot report that failure — the
 * scheme answers an unknown id with a 404 whose body the frame LOADS, so
 * `onError` never fires and `onLoad` does — which is why a canvas tab that
 * trusted its stored id would come back blank with no explanation.
 */
export function HtmlCanvas({ artifact }: HtmlCanvasProps) {
  const [state, setState] = useState<FrameState>({ status: "registering" });
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  /** Bumped to force a fresh frame element on an explicit reload. */
  const [attempt, setAttempt] = useState(0);

  const register = useCallback(
    async (live: () => boolean) => {
      setLoaded(false);
      setState({ status: "registering" });
      for (const round of [0, 1]) {
        try {
          const documentId = await previewDocumentRegister(artifact.kind, artifact.content);
          if (!live()) return;
          if (documentId.trim() === "") throw new Error("Preview registration returned no document id");
          setState({ status: "ready", src: previewDocumentSrc(documentId) });
          return;
        } catch {
          if (!live()) return;
          // Only the second failure is worth showing: the first is far more
          // likely a transient register than a lost document.
          if (round === 1) setState({ status: "failed" });
        }
      }
    },
    [artifact.content, artifact.kind],
  );

  useEffect(() => {
    let live = true;
    // Every `setState` inside is gated on `live`, so switching tabs mid-register
    // cannot land a resolved id on the tab that replaced this one.
    void register(() => live);
    return () => {
      live = false;
    };
  }, [register, attempt]);

  const copySource = async () => {
    try {
      await writeClipboardText(artifact.content);
      toast.success("Source copied to clipboard.");
    } catch {
      toast.error("The source could not be copied.");
    }
  };

  const saveSource = async () => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    try {
      const format = artifact.kind === "svg" ? "svg" : "html";
      const result = await documentExportSave(artifact.title, artifact.content, format);
      // A dismissed dialog is an answer, not a failure — say nothing.
      if (result.saved) toast.success(`Saved ${result.file_name ?? "the file"}.`);
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : "The document could not be saved.");
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full w-full flex-col overflow-hidden bg-card text-foreground">
      <div className="canvas-toolbar canvas-toolbar--end shrink-0">
        <Button
          aria-label={artifact.kind === "svg" ? "Copy SVG source" : "Copy HTML source"}
          onClick={() => void copySource()}
          size="sm"
          variant="secondary"
        >
          <Copy aria-hidden size={14} strokeWidth={1.8} />
          Copy
        </Button>
        <Button
          aria-label={artifact.kind === "svg" ? "Save file as SVG" : "Save file as HTML"}
          loading={busy}
          onClick={() => void saveSource()}
          size="sm"
          variant="secondary"
        >
          <Download aria-hidden size={14} strokeWidth={1.8} />
          Save file
        </Button>
        <Button
          disabled={state.status === "registering"}
          onClick={() => setAttempt((count) => count + 1)}
          size="sm"
          variant="secondary"
        >
          <RotateCw aria-hidden size={14} strokeWidth={1.8} />
          Reload
        </Button>
      </div>

      <div className="min-h-0 flex-1 bg-surface/30">
        {state.status === "registering" ? <Skeleton className="h-full w-full" /> : null}

        {state.status === "failed" ? (
          <div className="flex h-full w-full flex-col items-center justify-center gap-2 text-center">
            <p className="text-sm text-foreground">This document could not be opened.</p>
            <Button onClick={() => setAttempt((count) => count + 1)} type="button" variant="secondary">
              Try again
            </Button>
          </div>
        ) : null}

        {state.status === "ready" ? (
          <iframe
            aria-busy={!loaded}
            className={`block h-full w-full border-0 bg-card transition-opacity ${loaded ? "opacity-100" : "opacity-0"}`}
            inert={!loaded}
            key={attempt}
            referrerPolicy="no-referrer"
            sandbox="allow-scripts"
            src={state.src}
            title={artifact.title}
            onLoad={() => setLoaded(true)}
          />
        ) : null}
      </div>
    </div>
  );
}
