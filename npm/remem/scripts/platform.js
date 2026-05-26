"use strict";

const os = require("node:os");

function platformKey() {
  const platform = os.platform();
  const arch = os.arch();
  if (platform === "darwin" && arch === "arm64") return "darwin-arm64";
  if (platform === "darwin" && arch === "x64") return "darwin-x64";
  if (platform === "linux" && arch === "arm64") return "linux-arm64";
  if (platform === "linux" && arch === "x64") return "linux-x64";
  throw new Error(`Unsupported platform: ${platform}/${arch}`);
}

module.exports = { platformKey };
