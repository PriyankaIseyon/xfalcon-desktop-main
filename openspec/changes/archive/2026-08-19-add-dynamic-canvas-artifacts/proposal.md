# Dynamic Canvas: Interactive Spreadsheets, Charts & HTML Artifacts

## Why

Currently, analytical outputs (tables, charts, and generated HTML dashboards) are embedded directly within chat message bubbles. As messages scroll, data analysis outputs get buried, and users cannot easily sort, filter, or interact with structured tables or visualize data dynamically.

The existing `CanvasSlot.tsx` is a placeholder. Building a dedicated, responsive sidecar Canvas allows users to inspect, sort, and visualize datasets, interact with rendered visual charts (ECharts / Chart.js), and inspect live HTML/Tailwind previews in an isolated, secure sandbox alongside their conversation.

## What Changes

- **Dedicated Sidecar Canvas Surface:** Extends the right-hand panel of the workspace into an expandable Canvas slot with tabbed artifact navigation (Data Table, Chart, Document/HTML).
- **Interactive Data Grid:** A high-performance, virtualized data grid for tabular data (CSV / JSON data produced by the model or MCP tools) supporting column sorting, text filtering, and copy-to-clipboard.
- **Visual BI Charting Engine:** Native rendering for structured chart specs (bar, line, scatter, pie, heatmaps via ECharts) with responsive resize, zoom, and theme alignment (dark/light).
- **Isolated Sandbox HTML Renderer:** Reuses the secure `xf-preview` custom scheme to render HTML artifacts in sandboxed iframes without exposing app tokens or origins.
- **Message-to-Canvas Linking:** Inline artifact chips within assistant messages that automatically focus and reveal the corresponding artifact in the Canvas when clicked.

## Capabilities Affected

- `dynamic-canvas-artifacts` (new capability)
