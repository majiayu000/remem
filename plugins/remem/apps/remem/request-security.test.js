"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");

const {
  assertLocalHostAllowed,
  isLoopbackHost,
  parseHostHeader
} = require("./request-security");

function requestWithHost(host) {
  return { headers: { host } };
}

function serverOnPort(port) {
  return {
    address() {
      return { port };
    }
  };
}

test("request security recognizes intended loopback Host forms", () => {
  assert.equal(isLoopbackHost("LOCALHOST"), true);
  assert.equal(isLoopbackHost("[::1]"), true);
  assert.equal(isLoopbackHost("127.0.0.1"), true);
  assert.equal(isLoopbackHost("attacker.example"), false);

  assert.deepEqual(parseHostHeader("LOCALHOST:5577"), {
    host: "localhost:5577",
    hostname: "localhost",
    port: 5577
  });
  assert.deepEqual(parseHostHeader("[::1]:5577"), {
    host: "[::1]:5577",
    hostname: "::1",
    port: 5577
  });
});

test("request security accepts loopback Host only on the bound port", () => {
  assert.equal(assertLocalHostAllowed(requestWithHost("localhost:5577"), serverOnPort(5577)).hostname, "localhost");
  assert.equal(assertLocalHostAllowed(requestWithHost("[::1]:5577"), serverOnPort(5577)).hostname, "::1");
  assert.equal(assertLocalHostAllowed(requestWithHost("127.0.0.1"), serverOnPort(80)).hostname, "127.0.0.1");

  for (const host of [undefined, "", "attacker.example:5577", "bad:abc", "127.0.0.1:5578"]) {
    assert.throws(() => assertLocalHostAllowed(requestWithHost(host), serverOnPort(5577)), { statusCode: 403 });
  }
});
