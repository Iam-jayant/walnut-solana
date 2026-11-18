// Browser polyfill for the Node `Buffer` global. @anchor-lang/core, the Arcium
// client and the SDK use it. Import this module before any of them.
import { Buffer } from "buffer";

const scope = globalThis as unknown as { Buffer?: typeof Buffer };
if (typeof scope.Buffer === "undefined") scope.Buffer = Buffer;
