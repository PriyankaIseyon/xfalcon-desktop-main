import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App.tsx";
import { AppErrorBoundary } from "./app/AppErrorBoundary";
import { installAppearance } from "./app/appearance";
import { reportFontActivation } from "./app/typography";

void installAppearance();
reportFontActivation();
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <AppErrorBoundary>
      <App />
    </AppErrorBoundary>
  </React.StrictMode>
);
