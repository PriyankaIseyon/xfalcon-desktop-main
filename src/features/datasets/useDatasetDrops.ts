import { useEffect, useRef } from "react";
import { onDatasetDropped } from "../../lib/native";
import { applyDroppedRefusal, enqueueDroppedDataset } from "./datasetsStore";

/**
 * A file dropped on the window, wherever the person happens to be.
 *
 * Installed by the shell rather than by the Datasets destination, because the
 * drop is handled in the backend: it stages the file whatever is on screen, and
 * a listener that only existed while the destination was showing would leave
 * that staged file with nothing to claim it — invisible, and holding one of the
 * eight staging slots until the app exits.
 *
 * So a staged drop routes: `onStaged` takes the person to the destination,
 * which then opens the confirmation step for each staged file in turn. A
 * *refused* drop does not route, and is reported only when the destination is
 * already showing — the window accepts drops for other reasons too, and a skill
 * file landing on the chat must not raise "that is not a dataset".
 */
export function useDatasetDrops(showing: boolean, onStaged: () => void) {
  const showingRef = useRef(showing);
  showingRef.current = showing;
  const onStagedRef = useRef(onStaged);
  onStagedRef.current = onStaged;

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void onDatasetDropped((outcome) => {
      if (outcome.staged !== null) {
        enqueueDroppedDataset(outcome.staged);
        onStagedRef.current();
        return;
      }
      if (outcome.error !== null && showingRef.current) {
        applyDroppedRefusal({ error: outcome.error.code, stepIndex: outcome.error.step_index });
      }
    })
      .then((stop) => {
        if (cancelled) {
          void stop();
          return;
        }
        unlisten = stop;
      })
      // A listener that cannot be installed costs the drop affordance, not the
      // destination: the picker still works, and the catalog still lists.
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
}
