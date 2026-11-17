# Dashboard Feature Roadmap

Version 0.1 · 2026-08-25

## Goal

Make xFalcon Desktop provide the rich dashboard workflow shown in the reference video.

The work must extend the current product. It must not rewrite the existing chat, canvas, reports, datasets, or agent systems.

## Existing foundation

xFalcon Desktop already provides:

- Chat with streaming responses, reasoning, and tool activity.
- A bounded agent tool loop with MCP support.
- Dataset upload, preview, schema inspection, and cleanup recipes.
- Canvas artifacts for tables, charts, HTML, and SVG.
- Five chart types: bar, line, area, scatter, and pie.
- Table sorting, filtering, CSV export, and virtual scrolling.
- Saved reports with search, pin, rename, delete, and reopen actions.
- Native PDF, CSV, SVG, and document export paths.
- Approval-gated changes and revision restore.

## 1. First-class Dashboards workspace

Add a new **Dashboards** destination to the main workspace.

Each dashboard should be a structured asset that contains:

- A name and description.
- KPI, chart, table, and document widgets.
- Widget positions and sizes.
- Dataset and query references.
- Filter definitions.
- Refresh state and last-refresh time.
- Source conversation and message references where available.

The dashboard library should support:

- Search.
- Pin and unpin.
- Rename.
- Duplicate.
- Delete with confirmation.
- Recent and pinned sections.
- Reopen after an application restart.

### Reuse

- Reuse the existing workspace destination pattern.
- Reuse Canvas table, chart, and document renderers.
- Reuse the Reports list patterns for search, pin, rename, and delete.
- Reuse encrypted local persistence and account cleanup controls (reports themselves keep no local copy).

### Boundary

Do not replace Reports or Canvas.

- Canvas remains temporary exploration beside a conversation.
- Reports are DA report kits: the assistant saves them with DA's tools, and the Reports page lists, opens, and refreshes them. They are refreshed by re-running stored SQL on DA, and they keep no local copy.
- Dashboards become structured, editable, and refreshable assets.

## 2. Prompt-to-dashboard builder

Add a guided dashboard creation workflow.

The user should be able to:

1. Select one or more datasets or business topics.
2. Describe the required metrics, charts, tables, and filters.
3. Review the proposed dashboard plan.
4. Start generation.
5. Watch the generation progress.
6. Review and save the generated dashboard.

Show clear generation phases:

1. Analyze schema.
2. Build queries.
3. Fetch data.
4. Build dashboard.

The progress view should show useful facts such as:

- Tables and columns inspected.
- Queries created and validated.
- Datasets produced.
- Rows processed.
- Widgets created.
- Failures that require user action.

### Reuse

- Reuse the existing chat composer patterns.
- Reuse dataset and project binding.
- Reuse the current bounded agent runtime.
- Reuse MCP tools for data access.
- Reuse the live run timeline and tool activity presentation.

### Boundary

Do not create a separate agent framework or require multi-agent support.

The dashboard builder should be a new workflow over the current agent runtime.

## 3. Dashboard edit mode and interactions

Add an explicit **Edit mode** to a saved dashboard.

Edit mode should support:

- Drag and reorder widgets.
- Resize widgets.
- Add, duplicate, and delete widgets.
- Change a widget title and description.
- Change a chart type.
- Select dimensions and measures.
- Configure aggregation and formatting.
- Configure colours, icons, and legends.
- Add KPI cards.
- Add or remove dashboard sections.

Reader mode should support:

- Global date filters.
- Dataset and column filters.
- Cross-filtering between widgets.
- Drill-down into a selected value.
- Per-widget refresh.
- Full-dashboard refresh.
- Visible last-refresh status.
- CSV, SVG, PDF, and document export where applicable.

Store dashboard edits as small revisions. Provide Undo for recent changes.

### Reuse

- Reuse Canvas chart and table renderers.
- Reuse the existing five chart types.
- Reuse table sorting, filtering, and export.
- Reuse the native document save path.
- Reuse existing revision and restore concepts.

### Boundary

Do not rewrite the current Canvas implementation into a dashboard editor.

Extract or extend only the renderer behaviour that the new dashboard surface needs.

## 4. Dashboard-scoped Analyst panel

Add an Analyst panel that opens beside a dashboard.

The panel should understand the current context, including:

- The active dashboard.
- The selected widget.
- The widget query.
- Active filters.
- A selected dataset, table, or column.
- The dashboard revision being viewed.

Example requests include:

- "Fix this chart."
- "Make the chart wider."
- "Change this to weekly data."
- "Add a region filter."
- "Explain this spike."
- "Replace this KPI with gross margin."

The panel should:

- Show the original request.
- Show agent progress and tool steps.
- Show which dashboard items will change.
- Present a proposed change set before Apply.
- Apply the approved changes as one revision.
- Provide Undo after the change.
- Explain failures without leaving a partial dashboard update.

### Reuse

- Reuse chat streaming and reasoning presentation.
- Reuse the live tool activity components.
- Reuse the existing agent runtime and MCP sessions.
- Reuse approval and revision patterns.

### Boundary

The Analyst panel is a dashboard-scoped use of the current agent.

It is not a new multi-agent system.

## 5. Query lineage and data-quality workspace

Add inspectable **Queries** and **Data quality** views to each dashboard.

### Query view

For each query, show:

- Query name and purpose.
- SQL or provider query text when available.
- Bound parameters.
- Source datasets, tables, and columns.
- Widgets that consume the query.
- Execution time.
- Row count.
- Cache status when available.
- Validation warnings and errors.
- Last execution time.

Allow the user to rerun a query and inspect its latest result.

Show an execution plan only when the data provider supports it. Do not make this a requirement for every MCP server.

### Data-quality view

For each dataset and column, show available profiles such as:

- Inferred data type.
- Row and column counts.
- Null percentage.
- Distinct value count.
- Minimum and maximum values.
- Mean, median, and standard deviation for numeric columns.
- Example values.
- Potential outliers.
- Duplicate risks.
- Possible filter dimensions.
- Data-quality warnings.

Allow the Analyst panel to explain these results and suggest cleanup or query changes.

### Reuse

- Reuse the existing dataset catalog and schema drawer.
- Reuse dataset preview and cleanup recipes.
- Reuse MCP data access.
- Reuse Canvas tables for result previews.

### Boundary

Query details are provider capabilities. The UI must show when a detail is unavailable instead of inventing a value.

## Recommended delivery order

1. Add the structured dashboard contract, persistence, and library.
2. Save existing Canvas artifacts into a dashboard and reopen them.
3. Add layout editing, one global date filter, and refresh.
4. Add the prompt-to-dashboard builder.
5. Add the dashboard-scoped Analyst panel.
6. Add query lineage and data-quality details incrementally.

## First release slice

The first release should prove the complete dashboard path with a small scope:

1. Create a dashboard from an existing conversation.
2. Add one chart and one table from the Canvas.
3. Save the dashboard.
4. Reopen it after an application restart.
5. Drag and resize its widgets.
6. Apply one date filter.
7. Refresh its data.
8. Export the dashboard.

This slice establishes the durable dashboard model without changing the existing Chat or Canvas contracts, and builds on the Reports contract as it stands (DA report kits).

## Not part of this roadmap

The following additions do not materially close the gap shown in the reference video:

- Multi-agent or subagent execution.
- Shell or process execution.
- General web search.
- A per-conversation model picker.
- Voice, mobile, sharing, or collaboration.

These items should remain outside this dashboard work unless a separate requirement approves them.
