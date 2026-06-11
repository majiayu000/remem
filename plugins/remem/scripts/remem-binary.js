#!/usr/bin/env node
"use strict";

const {
  ensureRuntimeSync,
  expectedVersion,
  inspectVersion,
  isExecutable,
  pathCandidates,
  pluginRoot,
  repoCandidates,
  runRemem,
  versionMismatchMessage
} = require("./remem-runtime");

function findRememBinary() {
  return ensureRuntimeSync();
}

module.exports = {
  expectedVersion,
  findRememBinary,
  inspectVersion,
  isExecutable,
  pathCandidates,
  pluginRoot,
  repoCandidates,
  versionMismatchMessage,
  runRemem
};

if (require.main === module) {
  try {
    process.exit(runRemem(process.argv.slice(2)));
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exit(1);
  }
}
