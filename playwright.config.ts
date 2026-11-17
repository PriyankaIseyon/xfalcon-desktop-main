import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/visual",
  snapshotPathTemplate: "{testDir}/baselines/{arg}{ext}",
  fullyParallel: true,
  timeout: 60_000,
  workers: 4,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? "github" : "list",
  updateSnapshots: "none",
  expect: {
    toHaveScreenshot: {
      animations: "disabled",
      caret: "hide",
      // Tight on purpose. At the previous 0.3 / 2% the suite passed a baseline
      // that still drew the sidebar the app had before it grew a destination
      // rail: text glyphs are a small share of a page's pixels, and a
      // generous per-pixel tolerance forgave the rest. One fixed headless
      // Chromium renders this deterministically, so the only thing these
      // numbers have to absorb is antialiasing.
      maxDiffPixelRatio: 0.001,
      threshold: 0.15,
    },
  },
  use: {
    baseURL: "http://127.0.0.1:1420",
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
    {
      name: "office-webkit",
      testMatch: "office-pdf.spec.ts",
      use: { ...devices["Desktop Safari"] },
    },
  ],
  webServer: {
    command: "npm run dev -- --host 127.0.0.1",
    reuseExistingServer: !process.env.CI,
    url: "http://127.0.0.1:1420",
  },
});
