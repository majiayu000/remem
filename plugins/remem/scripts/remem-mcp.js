#!/usr/bin/env node
"use strict";

const { inspectVersion } = require("./remem-binary");
const { ensureRuntimeSync, runRemem } = require("./remem-runtime");

try {
  if (process.argv.includes("--self-test")) {
    const bin = ensureRuntimeSync();
    const version = inspectVersion(bin);
    if (!version.ok) {
      throw new Error(version.reason);
    }
    process.stdout.write(`${version.output}\n`);
    process.exit(0);
  }
  process.exit(runRemem(["mcp"]));
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(1);
}
