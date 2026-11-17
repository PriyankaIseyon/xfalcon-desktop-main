import { defineConfig, devices } from "@playwright/test";

/** E2e slice gate: same dev server as visual tests, URL aligned with vite `host: localhost`. */
export default defineConfig({
  testMatch: "tests/e2e/**/*.spec.ts",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: "http://localhost:1420",
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: "npm run dev",
    reuseExistingServer: !process.env.CI,
    url: "http://localhost:1420",
  },
});
