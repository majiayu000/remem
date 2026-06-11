#!/usr/bin/env node
"use strict";

const { runRemem } = require("./remem-binary");

try {
  const extraArgs = process.argv.slice(2);
  process.exit(runRemem(["install", "--target", "codex", "--hooks-only", ...extraArgs]));
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(1);
}
