#!/usr/bin/env node
"use strict";

const { runRemem } = require("./remem-binary");

try {
  process.exit(runRemem(["mcp"]));
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(1);
}
