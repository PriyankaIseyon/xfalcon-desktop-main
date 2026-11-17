import eslint from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: [
      ".agent/**",
      ".agents/**",
      ".claude/**",
      ".codex/**",
      ".cursor/**",
      ".kilo/**",
      ".opencode/**",
      ".worktrees/**",
      "**/.worktrees/**",
      ".github/agents/**",
      ".github/prompts/**",
      ".github/skills/**",
      "dist/**",
      "node_modules/**",
      "src-tauri/target/**",
      // DA's thumbnail capture scripts, vendored verbatim and pinned by digest
      // (`reports/kit_flow.rs`). Linting would only ask for edits they must not take.
      "src-tauri/src/reports/capture_assets/**",
      "target/**",
    ],
  },
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.es2022,
      },
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      "react-hooks/exhaustive-deps": "error",
      "react-hooks/rules-of-hooks": "error",
      ...reactRefresh.configs.vite.rules,
    },
  },
  {
    files: ["*.config.{js,ts}", "eslint.config.js", "scripts/**/*.mjs"],
    languageOptions: {
      globals: globals.node,
    },
  },
);
