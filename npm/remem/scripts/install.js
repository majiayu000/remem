#!/usr/bin/env node

"use strict";

const { spawnSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");

const { platformKey } = require("./platform");

const VERSION = "0.4.5";
const BASE_URL = `https://github.com/majiayu000/remem/releases/download/v${VERSION}`;
const ASSETS = {
  "darwin-arm64": {
    file: "remem-darwin-arm64.tar.gz",
    sha256: "35ffef27827c66e96c60149524c20e3af572c75f8f5d597eb740906f97255c22",
  },
  "darwin-x64": {
    file: "remem-darwin-x64.tar.gz",
    sha256: "71d13ddd4935dd9e13e37c8ec1ff081934fb7a12c1d83c6c7b9b425a7acaab4d",
  },
  "linux-arm64": {
    file: "remem-linux-arm64.tar.gz",
    sha256: "27ac660646801aa14d89cfcab4fc626d6dbb7992dfc43a4e9000fbd971a4dc61",
  },
  "linux-x64": {
    file: "remem-linux-x64.tar.gz",
    sha256: "91106ddecc684ab223f343caf089312ca61a39c42e323ea5c6ea57fb82e5100b",
  },
};

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

async function main() {
  if (process.env.REMEM_NPM_SKIP_DOWNLOAD === "1") {
    console.log("Skipping remem binary download because REMEM_NPM_SKIP_DOWNLOAD=1");
    return;
  }

  const key = platformKey();
  const asset = ASSETS[key];
  const vendorDir = path.join(__dirname, "..", "vendor", key);
  const binary = path.join(vendorDir, "remem");
  if (process.argv.includes("--skip-existing") && fs.existsSync(binary)) {
    return;
  }

  fs.rmSync(vendorDir, { recursive: true, force: true });
  fs.mkdirSync(vendorDir, { recursive: true, mode: 0o755 });

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "remem-npm-"));
  const archive = path.join(tmpDir, asset.file);
  const url = `${BASE_URL}/${asset.file}`;

  try {
    console.log(`Downloading remem ${VERSION} for ${key}`);
    await download(url, archive);
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
    console.log(`Installed remem binary to ${binary}`);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
