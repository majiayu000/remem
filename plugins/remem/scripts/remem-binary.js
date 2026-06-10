#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const VERSION_RE = /remem\s+([0-9]+\.[0-9]+\.[0-9]+(?:[-+][^\s]+)?)\s+\(schema v([0-9]+)\)/;

function isExecutable(candidate) {
  if (!candidate) return false;
  try {
    fs.accessSync(candidate, fs.constants.X_OK);
    return true;
  } catch (_error) {
    return false;
  }
}

function pluginRoot() {
  return path.resolve(__dirname, "..");
}

function expectedVersion() {
  const manifestPath = path.join(pluginRoot(), ".codex-plugin", "plugin.json");
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  if (!manifest.version) {
    throw new Error(`Plugin manifest is missing version: ${manifestPath}`);
  }
  return manifest.version;
}

function inspectVersion(candidate) {
  const result = spawnSync(candidate, ["--version"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 3000
  });
  if (result.error) {
    return {
      ok: false,
      reason: result.error.message
    };
  }
  if (result.status !== 0) {
    return {
      ok: false,
      reason: (result.stderr || result.stdout || `exit ${result.status}`).trim()
    };
  }
  const output = (result.stdout || result.stderr || "").trim();
  const match = VERSION_RE.exec(output);
  if (!match) {
    return {
      ok: false,
      reason: `unexpected version output: ${output}`
    };
  }
  return {
    ok: true,
    version: match[1],
    schemaVersion: Number(match[2]),
    output
  };
}

function pathCandidates() {
  const names = process.platform === "win32" ? ["remem.exe", "remem"] : ["remem"];
  return (process.env.PATH || "")
    .split(path.delimiter)
    .filter(Boolean)
    .flatMap((dir) => names.map((name) => path.join(dir, name)));
}

function repoCandidates() {
  const repoRoot = path.resolve(pluginRoot(), "..", "..");
  return [
    path.join(repoRoot, "target", "release", process.platform === "win32" ? "remem.exe" : "remem"),
    path.join(repoRoot, "target", "debug", process.platform === "win32" ? "remem.exe" : "remem")
  ];
}

function findRememBinary() {
  const expected = expectedVersion();
  const allowMismatch = process.env.REMEM_ALLOW_VERSION_MISMATCH === "1";
  const explicit = process.env.REMEM_BINARY;
  if (explicit) {
    if (!isExecutable(explicit)) {
      throw new Error(`REMEM_BINARY is set but is not executable: ${explicit}`);
    }
    const version = inspectVersion(explicit);
    if (allowMismatch || (version.ok && version.version === expected)) return explicit;
    throw new Error(versionMismatchMessage(explicit, expected, version));
  }

  const rejected = [];
  for (const candidate of [...repoCandidates(), ...pathCandidates()]) {
    if (!isExecutable(candidate)) continue;
    const version = inspectVersion(candidate);
    if (allowMismatch || (version.ok && version.version === expected)) return candidate;
    rejected.push(versionMismatchMessage(candidate, expected, version));
  }

  throw new Error(
    [
      `Unable to find remem ${expected}.`,
      "Build the matching binary with `cargo build --release`, install the same remem version on PATH,",
      "or set REMEM_BINARY=/absolute/path/to/remem.",
      "Set REMEM_ALLOW_VERSION_MISMATCH=1 only for explicit local debugging.",
      rejected.length ? `Rejected candidates: ${rejected.join(" | ")}` : "No executable candidates were found."
    ].join(" ")
  );
}

function versionMismatchMessage(candidate, expected, version) {
  if (!version.ok) {
    return `${candidate}: ${version.reason}`;
  }
  return `${candidate}: found remem ${version.version} (schema v${version.schemaVersion}), expected ${expected}`;
}

function runRemem(args, options = {}) {
  const bin = findRememBinary();
  const result = spawnSync(bin, args, {
    stdio: "inherit",
    env: {
      ...process.env,
      REMEM_PLUGIN_ROOT: pluginRoot()
    },
    ...options
  });

  if (result.error) {
    throw result.error;
  }
  return result.status === null ? 1 : result.status;
}

module.exports = {
  findRememBinary,
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
