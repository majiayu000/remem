#!/usr/bin/env node
"use strict";

const { inspectVersion } = require("./remem-binary");
const { ensureRuntime, runRememAsync } = require("./remem-runtime");

async function main() {
  if (process.argv.includes("--self-test")) {
    const bin = await ensureRuntime();
    const version = inspectVersion(bin);
    if (!version.ok) {
      throw new Error(version.reason);
    }
    process.stdout.write(`${version.output}\n`);
    return 0;
  }
  return runRememAsync(["mcp"]);
}

main()
  .then((status) => process.exit(status))
  .catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exit(1);
  });
