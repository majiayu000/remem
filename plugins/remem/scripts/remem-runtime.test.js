#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const {
  codesignRuntimeIfNeeded,
  copyRuntime,
  ensureRuntimeSync,
  ensureRuntime,
  expectedVersion,
  installRuntime,
  inspectRuntime,
  managedBinaryPath,
  pluginDataDir,
  releaseAssetForCurrentPlatform,
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

function shellQuote(value) {
  return `'${value.replace(/'/g, "'\\''")}'`;
}

function writeRecordingRemem(file, version, argsPath, stdinPath) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(
    file,
    [
      "#!/bin/sh",
      "if [ \"$1\" = \"--version\" ]; then",
      `  printf 'remem ${version} (schema v34)\\n'`,
      "  exit 0",
      "fi",
      `printf '%s\\n' "$@" > ${shellQuote(argsPath)}`,
      `cat > ${shellQuote(stdinPath)}`,
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
    platformKey: options.platformKey || "linux-x64",
    env: {
      PATH: options.path || "",
      REMEM_ALLOW_VERSION_MISMATCH: options.allowMismatch || ""
    }
  };
}

function currentPlatformKey() {
  if (process.platform === "darwin" && process.arch === "arm64") return "darwin-arm64";
  if (process.platform === "darwin" && process.arch === "x64") return "darwin-x64";
  if (process.platform === "linux" && process.arch === "arm64") return "linux-arm64";
  if (process.platform === "linux" && process.arch === "x64") return "linux-x64";
  throw new Error(`Unsupported platform: ${process.platform}/${process.arch}`);
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

test("release asset resolver uses release-hosted manifest when local assets are empty", async () => {
  const fx = fixture();
  writeJson(path.join(fx.pluginRoot, "runtimes", "remem-releases.json"), {
    versions: {
      "0.5.17": {
        base_url: "https://example.test/remem/releases/download/v0.5.17",
        assets: {}
      }
    }
  });
  const sha = "a".repeat(64);

  const asset = await releaseAssetForCurrentPlatform({
    ...fx,
    fetchJson: async (url) => {
      assert.equal(url, "https://example.test/remem/releases/download/v0.5.17/remem-releases.json");
      return {
        versions: {
          "0.5.17": {
            assets: {
              "darwin-arm64": { file: "remem-darwin-arm64.tar.gz", sha256: sha },
              "darwin-x64": { file: "remem-darwin-x64.tar.gz", sha256: sha },
              "linux-arm64": { file: "remem-linux-arm64.tar.gz", sha256: sha },
              "linux-x64": { file: "remem-linux-x64.tar.gz", sha256: sha }
            }
          }
        }
      };
    }
  });

  assert.match(asset.file, /^remem-(darwin|linux)-/);
  assert.equal(asset.sha256, sha);
  assert.match(asset.url, /^https:\/\/example\.test\/remem\/releases\/download\/v0\.5\.17\/remem-/);
});

test("release asset resolver rejects unsafe manifest file names", async () => {
  const key = currentPlatformKey();
  for (const file of [
    "../remem-linux-x64.tar.gz",
    "/tmp/remem-linux-x64.tar.gz",
    "nested/remem-linux-x64.tar.gz",
    "https://example.test/remem-linux-x64.tar.gz",
    "remem-windows-x64.tar.gz"
  ]) {
    const fx = fixture();
    writeJson(path.join(fx.pluginRoot, "runtimes", "remem-releases.json"), {
      versions: {
        "0.5.17": {
          assets: {
            [key]: { file, sha256: "a".repeat(64) }
          }
        }
      }
    });

    await assert.rejects(() => releaseAssetForCurrentPlatform(fx), /unsafe file name/);
  }
});

test("release asset resolver rejects unsafe remote manifest file names", async () => {
  const key = currentPlatformKey();
  const fx = fixture();
  writeJson(path.join(fx.pluginRoot, "runtimes", "remem-releases.json"), {
    versions: {
      "0.5.17": {
        base_url: "https://example.test/remem/releases/download/v0.5.17",
        assets: {}
      }
    }
  });

  await assert.rejects(
    () =>
      releaseAssetForCurrentPlatform({
        ...fx,
        fetchJson: async () => ({
          versions: {
            "0.5.17": {
              assets: {
                [key]: { file: "../remem-linux-x64.tar.gz", sha256: "a".repeat(64) }
              }
            }
          }
        })
      }),
    /unsafe file name/
  );
});

test("release download rejects remote manifest assets without valid checksums", async () => {
  const fx = fixture();
  writeJson(path.join(fx.pluginRoot, "runtimes", "remem-releases.json"), {
    versions: {
      "0.5.17": {
        base_url: "https://example.test/remem/releases/download/v0.5.17",
        assets: {}
      }
    }
  });

  await assert.rejects(
    () =>
      ensureRuntime({
        ...fx,
        allowDownload: true,
        fetchJson: async () => ({
          versions: {
            "0.5.17": {
              assets: {
                "linux-x64": { file: "remem-linux-x64.tar.gz", sha256: "not-a-sha" },
                "linux-arm64": { file: "remem-linux-arm64.tar.gz", sha256: "not-a-sha" },
                "darwin-x64": { file: "remem-darwin-x64.tar.gz", sha256: "not-a-sha" },
                "darwin-arm64": { file: "remem-darwin-arm64.tar.gz", sha256: "not-a-sha" }
              }
            }
          }
        })
      }),
    /missing a valid sha256/
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

test("ensureRuntime can use local runtime without adopting it", async () => {
  const fx = fixture();
  const source = path.join(fx.repoRoot, "target", "release", "remem");
  writeFakeRemem(source, "0.5.17");

  const resolved = await ensureRuntime({ ...fx, adoptLocal: false, allowDownload: false });

  assert.equal(resolved, source);
  assert.equal(fs.existsSync(managedBinaryPath(fx)), false);
  assert.equal(fs.existsSync(runtimeMetadataPath(fx)), false);
});

test("activation dry-run uses local runtime without writing plugin runtime files", async () => {
  const fx = fixture();
  const source = path.join(fx.repoRoot, "target", "release", "remem");
  writeFakeRemem(source, "0.5.17");

  const status = await runRememAsync(
    ["install", "--target", "codex", "--hooks-only", "--dry-run"],
    { ...fx, adoptLocal: false, allowDownload: false }
  );

  assert.equal(status, 0);
  assert.equal(fs.existsSync(managedBinaryPath(fx)), false);
  assert.equal(fs.existsSync(runtimeMetadataPath(fx)), false);
});

test("packaged Codex hooks call the plugin runtime hook wrapper", () => {
  const hooksPath = path.join(__dirname, "..", "hooks", "hooks.json");
  const hooks = JSON.parse(fs.readFileSync(hooksPath, "utf8"));

  assert.equal(
    hooks.hooks.SessionStart[0].hooks[0].command,
    'node "${PLUGIN_ROOT}/scripts/remem-hook.js" session-start'
  );
  assert.equal(hooks.hooks.SessionStart[0].hooks[0].timeout, 15);
  assert.equal(
    hooks.hooks.Stop[0].hooks[0].command,
    'node "${PLUGIN_ROOT}/scripts/remem-hook.js" stop'
  );
  assert.equal(hooks.hooks.Stop[0].hooks[0].timeout, 120);
  assert.equal(hooks.hooks.PostToolUse, undefined);
});

test("remem-hook session-start delegates to Codex context through explicit runtime", () => {
  const dir = tempDir("remem-plugin-hook-");
  const fake = path.join(dir, "remem");
  const argsPath = path.join(dir, "args.txt");
  const stdinPath = path.join(dir, "stdin.json");
  writeRecordingRemem(fake, expectedVersion(), argsPath, stdinPath);

  const input = '{"session_id":"sess-1","cwd":"/tmp/remem"}';
  const result = spawnSync(process.execPath, [path.join(__dirname, "remem-hook.js"), "session-start"], {
    encoding: "utf8",
    input,
    env: {
      ...process.env,
      REMEM_BINARY: fake,
      REMEM_PLUGIN_DATA: path.join(dir, "data")
    }
  });

  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(fs.readFileSync(argsPath, "utf8").trim().split(/\n/), [
    "context",
    "--host",
    "codex-cli"
  ]);
  assert.equal(fs.readFileSync(stdinPath, "utf8"), input);
});

test("remem-hook stop delegates to Codex summarize and rejects unknown actions", () => {
  const dir = tempDir("remem-plugin-hook-");
  const fake = path.join(dir, "remem");
  const argsPath = path.join(dir, "args.txt");
  const stdinPath = path.join(dir, "stdin.json");
  const env = {
    ...process.env,
    REMEM_BINARY: fake,
    REMEM_PLUGIN_DATA: path.join(dir, "data")
  };
  writeRecordingRemem(fake, expectedVersion(), argsPath, stdinPath);

  const ok = spawnSync(process.execPath, [path.join(__dirname, "remem-hook.js"), "stop"], {
    encoding: "utf8",
    input: '{"session_id":"sess-2"}',
    env
  });

  assert.equal(ok.status, 0, ok.stderr);
  assert.deepEqual(fs.readFileSync(argsPath, "utf8").trim().split(/\n/), [
    "summarize",
    "--host",
    "codex-cli"
  ]);

  const rejected = spawnSync(process.execPath, [path.join(__dirname, "remem-hook.js"), "observe"], {
    encoding: "utf8",
    env
  });
  assert.equal(rejected.status, 1);
  assert.match(rejected.stderr, /Usage: remem-hook\.js session-start\|stop/);
});

test("darwin arm64 runtimes require ad-hoc codesign", () => {
  assert.equal(shouldCodesignRuntime({ platformKey: "darwin-arm64" }), true);
  assert.equal(shouldCodesignRuntime({ platformKey: "darwin-x64" }), false);
  assert.equal(shouldCodesignRuntime({ platformKey: "linux-arm64" }), false);
  assert.equal(shouldCodesignRuntime({ platformKey: "win32-x64" }), false);
});

test("codesign is skipped without throwing on non-darwin-arm64 platforms", () => {
  assert.doesNotThrow(() => codesignRuntimeIfNeeded("unused", { platformKey: "win32-x64" }));
  assert.doesNotThrow(() => codesignRuntimeIfNeeded("unused", { platformKey: "linux-x64" }));
  assert.doesNotThrow(() => codesignRuntimeIfNeeded("unused", { platformKey: "freebsd-x64" }));
});
