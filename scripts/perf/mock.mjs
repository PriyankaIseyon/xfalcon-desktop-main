/* global window -- the page.evaluate and init-script code runs in Chromium */
// Generic perf mock for the Tauri native boundary. Installed with page.addInitScript.
// Returns plausible fixtures for every command the shell calls; unknown commands are logged and resolve to null.
// Mock options and their defaults. profile.mjs rejects a --cfg key that is not in this list.
export const MOCK_DEFAULTS = {
  conversations: 300,
  projects: 24,
  reports: 200,
  datasets: 200,
  dirEntries: 4000,
  messages: 0,
  ipcDelay: 0,
  ipcJitter: 0,
  csvRows: 20000,
  heavy: false,
  activeRunId: null,
};

export function installPerfMock(page, cfg = {}) {
  const options = { ...MOCK_DEFAULTS, ...cfg };
  return page.addInitScript((opts) => {
    const listeners = new Map();
    const callbacks = new Map();
    const unknown = new Set();
    const invocations = [];
    const now = Date.now();
    const iso = (offsetMin) => new Date(now - offsetMin * 60_000).toISOString();

    const conversations = Array.from({ length: opts.conversations }, (_, i) => ({
      version: 1,
      conversation_id: `conv-${i}`,
      title: `Quarterly revenue analysis number ${i + 1}`,
      updated_at: iso(i * 7),
      project_id: i % 5 === 0 ? `project-${i % opts.projects}` : null,
      pinned_at: i < 3 ? iso(i) : null,
    }));
    const projects = Array.from({ length: opts.projects }, (_, i) => ({
      version: 1,
      id: `project-${i}`,
      display_name: `Research project ${i + 1}`,
      root_path: `C:\\work\\research-${i + 1}`,
      status: "trusted",
      active: i === 0,
      trusted_at_ms: now - i * 1000,
    }));
    const reports = Array.from({ length: opts.reports }, (_, i) => ({
      version: 1,
      id: `report-${i}`,
      kind: "dashboard",
      title: `Revenue dashboard ${i + 1}`,
      description: `Revenue, retention, and pipeline summary for business unit ${i + 1}.`,
      created_at_ms: BigInt(now - i * 86_400_000),
      updated_at_ms: BigInt(now - i * 3_600_000),
      thumbnail_updated_at_ms: null,
      data_sources: [],
      pinned: i < 3,
      hidden: false,
      byte_size: BigInt(84_000 + i),
      scope: "user",
      is_owner: true,
      owner_display_name: null,
      refreshable: true,
      can_refresh: true,
      can_manage: true,
      can_delete: true,
      page_count: 1,
      dataset_count: 1,
      source_conversation_id: null,
    }));
    const datasets = Array.from({ length: opts.datasets }, (_, i) => ({
      version: 1,
      id: `dataset-${i}`,
      display_name: `sales_2026_${String(i + 1).padStart(3, "0")}.csv`,
      status: "ready",
      created_at_ms: BigInt(now - i * 86_400_000),
      updated_at_ms: BigInt(now - i * 3_600_000),
    }));
    // Deterministic project tree: root has 12 folders and 18 files; folders above depth 3 hold 6 sub-folders and
    // 40 files; "data" holds opts.dirEntries files (a large flat directory, as in a real data project).
    const listing = (path) => {
      const depth = path === "" ? 0 : path.split("/").length;
      const leaf = path.split("/").pop();
      const entry = (name, kind, i) => ({
        name,
        kind,
        size_bytes: kind === "directory" ? 0 : 1200 + i,
        modified_ms: now - i * 1000,
        is_sensitive: false,
      });
      if (leaf === "data")
        return Array.from({ length: opts.dirEntries }, (_, i) =>
          entry(`part-${String(i).padStart(5, "0")}.csv`, "file", i),
        );
      const dirs =
        depth === 0
          ? [
              "src",
              "data",
              "docs",
              "tests",
              "scripts",
              "assets",
              "config",
              "notebooks",
              "reports",
              "vendor",
              "build",
              "tmp",
            ]
          : depth < 3
            ? Array.from({ length: 6 }, (_, i) => `module-${i + 1}`)
            : [];
      const files = depth === 0 ? 18 : depth < 3 ? 40 : 30;
      return [
        ...dirs.map((n, i) => entry(n, "directory", i)),
        ...Array.from({ length: files }, (_, i) =>
          entry(`file-${i + 1}.${i % 3 === 0 ? "ts" : i % 3 === 1 ? "md" : "json"}`, "file", i),
        ),
      ];
    };
    const heavyAnswer = (i) => {
      const para = (k) =>
        `Paragraph ${k} of answer ${i}: **revenue** rose while *margin* fell, see \`region\` and [the dashboard](https://example.com/${k}). `.repeat(
          3,
        );
      const code =
        "```sql\nSELECT region, SUM(revenue) AS revenue, AVG(margin) AS margin\nFROM sales\nWHERE quarter = 'Q3'\nGROUP BY region\nORDER BY revenue DESC;\n```";
      const py =
        "```python\nimport pandas as pd\ndf = pd.read_csv('sales.csv')\nprint(df.groupby('region').revenue.sum())\n```";
      const rows = Array.from({ length: 12 }, (_, r) => `| Region ${r} | ${r * 100} | ${r / 10} |`).join("\n");
      const table = `| region | revenue | margin |\n|---|---|---|\n${rows}`;
      return [
        para(1),
        para(2),
        code,
        para(3),
        "- item one\n- item two\n- item three",
        table,
        py,
        para(4),
        para(5),
        para(6),
      ].join("\n\n");
    };
    const messages = Array.from({ length: opts.messages }, (_, i) =>
      opts.heavy && i % 2 === 1
        ? {
            version: 1,
            message_id: `m-${i}`,
            role: "assistant",
            text: heavyAnswer(i),
            sequence: BigInt(i + 1),
            tool_calls: [],
          }
        : {
            version: 1,
            message_id: `m-${i}`,
            role: i % 2 === 0 ? "user" : "assistant",
            text:
              i % 2 === 0
                ? `Question ${i}: how did revenue and margin move for the region last week?`
                : `Answer ${i}: **Revenue** rose 4.2% while margin fell 0.3 points.\n\n- North: up\n- South: flat\n\n\`\`\`sql\nSELECT region, SUM(revenue) FROM sales GROUP BY region;\n\`\`\`\n\nSee the table below.\n\n| region | revenue |\n|---|---|\n| North | 120 |\n| South | 95 |\n`,
            sequence: BigInt(i + 1),
            tool_calls: [],
          },
    );

    let csvCache = null;
    const csvBytes = () => {
      if (csvCache) return csvCache;
      const lines = ["region,product,units,revenue,margin,cost,channel,quarter"];
      for (let i = 0; i < (opts.csvRows ?? 20000); i++)
        lines.push(
          `North ${i % 7},Widget ${i % 13},${(i * 7) % 500},${((i * 37) % 10000) / 10},${(i % 40) / 100},${((i * 11) % 5000) / 10},online,Q${(i % 4) + 1}`,
        );
      csvCache = new TextEncoder().encode(lines.join("\n") + "\n");
      return csvCache;
    };

    function reg(cb, once = false) {
      const id = window.crypto.getRandomValues(new Uint32Array(1))[0];
      callbacks.set(id, (d) => {
        if (once) callbacks.delete(id);
        return cb(d);
      });
      return id;
    }
    const run = (id, d) => callbacks.get(id)?.(d);

    // A 1x1 PNG: the stored thumbnail a Reports card decodes. Its size is not the size of a real thumbnail.
    const ONE_PIXEL_PNG =
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    const bootstrap = {
      version: 1,
      application_version: "0.0.0-perf",
      platform: "windows",
      session: {
        status: "authenticated",
        version: 1,
        identity: { version: 1, user_id: "u", tenant_id: "t", display_name: "Perf User", email: null },
        expires_at_ms: now + 3_600_000,
      },
    };

    function invoke(cmd, args = {}) {
      if (opts.ipcDelay > 0 && !cmd.startsWith("plugin:event|")) {
        return new Promise((resolve, reject) =>
          setTimeout(
            () => invokeNow(cmd, args).then(resolve, reject),
            opts.ipcDelay + Math.floor((((invocations.length * 2654435761) % 1000) / 1000) * opts.ipcJitter),
          ),
        );
      }
      return invokeNow(cmd, args);
    }
    // Runs after invoke()'s simulated IPC delay, so t is when the command reached this mock backend, not when the app called it.
    function invokeNow(cmd, args = {}) {
      invocations.push({ cmd, t: performance.now() });
      if (cmd.startsWith("plugin:event|")) {
        if (cmd === "plugin:event|listen") {
          const arr = listeners.get(args.event) ?? [];
          arr.push(args.handler);
          listeners.set(args.event, arr);
          return Promise.resolve(args.handler);
        }
        return Promise.resolve(null);
      }
      switch (cmd) {
        case "app_bootstrap":
          return Promise.resolve(bootstrap);
        case "app_health":
          return Promise.resolve({ version: 1, status: "ready" });
        case "workspace_status":
          return Promise.resolve({
            version: 1,
            tenant_host: "tenant.example",
            mcp_resource_url: "https://tenant.example/mcp",
            mcp_servers: [],
            models_available: 1,
            default_model_id: "gpt-test",
            default_provider: "openai",
            chat_ready: true,
            setup_note: "",
          });
        case "settings_appearance_get":
          return Promise.resolve({
            version: 1,
            theme: "light",
            reading_size: "small",
            show_reasoning: false,
            show_follow_ups: true,
          });
        case "project_list":
          return Promise.resolve({ version: 1, projects });
        case "project_activate":
          return Promise.resolve({ ...projects[0], active: true });
        case "chat_workspace_bootstrap":
          return Promise.resolve({ version: 1, conversations, recovery_notice: null });
        case "chat_conversation_create":
          return Promise.resolve({
            version: 1,
            conversation_id: "conv-new",
            title: "New conversation",
            availability: "live",
            messages: [],
            active_run_id: null,
          });
        case "chat_message_send":
          return Promise.resolve({
            version: 1,
            status: "submitted",
            conversation_id: args.conversationId ?? "conv-new",
            run_id: "run-new",
            message: "ok",
          });
        case "chat_conversation_open":
          return Promise.resolve({
            version: 1,
            conversation_id: args.conversationId ?? "conv-0",
            title: "Perf conversation",
            availability: "live",
            messages,
            active_run_id: opts.activeRunId,
          });
        case "report_list":
          return Promise.resolve({ version: 1, reports, project_origins: [] });
        case "dataset_list":
          return Promise.resolve({ version: 1, datasets, uploads_enabled: true, truncated: false });
        case "upload_queue":
          return Promise.resolve({ version: 1, revision: 0, uploads: [] });
        case "skill_list":
          return Promise.resolve({ version: 1, skills: [], errors: [] });
        case "mcp_server_list":
          return Promise.resolve({ version: 1, servers: [] });
        case "credential_list":
          return Promise.resolve({ version: 1, credentials: [] });
        case "chat_custom_model_list":
          return Promise.resolve({ version: 1, models: [] });
        case "chat_model_catalog":
          return Promise.resolve({ version: 1, providers: [], models: [], default_model_id: null });
        case "chat_message_feedback_list":
          return Promise.resolve([]);
        case "project_list_directory":
          return Promise.resolve({
            version: 1,
            project_id: args.projectId,
            path: args.path ?? "",
            entries: listing(args.path ?? ""),
            truncated: false,
          });
        case "project_file_stat":
          return Promise.resolve({
            kind: "text",
            version: 1,
            path: args.path,
            size_bytes: String(args.path).endsWith(".csv") ? csvBytes().length : 40,
            modified_ms: now,
          });
        case "project_file_read_bytes": {
          const bytes = String(args.path).endsWith(".csv")
            ? csvBytes()
            : new TextEncoder().encode("export default 42;\n");
          const slice = bytes.slice(args.offset ?? 0, (args.offset ?? 0) + (args.length ?? bytes.length));
          return Promise.resolve(slice.buffer);
        }
        case "project_schema_summary":
          return Promise.resolve({ version: 1, project_id: args.projectId, tables: [] });
        case "update_check":
          return Promise.resolve({ version: 1, status: "up_to_date" });
        case "report_thumbnail":
          return Promise.resolve({ version: 1, id: args.reportId, format: "png", data_base64: ONE_PIXEL_PNG });
        case "chat_follow_ups":
          return Promise.resolve({ suggestions: ["Break this down by region", "What is driving this change?"] });
        case "delight_greeting_claim":
          return Promise.resolve(null);
        case "delight_message_sent":
          return Promise.resolve(0);
        default:
          unknown.add(cmd);
          return Promise.resolve(null);
      }
    }

    Object.defineProperty(window, "__TAURI_EVENT_PLUGIN_INTERNALS__", {
      configurable: true,
      writable: true,
      value: {
        unregisterListener(_e, id) {
          callbacks.delete(id);
        },
      },
    });
    Object.defineProperty(window, "__PERF_MOCK__", {
      configurable: true,
      value: {
        invocations,
        unknown,
        listeners,
        run,
        emit: (event, payload) => {
          for (const h of listeners.get(event) ?? []) run(h, { event, payload });
        },
      },
    });
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: false,
      writable: false,
      value: {
        invoke,
        transformCallback: reg,
        unregisterCallback: (id) => callbacks.delete(id),
        runCallback: run,
        callbacks,
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
      },
    });
  }, options);
}
