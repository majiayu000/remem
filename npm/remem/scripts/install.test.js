"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");

const packageJson = require("../package.json");
const {
  BASE_URL,
  VERSION,
  codesignBinary,
  resolveAsset,
  shouldCodesign,
  smokeBinary,
} = require("./install");

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

test("resolveAsset accepts only platform release archive basenames", () => {
  for (const key of ["darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64"]) {
    const manifest = {
      versions: {
        [VERSION]: {
          assets: {
            [key]: {
              file: `remem-${key}.tar.gz`,
              sha256: "a".repeat(64),
            },
          },
        },
      },
    };

    assert.equal(resolveAsset(manifest, VERSION, key).file, `remem-${key}.tar.gz`);
  }
});

test("resolveAsset rejects unsafe manifest file names", () => {
  for (const file of [
    "../remem-linux-x64.tar.gz",
    "/tmp/remem-linux-x64.tar.gz",
    "nested/remem-linux-x64.tar.gz",
    "https://example.test/remem-linux-x64.tar.gz",
    "remem-darwin-x64.tar.gz",
  ]) {
    const manifest = {
      versions: {
        [VERSION]: {
          assets: {
            "linux-x64": {
              file,
              sha256: "a".repeat(64),
            },
          },
        },
      },
    };

    assert.throws(() => resolveAsset(manifest, VERSION, "linux-x64"), /unsafe file name/);
  }
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

test("codesign is required only for darwin arm64", () => {
  assert.equal(shouldCodesign("darwin", "arm64"), true);
  assert.equal(shouldCodesign("darwin", "x64"), false);
  assert.equal(shouldCodesign("linux", "arm64"), false);
  assert.equal(shouldCodesign("linux", "x64"), false);
});

test("codesignBinary skips non-darwin-arm64 platforms", () => {
  let called = false;
  assert.equal(
    codesignBinary("/tmp/remem", {
      platform: "linux",
      arch: "x64",
      spawnSync: () => {
        called = true;
        return { status: 0 };
      },
    }),
    false
  );
  assert.equal(called, false);
});

test("codesignBinary invokes ad-hoc signing on darwin arm64", () => {
  const calls = [];
  assert.equal(
    codesignBinary("/tmp/remem", {
      platform: "darwin",
      arch: "arm64",
      spawnSync: (cmd, args, options) => {
        calls.push({ cmd, args, options });
        return { status: 0, stdout: "", stderr: "" };
      },
    }),
    true
  );
  assert.deepEqual(calls[0].cmd, "codesign");
  assert.deepEqual(calls[0].args, ["--force", "--sign", "-", "/tmp/remem"]);
});

test("codesignBinary fails closed on darwin arm64 signing failure", () => {
  assert.throws(
    () =>
      codesignBinary("/tmp/remem", {
        platform: "darwin",
        arch: "arm64",
        spawnSync: () => ({ status: 1, stdout: "", stderr: "rejected" }),
      }),
    /codesign failed.*rejected/
  );
});

test("smokeBinary runs remem --version", () => {
  const output = smokeBinary("/tmp/remem", {
    spawnSync: (cmd, args) => {
      assert.equal(cmd, "/tmp/remem");
      assert.deepEqual(args, ["--version"]);
      return { status: 0, stdout: "remem 1.2.3 (schema v34)\n", stderr: "" };
    },
  });
  assert.equal(output, "remem 1.2.3 (schema v34)");
});
