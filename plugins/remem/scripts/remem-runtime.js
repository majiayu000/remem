#!/usr/bin/env node
"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const VERSION_RE = /remem\s+([0-9]+\.[0-9]+\.[0-9]+(?:[-+][^\s]+)?)\s+\(schema v([0-9]+)\)/;

function binaryName() {
  return process.platform === "win32" ? "remem.exe" : "remem";
}

function pluginRoot(options = {}) {
  return path.resolve(options.pluginRoot || path.join(__dirname, ".."));
}

function repoRoot(options = {}) {
  return path.resolve(options.repoRoot || path.join(pluginRoot(options), "..", ".."));
}

function expectedVersion(options = {}) {
  if (options.expectedVersion) return options.expectedVersion;
  const manifestPath = path.join(pluginRoot(options), ".codex-plugin", "plugin.json");
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  if (!manifest.version) {
    throw new Error(`Plugin manifest is missing version: ${manifestPath}`);
  }
  return manifest.version;
}

function defaultPluginDataDir() {
  if (process.env.REMEM_PLUGIN_DATA) return process.env.REMEM_PLUGIN_DATA;
  if (process.env.PLUGIN_DATA) return process.env.PLUGIN_DATA;
  const home = os.homedir();
  if (process.platform === "win32") {
    return path.join(process.env.LOCALAPPDATA || home, "remem", "codex-plugin");
  }
  return path.join(home, ".remem", "codex-plugin");
}

function pluginDataDir(options = {}) {
  return path.resolve(options.pluginData || defaultPluginDataDir());
}

function managedBinaryPath(options = {}) {
  return path.join(pluginDataDir(options), "bin", binaryName());
}

function runtimeMetadataPath(options = {}) {
  return path.join(pluginDataDir(options), "runtime.json");
}

