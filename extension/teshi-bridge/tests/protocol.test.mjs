import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const testDir = dirname(fileURLToPath(import.meta.url));
const extensionDir = resolve(testDir, "..");
const repoRoot = resolve(extensionDir, "..", "..");
const manifest = JSON.parse(readFileSync(resolve(extensionDir, "manifest.json"), "utf8"));
const background = readFileSync(resolve(extensionDir, "background.js"), "utf8");
const fixtures = JSON.parse(
  readFileSync(resolve(repoRoot, "resources", "browser_contract_fixtures.json"), "utf8"),
);

test("manifest permits persisted profile-local identity and debugger access", () => {
  assert.equal(manifest.manifest_version, 3);
  assert.ok(manifest.permissions.includes("storage"));
  assert.ok(manifest.permissions.includes("debugger"));
});

test("service worker implements protocol-v1 identity and routed operations", () => {
  assert.match(background, /const PROTOCOL_VERSION = 1/);
  assert.match(background, /chrome\.storage\.local\.get/);
  assert.match(background, /crypto\.randomUUID/);
  assert.match(background, /extension_instance_id/);
  assert.match(background, /verifyPlaywrightLocators/);
  assert.match(background, /captureBrowserEvidence/);
  assert.match(background, /page_context_revision/);
});

test("legacy fixtures cover every migration boundary", () => {
  const legacy = fixtures.legacy;
  for (const boundary of ["heartbeat", "command", "response", "frame_meta", "cdp_endpoint"]) {
    assert.ok(legacy[boundary], `missing ${boundary} fixture`);
  }
  assert.equal(legacy.command.request_id, legacy.response.request_id);
  assert.equal(legacy.cdp_endpoint.mode, "chrome");
});
