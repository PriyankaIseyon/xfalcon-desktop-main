import React from "react";
import ReactDOM from "react-dom/client";
import { AppErrorBoundary } from "./app/AppErrorBoundary";
import { installAppearance } from "./app/appearance";
import { BrokenSurface } from "./devStartupVisualBrokenSurface";

void installAppearance();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <AppErrorBoundary>
      <BrokenSurface />
    </AppErrorBoundary>
  </React.StrictMode>,
);