function isExecutable(candidate) {
  if (!candidate) return false;
  try {
    fs.accessSync(candidate, fs.constants.X_OK);
    return true;
  } catch (_error) {
    return false;
  }
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

function pathCandidates(env = process.env) {
  const names = process.platform === "win32" ? ["remem.exe", "remem"] : ["remem"];
  return (env.PATH || "")
    .split(path.delimiter)
    .filter(Boolean)
    .flatMap((dir) => names.map((name) => path.join(dir, name)));
}

function repoCandidates(options = {}) {
  const root = repoRoot(options);
  return [
    path.join(root, "target", "release", binaryName()),
    path.join(root, "target", "debug", binaryName())
  ];
}

function candidateEntries(options = {}) {
  const env = options.env || process.env;
  const entries = [];
  if (env.REMEM_BINARY) {
    entries.push({ source: "explicit", path: env.REMEM_BINARY, adoptable: true });
  }
  entries.push({ source: "managed", path: managedBinaryPath(options), adoptable: true });
  for (const candidate of repoCandidates(options)) {
    entries.push({ source: "repo", path: candidate, adoptable: true });
  }
  for (const candidate of pathCandidates(env)) {
    entries.push({ source: "path", path: candidate, adoptable: false });
  }
  return entries;
}

function inspectCandidate(entry, expected, allowMismatch) {
  if (!isExecutable(entry.path)) {
    return {
      ...entry,
      exists: fs.existsSync(entry.path),
      executable: false,
      ok: false,
      reason: "not executable"
    };
  }
  const version = inspectVersion(entry.path);
  const versionOk = version.ok && (allowMismatch || version.version === expected);
  return {
    ...entry,
    exists: true,
    executable: true,
    ok: versionOk,
    version: version.version,
    schemaVersion: version.schemaVersion,
    output: version.output,
    reason: versionOk ? undefined : versionMismatchMessage(entry.path, expected, version)
  };
}

function inspectRuntime(options = {}) {
  const expected = expectedVersion(options);
  const allowMismatch = (options.env || process.env).REMEM_ALLOW_VERSION_MISMATCH === "1";
  const candidates = candidateEntries(options).map((entry) =>
    inspectCandidate(entry, expected, allowMismatch)
  );
  const selected = candidates.find((candidate) => candidate.ok && candidate.adoptable);
  const pathMatch = candidates.find((candidate) => candidate.ok && !candidate.adoptable);
  return {
    expectedVersion: expected,
    pluginRoot: pluginRoot(options),
    pluginData: pluginDataDir(options),
    managedBinary: managedBinaryPath(options),
    metadataPath: runtimeMetadataPath(options),
    selected,
    pathMatch,
    candidates
  };
}

function versionMismatchMessage(candidate, expected, version) {
  if (!version.ok) {
    return `${candidate}: ${version.reason}`;
  }
  return `${candidate}: found remem ${version.version} (schema v${version.schemaVersion}), expected ${expected}`;
}

function shouldCodesignRuntime(options = {}) {
  if (options.platformKey) return options.platformKey === "darwin-arm64";
  return os.platform() === "darwin" && os.arch() === "arm64";
}

function codesignRuntimeIfNeeded(candidate, options = {}) {
  if (!shouldCodesignRuntime(options)) return;
  const result = spawnSync("codesign", ["--force", "--sign", "-", candidate], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 5000
  });
  if (result.error) {
    throw new Error(`codesign failed for ${candidate}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    const reason = (result.stderr || result.stdout || `exit ${result.status}`).trim();
    throw new Error(`codesign failed for ${candidate}: ${reason}`);
  }
}

function copyRuntime(source, options = {}) {
  const dest = managedBinaryPath(options);
  fs.mkdirSync(path.dirname(dest), { recursive: true, mode: 0o755 });
  fs.copyFileSync(source, dest);
  fs.chmodSync(dest, 0o755);
  codesignRuntimeIfNeeded(dest, options);
  const version = inspectVersion(dest);
  if (!version.ok) {
    throw new Error(`Copied runtime is not executable: ${version.reason}`);
  }
  const expected = expectedVersion(options);
  const allowMismatch = (options.env || process.env).REMEM_ALLOW_VERSION_MISMATCH === "1";
  if (!allowMismatch && version.version !== expected) {
    throw new Error(versionMismatchMessage(dest, expected, version));
  }
  const metadata = {
    version: version.version,
    schema: version.schemaVersion,
    source: "local-copy",
    source_path: source,
    binary: dest,
    installed_at: new Date().toISOString()
  };
  writeMetadata(metadata, options);
  return {
    path: dest,
    version,
    metadata
  };
}

function writeMetadata(metadata, options = {}) {
  const metadataPath = runtimeMetadataPath(options);
  fs.mkdirSync(path.dirname(metadataPath), { recursive: true, mode: 0o755 });
  fs.writeFileSync(metadataPath, `${JSON.stringify(metadata, null, 2)}\n`, { mode: 0o600 });
}

function readReleaseManifest(options = {}) {
  const manifestPath = path.join(pluginRoot(options), "runtimes", "remem-releases.json");
  if (!fs.existsSync(manifestPath)) return null;
  return JSON.parse(fs.readFileSync(manifestPath, "utf8"));
}

function platformKey() {
  const platform = os.platform();
  const arch = os.arch();
  if (platform === "darwin" && arch === "arm64") return "darwin-arm64";
  if (platform === "darwin" && arch === "x64") return "darwin-x64";
  if (platform === "linux" && arch === "arm64") return "linux-arm64";
  if (platform === "linux" && arch === "x64") return "linux-x64";
  throw new Error(`Unsupported platform: ${platform}/${arch}`);
}

function releaseAssetForCurrentPlatform(options = {}) {
  const manifest = readReleaseManifest(options);
  if (!manifest) return null;
  const expected = expectedVersion(options);
  const release = manifest.versions && manifest.versions[expected];
  if (!release) return null;
  const asset = release.assets && release.assets[platformKey()];
  if (!asset) return null;
  const baseUrl = release.base_url || `https://github.com/majiayu000/remem/releases/download/v${expected}`;
  return {
    ...asset,
    url: asset.url || `${baseUrl}/${asset.file}`
  };
}

function download(url, dest, redirects = 0) {
  return new Promise((resolve, reject) => {
    const request = https.get(url, (response) => {
      if (
        response.statusCode >= 300 &&
        response.statusCode < 400 &&
        response.headers.location
      ) {
        response.resume();
        if (redirects >= 5) {
          reject(new Error(`Too many redirects while downloading ${url}`));
          return;
        }
        const next = new URL(response.headers.location, url).toString();
        resolve(download(next, dest, redirects + 1));
        return;
      }

      if (response.statusCode !== 200) {
        response.resume();
        reject(new Error(`Download failed with HTTP ${response.statusCode}: ${url}`));
        return;
      }

      const output = fs.createWriteStream(dest, { mode: 0o600 });
      response.pipe(output);
      output.on("finish", () => output.close(resolve));
      output.on("error", reject);
    });
    request.on("error", reject);
  });
}

function sha256(file) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(file));
  return hash.digest("hex");
}

