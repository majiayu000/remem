#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  copyRuntime,
  ensureRuntimeSync,
  installRuntime,
  inspectRuntime,
  managedBinaryPath,
  pluginDataDir,
  runRememAsync,
  runtimeMetadataPath,
  shouldCodesignRuntime
} = require("./remem-runtime");

function tempDir(prefix) {
  return fs.mkdtempSync(path.join(os.tmpdir(), prefix));
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function writeFakeRemem(file, version) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(
    file,
    [
      "#!/bin/sh",
      "if [ \"$1\" = \"--version\" ]; then",
      `  printf 'remem ${version} (schema v34)\\n'`,
      "  exit 0",
      "fi",
      "exit 0",
      ""
    ].join("\n")
  );
  fs.chmodSync(file, 0o755);
}

function fixture(options = {}) {
  const root = tempDir("remem-plugin-root-");
  const repo = tempDir("remem-plugin-repo-");
  const data = tempDir("remem-plugin-data-");
  writeJson(path.join(root, ".codex-plugin", "plugin.json"), {
    name: "remem",
    version: "0.5.17"
  });
  return {
    pluginRoot: root,
    repoRoot: repo,
    pluginData: data,
    env: {
      PATH: options.path || "",
      REMEM_ALLOW_VERSION_MISMATCH: options.allowMismatch || ""
    }
  };
}

test("plugin data prefers explicit override", () => {
  const data = tempDir("remem-plugin-data-");
  assert.equal(pluginDataDir({ pluginData: data }), data);
});

test("ensureRuntimeSync copies matching repo binary into plugin data", () => {
  const fx = fixture();
  const source = path.join(fx.repoRoot, "target", "release", "remem");
  writeFakeRemem(source, "0.5.17");

  const installed = ensureRuntimeSync(fx);

  assert.equal(installed, managedBinaryPath(fx));
  assert.equal(fs.existsSync(installed), true);
  assert.equal(fs.existsSync(runtimeMetadataPath(fx)), true);
  const status = inspectRuntime(fx);
  assert.equal(status.selected.source, "managed");
});

test("matching PATH binary is reported but not silently adopted", () => {
  const pathDir = tempDir("remem-plugin-path-");
  writeFakeRemem(path.join(pathDir, "remem"), "0.5.17");
  const fx = fixture({ path: pathDir });

  assert.throws(() => ensureRuntimeSync(fx), /does not adopt PATH binaries silently|Unable to find/);
  const status = inspectRuntime(fx);
  assert.equal(status.pathMatch.path, path.join(pathDir, "remem"));
  assert.equal(fs.existsSync(managedBinaryPath(fx)), false);
});

test("mismatched managed binary is rejected", () => {
  const fx = fixture();
  writeFakeRemem(managedBinaryPath(fx), "0.5.11");

  const status = inspectRuntime(fx);

  assert.equal(status.selected, undefined);
  assert.match(status.candidates[0].reason, /expected 0.5.17/);
});

test("copyRuntime rejects mismatched source version", () => {
  const fx = fixture();
  const source = path.join(fx.repoRoot, "target", "release", "remem");
  writeFakeRemem(source, "0.5.11");

  assert.throws(() => copyRuntime(source, fx), /expected 0.5.17/);
  assert.equal(fs.existsSync(runtimeMetadataPath(fx)), false);
});

test("installRuntime honors no-download fallback", async () => {
  const fx = fixture();

  await assert.rejects(
    () => installRuntime({ ...fx, allowDownload: false }),
    (error) => {
      assert.match(error.message, /Unable to find plugin-managed remem 0.5.17/);
      assert.doesNotMatch(error.message, /No checked release asset/);
      return true;
    }
  );
});

test("runRememAsync resolves local runtime before executing", async () => {
  const fx = fixture();
  const source = path.join(fx.repoRoot, "target", "release", "remem");
  writeFakeRemem(source, "0.5.17");

  const status = await runRememAsync(["mcp"], fx);

  assert.equal(status, 0);
  assert.equal(fs.existsSync(managedBinaryPath(fx)), true);
});

test("darwin arm64 runtimes require ad-hoc codesign", () => {
  assert.equal(shouldCodesignRuntime({ platformKey: "darwin-arm64" }), true);
  assert.equal(shouldCodesignRuntime({ platformKey: "darwin-x64" }), false);
  assert.equal(shouldCodesignRuntime({ platformKey: "linux-arm64" }), false);
});
