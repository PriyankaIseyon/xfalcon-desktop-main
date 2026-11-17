import { useCallback, useEffect, useState, type Dispatch, type SetStateAction } from "react";

export const APP_NARROW_QUERY = "screen and (max-width: 900px)";

export interface UseAppLayoutOptions {
  narrowQuery?: string;
}

export interface AppLayoutState {
  isNarrow: boolean;
  sidebarOpen: boolean;
  setSidebarOpen: Dispatch<SetStateAction<boolean>>;
  toggleSidebar: () => void;
}

function matchesQuery(query: string): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia(query).matches
    : false;
}

export function useAppLayout({ narrowQuery = APP_NARROW_QUERY }: UseAppLayoutOptions = {}): AppLayoutState {
  const [isNarrow, setIsNarrow] = useState(() => matchesQuery(narrowQuery));
  // A desktop rail and a narrow drawer are different choices. A resize must
  // not turn the drawer's required closed state into a permanent desktop rail.
  const [wideSidebarOpen, setWideSidebarOpen] = useState(true);
  const [narrowSidebarOpen, setNarrowSidebarOpen] = useState(false);

  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const media = window.matchMedia(narrowQuery);
    const apply = (matches: boolean): void => {
      setIsNarrow(matches);
    };
    const handleChange = (event: MediaQueryListEvent): void => apply(event.matches);

    apply(media.matches);
    media.addEventListener("change", handleChange);
    return () => media.removeEventListener("change", handleChange);
  }, [narrowQuery]);

  const setSidebarOpen = useCallback<Dispatch<SetStateAction<boolean>>>(
    (next) => {
      const setCurrent = isNarrow ? setNarrowSidebarOpen : setWideSidebarOpen;
      setCurrent((current) => (typeof next === "function" ? next(current) : next));
    },
    [isNarrow],
  );

  const toggleSidebar = useCallback(() => {
    setSidebarOpen((current) => !current);
  }, [setSidebarOpen]);

  return {
    isNarrow,
    sidebarOpen: isNarrow ? narrowSidebarOpen : wideSidebarOpen,
    setSidebarOpen,
    toggleSidebar,
  };
}
