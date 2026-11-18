// Browser entry of @anchor-lang/core with a default export added.
// @arcium-hq/client does `import anchor from "@anchor-lang/core"` (default
// import) to read `BN`; the browser build only has named exports.
import * as anchor from "@anchor-lang/core/dist/browser/index.js";

export * from "@anchor-lang/core/dist/browser/index.js";
export default anchor;
