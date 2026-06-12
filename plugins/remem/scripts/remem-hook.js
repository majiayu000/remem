#!/usr/bin/env node
"use strict";

const { runRememAsync } = require("./remem-runtime");

const ACTIONS = new Map([
  ["session-start", ["context", "--host", "codex-cli"]],
  ["stop", ["summarize", "--host", "codex-cli"]]
]);

async function main(argv) {
  const [action, ...extra] = argv;
  if (!ACTIONS.has(action) || extra.length > 0) {
    const supported = Array.from(ACTIONS.keys()).join("|");
    throw new Error(`Usage: remem-hook.js ${supported}`);
  }
  return runRememAsync(ACTIONS.get(action), {
    allowDownload: false
  });
}

module.exports = {
  ACTIONS,
  main
};

if (require.main === module) {
  main(process.argv.slice(2))
    .then((status) => process.exit(status))
    .catch((error) => {
      process.stderr.write(`${error.message}\n`);
      process.exit(1);
    });
}
