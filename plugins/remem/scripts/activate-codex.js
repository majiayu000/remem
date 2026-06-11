#!/usr/bin/env node
"use strict";

const { runRememAsync } = require("./remem-runtime");

async function main() {
  const extraArgs = process.argv.slice(2);
  const dryRun = extraArgs.includes("--dry-run");
  return runRememAsync(["install", "--target", "codex", "--hooks-only", ...extraArgs], {
    allowDownload: !dryRun,
    adoptLocal: !dryRun
  });
}

main()
  .then((status) => process.exit(status))
  .catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exit(1);
  });
