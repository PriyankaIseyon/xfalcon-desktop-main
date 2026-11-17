import { defineConfig } from "@playwright/test";
import { fileURLToPath } from "node:url";
import chatConfig from "./playwright.config";

const baseURL = "http://127.0.0.1:1422";

/** Run after `npm run build` to measure the frontend shipped in the app. */
export default defineConfig({
  ...chatConfig,
  testDir: fileURLToPath(new URL(".", import.meta.url)),
  use: { ...chatConfig.use, baseURL },
  webServer: {
    cwd: fileURLToPath(new URL("../..", import.meta.url)),
    command: "npm run preview -- --host 127.0.0.1 --port 1422 --strictPort",
    reuseExistingServer: !process.env.CI,
    url: baseURL,
  },
});
