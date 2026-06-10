#!/usr/bin/env node
"use strict";

const { runRemem } = require("./remem-binary");

try {
  const extraArgs = process.argv.slice(2);
  process.exit(runRemem(["install", "--target", "codex", ...extraArgs]));
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(1);
}
