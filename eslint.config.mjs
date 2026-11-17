// ESLint flat config for the TypeScript and JavaScript workspace.
// CI runs `pnpm lint` (eslint . --max-warnings 0).
import js from "@eslint/js";
import { defineConfig } from "eslint/config";
import globals from "globals";
import tseslint from "typescript-eslint";

export default defineConfig(
  {
    ignores: [
      "**/node_modules/",
      "**/dist/",
      "**/.next/",
      "**/next-env.d.ts",
      "**/out/",
      "**/coverage/",
      "target/",
      "build/",
      ".anchor/",
      "test-ledger/",
      ".kiro/",
      "semantic-review/",
    ],
  },
  js.configs.recommended,
  tseslint.configs.recommended,
  {
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: { ...globals.node },
    },
    linterOptions: {
      reportUnusedDisableDirectives: "error",
    },
  },
  {
    files: ["**/*.cjs"],
    languageOptions: { sourceType: "commonjs" },
  },
  {
    files: ["tests/**/*.ts", "**/*.test.ts", "**/*.spec.ts"],
    languageOptions: { globals: { ...globals.mocha } },
  },
);
