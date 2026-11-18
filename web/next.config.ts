import type { NextConfig } from "next";

// @arcium-hq/client imports the Node `crypto` and `fs` modules. In the browser
// bundle they resolve to small shims (SHA-256/SHA3 and random bytes only).
// It also default-imports @anchor-lang/core, whose browser build has only named
// exports, so that import goes through a shim that adds the default export.
const nextConfig: NextConfig = {
  reactStrictMode: true,
  turbopack: {
    root: `${import.meta.dirname}/..`,
    resolveAlias: {
      "@anchor-lang/core": "./lib/shims/anchor.js",
      crypto: { browser: "./lib/shims/crypto.ts" },
      fs: { browser: "./lib/shims/fs.ts" },
    },
  },
};

export default nextConfig;
