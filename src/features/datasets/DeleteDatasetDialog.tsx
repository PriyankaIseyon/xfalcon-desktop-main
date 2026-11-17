import { useEffect, useRef, useState } from "react";
import { Trash2 } from "lucide-react";
import type { DatasetView } from "../../lib/generated/contracts";
import { Button } from "../../ui/Button";
import { Alert } from "../../ui/Alert";
import { Field, Input } from "../../ui/Field";
import { Modal } from "../../ui/Modal";

/**
 * Type-to-confirm delete.
 *
 * The name has to be typed because the rows are scanned quickly and the action
 * is on every one of them: a mis-aimed click would otherwise remove a dataset
 * and everything the workspace derived from it, with nothing to undo it.
 */
export interface DeleteDatasetDialogProps {
  dataset: DatasetView | null;
  onCancel: () => void;
  /** Null after deletion, or safe retained copy for the retry surface. */
  onConfirm: (dataset: DatasetView) => Promise<string | null>;
}

export function DeleteDatasetDialog({ dataset, onCancel, onConfirm }: DeleteDatasetDialogProps) {
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const busyRef = useRef(false);

  // Cleared when the dialog closes, so the next open starts blank rather than
  // pre-confirmed with the previous dataset's name.
  useEffect(() => {
    setTyped("");
    setBusy(false);
    setProblem(null);
    busyRef.current = false;
  }, [dataset?.id]);

  if (dataset === null) return null;
  const matches = typed === dataset.display_name;

  return (
    <Modal
      open
      size="sm"
      role="alertdialog"
      dismissible={!busy}
      onClose={onCancel}
      title={`Delete “${dataset.display_name}”?`}
      footer={
        <>
          <Button type="button" variant="secondary" onClick={onCancel} disabled={busy}>
            Keep dataset
          </Button>
          <Button
            type="button"
            variant="destructive"
            disabled={!matches}
            loading={busy}
            onClick={() => {
              if (busyRef.current) return;
              busyRef.current = true;
              setBusy(true);
              setProblem(null);
              void onConfirm(dataset)
                .then((nextProblem) => {
                  if (nextProblem !== null) setProblem(nextProblem);
                })
                .finally(() => {
                  busyRef.current = false;
                  setBusy(false);
                });
            }}
          >
            <Trash2 aria-hidden size={14} />
            {busy ? "Deleting…" : "Delete dataset"}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        {problem !== null ? <Alert tone="danger">{problem}</Alert> : null}
        <p className="type-body-sm">
          This removes <strong>{dataset.display_name}</strong> and every table the workspace built from it. It cannot be
          undone.
        </p>
        <Field label={`Type "${dataset.display_name}" to confirm`} htmlFor="delete-dataset-name">
          <Input
            id="delete-dataset-name"
            autoFocus
            autoComplete="off"
            value={typed}
            onChange={(event) => setTyped(event.currentTarget.value)}
          />
        </Field>
      </div>
    </Modal>
  );
}
