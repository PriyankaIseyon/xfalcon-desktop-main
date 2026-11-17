import { Skeleton } from "../../ui/Skeleton";

/**
 * Full-shell loading state between sign-in and workspace readiness
 * (spec: workspace-arrival). The ghosts hold the same large regions as the
 * final shell, so the reveal changes content rather than moving its frame.
 * Shimmer is CSS-only and goes static under `prefers-reduced-motion`.
 */
export function WorkspaceSkeleton({ inert = false }: { inert?: boolean }) {
  return (
    <div aria-live="polite" className="workspace-skeleton" inert={inert} role="status">
      <div aria-hidden className="workspace-skeleton__sidebar">
        <div className="workspace-skeleton__brand">
          <Skeleton className="workspace-skeleton__mark" />
          <Skeleton className="workspace-skeleton__wordmark" />
        </div>
        <div className="workspace-skeleton__destinations">
          {Array.from({ length: 4 }, (_, index) => (
            <div className="workspace-skeleton__destination" key={index}>
              <Skeleton className="workspace-skeleton__icon" />
              <Skeleton className="workspace-skeleton__destination-label" />
            </div>
          ))}
        </div>
        <Skeleton className="workspace-skeleton__search" />
        <Skeleton className="workspace-skeleton__row workspace-skeleton__row--action" />
        <Skeleton className="workspace-skeleton__section-label" />
        {Array.from({ length: 3 }, (_, index) => (
          <Skeleton className="workspace-skeleton__row" key={index} />
        ))}
        <span className="workspace-skeleton__spacer" />
        <Skeleton className="workspace-skeleton__row workspace-skeleton__row--utility" />
        <Skeleton className="workspace-skeleton__account" />
      </div>
      <div className="workspace-skeleton__main">
        <div aria-hidden className="workspace-skeleton__titlebar">
          <Skeleton className="workspace-skeleton__titlebar-button" />
          <Skeleton className="workspace-skeleton__title" />
          <Skeleton className="workspace-skeleton__titlebar-action" />
        </div>
        <div className="workspace-skeleton__stage">
          <div className="workspace-skeleton__hero">
            <Skeleton className="workspace-skeleton__hero-mark" />
            <p className="workspace-skeleton__label type-display">Loading workspace…</p>
            <div aria-hidden className="workspace-skeleton__welcome">
              <Skeleton className="workspace-skeleton__composer" />
              <div className="workspace-skeleton__suggestions">
                {Array.from({ length: 4 }, (_, index) => (
                  <Skeleton className="workspace-skeleton__suggestion" key={index} />
                ))}
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
