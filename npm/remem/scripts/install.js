#!/usr/bin/env node

"use strict";

const { spawnSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");

const { platformKey } = require("./platform");

const VERSION = require("../package.json").version;
const BASE_URL = `https://github.com/majiayu000/remem/releases/download/v${VERSION}`;

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

function fetchJson(url, redirects = 0) {
  return new Promise((resolve, reject) => {
    const request = https.get(url, (response) => {
      if (
        response.statusCode >= 300 &&
        response.statusCode < 400 &&
        response.headers.location
      ) {
        response.resume();
        if (redirects >= 5) {
          reject(new Error(`Too many redirects while fetching ${url}`));
          return;
        }
        const next = new URL(response.headers.location, url).toString();
        resolve(fetchJson(next, redirects + 1));
        return;
      }

      if (response.statusCode !== 200) {
        response.resume();
        reject(new Error(`Manifest fetch failed with HTTP ${response.statusCode}: ${url}`));
        return;
      }

      let body = "";
      response.setEncoding("utf8");
      response.on("data", (chunk) => {
        body += chunk;
      });
      response.on("end", () => {
        try {
          resolve(JSON.parse(body));
        } catch (error) {
          reject(new Error(`Invalid release manifest JSON from ${url}: ${error.message}`));
        }
      });
    });
    request.on("error", reject);
  });
}

function sha256(file) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(file));
  return hash.digest("hex");
}

function shouldCodesign(platform = process.platform, arch = process.arch) {
  return platform === "darwin" && arch === "arm64";
}

function codesignBinary(binary, options = {}) {
  const platform = options.platform || process.platform;
  const arch = options.arch || process.arch;
  if (!shouldCodesign(platform, arch)) {
    return false;
  }
  const spawn = options.spawnSync || spawnSync;
  const result = spawn("codesign", ["--force", "--sign", "-", binary], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 5000,
  });
  if (result.error) {
    throw new Error(`codesign failed for ${binary}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    const reason = (result.stderr || result.stdout || `exit ${result.status}`).trim();
    throw new Error(`codesign failed for ${binary}: ${reason}`);
  }
  return true;
}

function smokeBinary(binary, options = {}) {
  const spawn = options.spawnSync || spawnSync;
  const result = spawn(binary, ["--version"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 5000,
  });
  if (result.error) {
    throw new Error(`installed remem binary is not executable: ${result.error.message}`);
  }
  if (result.status !== 0) {
    const reason = (result.stderr || result.stdout || `exit ${result.status}`).trim();
    throw new Error(`installed remem binary failed --version: ${reason}`);
  }
  return (result.stdout || result.stderr || "").trim();
}

function expectedAssetFile(key) {
  if (!/^(darwin|linux)-(arm64|x64)$/.test(key)) {
    throw new Error(`Unsupported release asset platform key: ${key}`);
  }
  return `remem-${key}.tar.gz`;
}

function validateAssetFile(file, key) {
  const expected = expectedAssetFile(key);
  if (file !== expected) {
    throw new Error(`Release manifest asset ${key} has unsafe file name: ${file}`);
  }
  return file;
}

function resolveAsset(manifest, version, key) {
  const release = manifest?.versions?.[version];
  const asset = release?.assets?.[key];
  if (!asset || typeof asset.file !== "string") {
    throw new Error(`Release manifest for remem ${version} is missing asset ${key}`);
  }
  const file = validateAssetFile(asset.file, key);
  if (!/^[0-9a-f]{64}$/i.test(asset.sha256 || "")) {
    throw new Error(`Release manifest asset ${key} is missing a valid sha256`);
  }
  const baseUrl = typeof release.base_url === "string" && release.base_url
    ? release.base_url.replace(/\/$/, "")
    : BASE_URL;
  return {
    file,
    sha256: asset.sha256.toLowerCase(),
    url: `${baseUrl}/${file}`,
  };
}

async function main() {
  if (process.env.REMEM_NPM_SKIP_DOWNLOAD === "1") {
    console.log("Skipping remem binary download because REMEM_NPM_SKIP_DOWNLOAD=1");
    return;
  }

  const key = platformKey();
  const vendorDir = path.join(__dirname, "..", "vendor", key);
  const binary = path.join(vendorDir, "remem");
  if (process.argv.includes("--skip-existing") && fs.existsSync(binary)) {
    return;
  }

  fs.rmSync(vendorDir, { recursive: true, force: true });
  fs.mkdirSync(vendorDir, { recursive: true, mode: 0o755 });

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "remem-npm-"));

  try {
    const manifestUrl = `${BASE_URL}/remem-releases.json`;
    const manifest = await fetchJson(manifestUrl);
    const asset = resolveAsset(manifest, VERSION, key);
    const archive = path.join(tmpDir, "remem-release.tar.gz");

    console.log(`Downloading remem ${VERSION} for ${key}`);
    await download(asset.url, archive);
    const actual = sha256(archive);
    if (actual !== asset.sha256) {
      throw new Error(`Checksum mismatch for ${asset.file}: expected ${asset.sha256}, got ${actual}`);
    }

    const tar = spawnSync("tar", ["-xzf", archive, "-C", vendorDir], {
      stdio: "inherit",
    });
    if (tar.error) throw tar.error;
    if (tar.status !== 0) throw new Error(`tar exited with status ${tar.status}`);

    fs.chmodSync(binary, 0o755);
    codesignBinary(binary);
    smokeBinary(binary);
    console.log(`Installed remem binary to ${binary}`);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
}

if (require.main === module) {
  main().catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}

module.exports = {
  BASE_URL,
  VERSION,
  codesignBinary,
  resolveAsset,
  shouldCodesign,
  smokeBinary,
};