async function downloadRuntime(options = {}) {
  const asset = releaseAssetForCurrentPlatform(options);
  if (!asset) {
    throw new Error(
      `No checked release asset is available for remem ${expectedVersion(options)} on ${platformKey()}`
    );
  }
  if (!asset.sha256 || !/^[0-9a-f]{64}$/i.test(asset.sha256)) {
    throw new Error(`Release asset is missing a valid sha256: ${asset.file || asset.url}`);
  }

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "remem-plugin-"));
  const archive = path.join(tmpDir, asset.file || "remem.tar.gz");
  const extractDir = path.join(tmpDir, "extract");
  try {
    await download(asset.url, archive);
    const actual = sha256(archive);
    if (actual !== asset.sha256) {
      throw new Error(`Checksum mismatch for ${asset.file}: expected ${asset.sha256}, got ${actual}`);
    }
    fs.mkdirSync(extractDir, { recursive: true, mode: 0o755 });
    const tar = spawnSync("tar", ["-xzf", archive, "-C", extractDir], {
      stdio: "ignore"
    });
    if (tar.error) throw tar.error;
    if (tar.status !== 0) throw new Error(`tar exited with status ${tar.status}`);
    const extracted = path.join(extractDir, binaryName());
    if (!isExecutable(extracted)) {
      throw new Error(`Release archive did not contain executable ${binaryName()}`);
    }
    const installed = copyRuntime(extracted, options);
    installed.metadata.source = "github-release";
    installed.metadata.url = asset.url;
    installed.metadata.sha256 = asset.sha256;
    writeMetadata(installed.metadata, options);
    return installed;
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
}

async function ensureRuntime(options = {}) {
  const status = inspectRuntime(options);
  if (status.selected && status.selected.source === "managed") {
    return status.selected.path;
  }
  if (status.selected && status.selected.source === "explicit") {
    return status.selected.path;
  }

  const localCandidate = status.candidates.find(
    (candidate) => candidate.ok && candidate.adoptable && candidate.source === "repo"
  );
  if (localCandidate) {
    if (options.adoptLocal === false) return localCandidate.path;
    return copyRuntime(localCandidate.path, options).path;
  }

  if (options.allowDownload) {
    const downloaded = await downloadRuntime(options);
    return downloaded.path;
  }

  if (status.pathMatch) {
    throw new Error(
      [
        `A matching remem ${status.expectedVersion} exists on PATH at ${status.pathMatch.path},`,
        "but the plugin does not adopt PATH binaries silently.",
        "Run `node plugins/remem/scripts/remem-runtime.js install --adopt-path` to copy it into plugin storage,",
        "or set REMEM_BINARY explicitly for development."
      ].join(" ")
    );
  }

  throw new Error(runtimeMissingMessage(status));
}

function runtimeMissingMessage(status) {
  const rejected = status.candidates
    .filter((candidate) => candidate.executable && !candidate.ok)
    .map((candidate) => candidate.reason)
    .filter(Boolean);
  return [
    `Unable to find plugin-managed remem ${status.expectedVersion}.`,
    `Plugin data: ${status.pluginData}.`,
    "Build this checkout with `cargo build --release`, then run `node plugins/remem/scripts/remem-runtime.js install`,",
    "or publish checked release assets in `plugins/remem/runtimes/remem-releases.json`.",
    rejected.length ? `Rejected candidates: ${rejected.join(" | ")}` : "No executable candidates were found."
  ].join(" ");
}

async function installRuntime(options = {}) {
  const status = inspectRuntime(options);
  const adoptPath = options.adoptPath === true;
  const matchingManaged = status.candidates.find(
    (candidate) => candidate.ok && candidate.source === "managed"
  );
  if (matchingManaged && !options.force) return matchingManaged.path;

  const explicit = status.candidates.find((candidate) => candidate.ok && candidate.source === "explicit");
  if (explicit) return copyRuntime(explicit.path, options).path;

  const local = status.candidates.find((candidate) => candidate.ok && candidate.source === "repo");
  if (local) return copyRuntime(local.path, options).path;

  const matchingPath = status.candidates.find((candidate) => candidate.ok && candidate.source === "path");
  if (matchingPath && adoptPath) return copyRuntime(matchingPath.path, options).path;

  if (!options.allowDownload) {
    throw new Error(runtimeMissingMessage(status));
  }

  return downloadRuntime(options).then((installed) => installed.path);
}

