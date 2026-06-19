#!/usr/bin/env node

"use strict";

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const { platformKey } = require("../scripts/platform");

function binaryPath() {
  if (process.env.REMEM_NPM_BINARY) {
    return process.env.REMEM_NPM_BINARY;
  }
  return path.join(__dirname, "..", "vendor", platformKey(), "remem");
}

const bin = binaryPath();
if (!fs.existsSync(bin)) {
  console.error(`remem binary not found at ${bin}`);
  console.error("Run `npm rebuild @remem-ai/remem` or set REMEM_NPM_BINARY.");
  process.exit(1);
}

const result = spawnSync(bin, process.argv.slice(2), {
  stdio: "inherit",
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status === null ? 1 : result.status);
