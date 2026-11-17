import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    // Pinned, not left to the default. On WSL the forks pool lets a worker time
    // out, drop its whole file, and still exit 0 — a green run that silently
    // tested less than it claimed. Threads surface the failure
    // (spec: chat-streaming-performance, "a dropped test file fails the run").
    pool: "threads",
    exclude: [
      "**/.kilo/**",
      "**/.worktrees/**",
      "**/dist/**",
      "**/node_modules/**",
      "src-tauri/target/**",
      "target/**",
    ],
    include: ["src/**/*.{test,spec}.{ts,tsx}", "benchmarks/**/*.{test,spec,perf.test,bench}.{ts,tsx}"],
    setupFiles: ["./src/test/setup.ts"],
  },
});
