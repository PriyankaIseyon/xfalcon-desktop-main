import { createContext, type ReactNode } from "react";

/**
 * Actions a host adds to the header of the card it renders — the project file
 * view's "Reload file" and "Edit", which act on the same file the card shows,
 * so they belong in the card's one action row rather than in a second row
 * above it. Read by the outermost card only: `ArtifactFrame` clears it for
 * everything inside, so a fence nested in a Markdown file gets none.
 */
export const ArtifactHostActions = createContext<ReactNode>(null);