function runtimeEnv(options = {}) {
  return {
    ...process.env,
    ...(options.env || {}),
    REMEM_PLUGIN_ROOT: pluginRoot(options),
    REMEM_PLUGIN_DATA: pluginDataDir(options)
  };
}

function runRemem(args, options = {}) {
  const env = runtimeEnv(options);
  const bin = options.binary || fs.realpathSync(ensureRuntimeSync(options));
  const result = spawnSync(bin, args, {
    stdio: "inherit",
    env,
    ...options.spawnOptions
  });

  if (result.error) {
    throw result.error;
  }
  return result.status === null ? 1 : result.status;
}

async function runRememAsync(args, options = {}) {
  const env = runtimeEnv(options);
  const runtimeOptions = {
    ...options,
    allowDownload: options.allowDownload !== false
  };
  const resolved = options.binary || (await ensureRuntime(runtimeOptions));
  const bin = fs.realpathSync(resolved);
  const result = spawnSync(bin, args, {
    stdio: "inherit",
    env,
    ...options.spawnOptions
  });

  if (result.error) {
    throw result.error;
  }
  return result.status === null ? 1 : result.status;
}

function ensureRuntimeSync(options = {}) {
  const status = inspectRuntime(options);
  if (status.selected && ["managed", "explicit"].includes(status.selected.source)) {
    return status.selected.path;
  }
  const localCandidate = status.candidates.find(
    (candidate) => candidate.ok && candidate.adoptable && candidate.source === "repo"
  );
  if (localCandidate) {
    if (options.adoptLocal === false) return localCandidate.path;
    return copyRuntime(localCandidate.path, options).path;
  }
  throw new Error(runtimeMissingMessage(status));
}

function humanStatus(status) {
  const lines = [
    `expected: remem ${status.expectedVersion}`,
    `plugin_root: ${status.pluginRoot}`,
    `plugin_data: ${status.pluginData}`,
    `managed_binary: ${status.managedBinary}`,
    `selected: ${status.selected ? `${status.selected.source} ${status.selected.path}` : "none"}`
  ];
  for (const candidate of status.candidates) {
    if (!candidate.executable && !candidate.exists) continue;
    const state = candidate.ok ? "ok" : "rejected";
    lines.push(`${state}: ${candidate.source} ${candidate.path}${candidate.reason ? ` (${candidate.reason})` : ""}`);
  }
  return `${lines.join("\n")}\n`;
}

async function main(argv) {
  const [command = "status", ...args] = argv;
  const options = {
    force: args.includes("--force"),
    adoptPath: args.includes("--adopt-path"),
    allowDownload: !args.includes("--no-download")
  };
  if (command === "status") {
    const status = inspectRuntime();
    if (args.includes("--json")) {
      process.stdout.write(`${JSON.stringify(status, null, 2)}\n`);
    } else {
      process.stdout.write(humanStatus(status));
    }
    return 0;
  }
  if (command === "install") {
    const installed = await installRuntime(options);
    process.stdout.write(`${installed}\n`);
    return 0;
  }
  if (command === "path") {
    process.stdout.write(`${ensureRuntimeSync()}\n`);
    return 0;
  }
  if (command === "self-test") {
    const installed = ensureRuntimeSync();
    const version = inspectVersion(installed);
    if (!version.ok) throw new Error(version.reason);
    process.stdout.write(`${version.output}\n`);
    return 0;
  }
  throw new Error(`Unknown remem runtime command: ${command}`);
}

module.exports = {
  binaryName,
  candidateEntries,
  codesignRuntimeIfNeeded,
  copyRuntime,
  ensureRuntime,
  ensureRuntimeSync,
  expectedVersion,
  installRuntime,
  inspectRuntime,
  inspectVersion,
  isExecutable,
  managedBinaryPath,
  pathCandidates,
  pluginDataDir,
  pluginRoot,
  repoCandidates,
  repoRoot,
  runRemem,
  runRememAsync,
  runtimeMetadataPath,
  shouldCodesignRuntime,
  versionMismatchMessage
};

if (require.main === module) {
  main(process.argv.slice(2))
    .then((status) => process.exit(status))
    .catch((error) => {
      process.stderr.write(`${error.message}\n`);
      process.exit(1);
    });
}
