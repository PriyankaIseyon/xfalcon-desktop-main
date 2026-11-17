import React from "react";
import ReactDOM from "react-dom/client";
import { setPlatform } from "./lib/platform";
import { Gallery } from "./devPrimitiveGallery";

/** The mount for `primitives.html`, kept apart from the specimens so the
 * gallery module exports components only and Fast Refresh works while a
 * specimen is being edited. */
setPlatform("windows");
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Gallery />
  </React.StrictMode>,
);
