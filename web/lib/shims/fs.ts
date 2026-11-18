// Browser replacement for the Node `fs` module imported (but never called on
// the Web App's code path) by @arcium-hq/client.
const fs = {
  readFileSync(): never {
    throw new Error("the file system is not available in the browser");
  },
};

export default fs;
