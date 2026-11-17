import React, { Suspense } from "react";
import { DialogShell } from "../../ui/DialogShell";
import { SettingsButton } from "./SettingsControls";
import "./VersionThanks.css";

const GradientASCII = React.lazy(() =>
  import("../delight/GradientASCII").then(({ GradientASCII: Component }) => ({ default: Component })),
);

interface VersionThanksProps {
  version: string;
  onClose: () => void;
}

export function VersionThanks({ version, onClose }: VersionThanksProps) {
  const titleId = React.useId();
  const descriptionId = React.useId();

  return (
    <DialogShell
      open
      onDismiss={onClose}
      dismissOnScrim={false}
      labelledBy={titleId}
      describedBy={descriptionId}
      className="xf-material xf-overlay-pop version-thanks__panel"
    >
      <Suspense fallback={null}>
        <GradientASCII className="version-thanks__gradient" />
      </Suspense>
      <div className="version-thanks__content">
        <p className="version-thanks__eyebrow type-label">xFalcon Desktop v{version}</p>
        <h2 id={titleId} className="version-thanks__title">
          Thank you for being here.
        </h2>
        <p id={descriptionId} className="version-thanks__copy">
          Built with care by the xFalcon team.
        </p>
        <SettingsButton data-dialog-initial-focus onClick={onClose}>
          Close
        </SettingsButton>
      </div>
    </DialogShell>
  );
}
