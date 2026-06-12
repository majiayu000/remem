"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");

const packageJson = require("../package.json");
const { BASE_URL, VERSION, resolveAsset } = require("./install");

test("installer version follows package.json", () => {
  assert.equal(VERSION, packageJson.version);
  assert.equal(
    BASE_URL,
    `https://github.com/majiayu000/remem/releases/download/v${packageJson.version}`
  );
});

test("resolveAsset reads checksummed release manifest asset", () => {
  const manifest = {
    versions: {
      [VERSION]: {
        base_url: "https://example.test/remem/releases/download/custom",
        assets: {
          "linux-x64": {
            file: "remem-linux-x64.tar.gz",
            sha256: "a".repeat(64),
          },
        },
      },
    },
  };

  assert.deepEqual(resolveAsset(manifest, VERSION, "linux-x64"), {
    file: "remem-linux-x64.tar.gz",
    sha256: "a".repeat(64),
    url: "https://example.test/remem/releases/download/custom/remem-linux-x64.tar.gz",
  });
});

test("resolveAsset rejects missing checksums", () => {
  const manifest = {
    versions: {
      [VERSION]: {
        assets: {
          "linux-x64": {
            file: "remem-linux-x64.tar.gz",
            sha256: "not-a-sha",
          },
        },
      },
    },
  };

  assert.throws(
    () => resolveAsset(manifest, VERSION, "linux-x64"),
    /missing a valid sha256/
  );
});
