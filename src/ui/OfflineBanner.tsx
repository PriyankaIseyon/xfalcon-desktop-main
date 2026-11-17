import { Alert } from "./Alert";

export interface OfflineBannerProps {
  visible?: boolean;
}

export function OfflineBanner({ visible }: OfflineBannerProps) {
  const offline = visible ?? (typeof navigator !== "undefined" ? navigator.onLine === false : false);
  if (!offline) return null;
  return (
    <Alert className="rounded-none border-x-0 border-t-0" tone="warn" title="You appear to be offline">
      Actions that need the secure runtime or network may not complete until connectivity returns.
    </Alert>
  );
}
