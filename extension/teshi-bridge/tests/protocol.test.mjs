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

test("privileged Chromium permissions are optional and requested only from popup gestures", () => {
  assert.deepEqual(manifest.optional_permissions.sort(), ["contentSettings", "cookies", "management"]);
  assert.match(background, /async function optionalPermissionStatus/);
  assert.match(background, /chrome\.permissions\.onAdded/);
  assert.match(background, /chrome\.permissions\.onRemoved/);
  const popup = readFileSync(resolve(extensionDir, "popup.js"), "utf8");
  assert.match(popup, /button\.addEventListener\("click"/);
  assert.match(popup, /chrome\.permissions\.request/);
  assert.doesNotMatch(background, /chrome\.permissions\.request/);
});

test("privileged JavaScript and raw CDP stay target-scoped and result-bounded", () => {
  assert.match(background, /async function executePrivilegedJavascript/);
  assert.match(background, /async function executePrivilegedCdp/);
  assert.match(background, /pageContextRevision\(tab\.id\)/);
  assert.match(background, /MAX_PRIVILEGED_RESULT_BYTES/);
  assert.match(background, /code: "browser_result_too_large"/);
  assert.match(background, /cmd === "execute_privileged_javascript"/);
  assert.match(background, /cmd === "execute_privileged_cdp"/);
});

test("cookie, content-setting, and extension metadata operations stay permission and scope gated", () => {
  assert.match(background, /async function listBrowserCookies/);
  assert.match(background, /chrome\.cookies\.getAll\(\{ url: tab\.url \}\)/);
  assert.match(background, /value_redacted: !includeValues/);
  assert.match(background, /partition_key:/);
  assert.match(background, /async function accessBrowserContentSetting/);
  assert.match(background, /primaryPattern = `\$\{parsed\.origin\}\/\*`/);
  assert.match(background, /const CONTENT_SETTING_APIS = Object\.freeze/);
  assert.match(background, /async function listBrowserExtensions/);
  assert.match(background, /mutations_enabled: false/);
  assert.doesNotMatch(background, /chrome\.management\.(setEnabled|uninstall)\(/);
  assert.match(background, /phasedFeatures\(optionalPermissions\)/);
});

test("service worker implements protocol-v1 identity and routed operations", () => {
  assert.match(background, /const PROTOCOL_VERSION = 1/);
  assert.match(background, /chrome\.storage\.local\.get/);
  assert.match(background, /crypto\.randomUUID/);
  assert.match(background, /extension_instance_id/);
  assert.match(background, /verifyPlaywrightLocators/);
  assert.match(background, /captureBrowserEvidence/);
  assert.match(background, /page_context_revision/);
  assert.match(background, /p0\.control/);
  assert.match(background, /p1\.observability_artifacts/);
  assert.match(background, /supported_actions/);
  assert.match(background, /supported_operations/);
  assert.match(background, /direct_command/);
  assert.match(background, /ws\.send\(JSON\.stringify\(reply\)\)/);
});

test("pausing screencast preserves the correlated direct command socket", () => {
  const pauseBody = background.match(
    /async function pauseScreencast\(\) \{(?<body>[\s\S]*?)\n\}/,
  )?.groups?.body;
  assert.ok(pauseBody, "pauseScreencast implementation missing");
  assert.doesNotMatch(pauseBody, /closeStreamWebSocket\(\)/);
  const stopBody = background.match(
    /async function stopStreamSession\(\) \{(?<body>[\s\S]*?)\n\}\n\nasync function startStreamSession/,
  )?.groups?.body;
  assert.ok(stopBody, "stopStreamSession implementation missing");
  assert.doesNotMatch(stopBody, /closeStreamWebSocket\(\)/);
  const resumeBody = background.match(
    /async function resumeScreencast\(\) \{(?<body>[\s\S]*?)\n\}\n\nasync function attachActiveTab/,
  )?.groups?.body;
  assert.ok(resumeBody, "resumeScreencast implementation missing");
  assert.doesNotMatch(resumeBody, /startStreamSession\(/);
  assert.match(resumeBody, /Page\.startScreencast/);
  assert.doesNotMatch(
    background,
    /await detachIfNeeded\(\);\s*const startedAt = Date\.now\(\)/,
  );
  const detachListener = background.match(
    /chrome\.debugger\.onDetach\.addListener\([^]*?\n\}\);/,
  )?.[0];
  assert.ok(detachListener, "debugger detach listener missing");
  assert.doesNotMatch(detachListener, /closeStreamWebSocket\(\)/);
});

test("console capture uses target-scoped CDP events and bounded broker transport", () => {
  assert.match(background, /async function startConsoleCapture/);
  assert.match(background, /async function stopConsoleCapture/);
  assert.match(background, /Runtime\.consoleAPICalled/);
  assert.match(background, /Log\.entryAdded/);
  assert.match(background, /type: "console_event"/);
  assert.match(background, /consoleCaptureTabIds\.has\(source\.tabId\)/);
  assert.match(background, /cmd === "start_console_capture"/);
  assert.match(background, /cmd === "stop_console_capture"/);
});

test("network capture emits metadata and retrieves bodies only on explicit command", () => {
  assert.match(background, /async function startNetworkCapture/);
  assert.match(background, /Network\.requestWillBeSent/);
  assert.match(background, /Network\.responseReceived/);
  assert.match(background, /type: "network_event"/);
  assert.match(background, /metadata_only: true/);
  assert.match(background, /cmd === "get_network_response_body"/);
  assert.match(background, /Network\.getResponseBody/);
});

test("artifact transport is bounded and element clips use page coordinates", () => {
  assert.match(background, /const MAX_ARTIFACT_BYTES = 50 \* 1024 \* 1024/);
  assert.match(background, /artifactBytes > MAX_ARTIFACT_BYTES/);
  assert.match(background, /let pageOffsetX = window\.scrollX/);
  assert.match(background, /let pageOffsetY = window\.scrollY/);
  assert.match(background, /x:rect\.left \+ pageOffsetX/);
  assert.match(background, /y:rect\.top \+ pageOffsetY/);
});

test("loopback HTTP mutations use the discovered broker token", () => {
  assert.match(background, /function cacheBrokerToken/);
  assert.match(background, /searchParams\.get\("token"\)/);
  assert.match(background, /authenticatedUrl\.searchParams\.set\("token", cachedBrokerToken\)/);
  assert.match(background, /"X-Teshi-Broker-Token": cachedBrokerToken/);
  assert.match(background, /res = await bridgePost\(HEARTBEAT_URL/);
  assert.doesNotMatch(background, /fetch\(HEARTBEAT_URL/);
});

test("mutation monitoring captures bounded summaries around one action dispatch", () => {
  assert.match(background, /async function captureMonitoringSummary/);
  assert.match(background, /unique\.length >= 100/);
  assert.match(background, /function diffMonitoringSummaries/);
  const executeBranch = background.match(
    /if \(cmd === "get_page_snapshot"\)[\s\S]*?else if \(cmd === "activate_tab"\)/,
  )?.[0];
  assert.ok(executeBranch, "execute locator branch missing");
  assert.equal((executeBranch.match(/await executeLocator\(/g) || []).length, 1);
  assert.match(executeBranch, /beforeSummary/);
  assert.match(executeBranch, /afterSummary/);
});

test("file upload resolves one actionable input and uses CDP file assignment", () => {
  assert.match(background, /async function setFileInputFiles/);
  assert.match(background, /DOM\.enable/);
  assert.match(background, /DOM\.describeNode/);
  assert.match(background, /DOM\.setFileInputFiles/);
  assert.match(background, /element\.type \|\| ''\)\.toLowerCase\(\) !== 'file'/);
  assert.match(background, /uploaded_files: files\.length/);
  assert.doesNotMatch(background, /uploaded_paths/);
});

test("tab activation focuses windows only by explicit opt-in and grouping degrades non-fatally", () => {
  assert.match(background, /async function activateTab\(tabId, windowId = null, focusWindow = false\)/);
  assert.match(background, /if \(focusWindow\) \{\s*windowFocused = Boolean\(await chrome\.windows\.update/);
  assert.match(background, /activateTab\(target\?\.tab_id \?\? tabId, target\?\.window_id \?\? msg\.window_id, msg\.focus_window\)/);
  assert.match(background, /code: "tab_group_unavailable"/);
  assert.match(background, /ok: true,\s*organized: false/);
});

test("phased fixtures distinguish safe, artifact, and privileged availability", () => {
  const phased = fixtures.phased;
  assert.deepEqual(phased.p0_only_heartbeat.features, [
    { feature: "p0.control", available: true },
  ]);
  assert.ok(
    phased.p0_p1_heartbeat.features.some(
      (entry) => entry.feature === "p1.observability_artifacts" && entry.available,
    ),
  );
  assert.ok(phased.p0_p1_heartbeat.supported_operations.includes("list_console_events"));
  assert.ok(phased.p0_p1_heartbeat.supported_operations.includes("get_network_request_detail"));
  assert.equal(phased.p2_optional_permissions_heartbeat.optional_permissions.cookies, false);
  assert.equal(
    phased.incompatible_feature_request.required_feature,
    "p1.observability_artifacts",
  );
});

test("legacy fixtures cover every migration boundary", () => {
  const legacy = fixtures.legacy;
  for (const boundary of ["heartbeat", "command", "response", "frame_meta", "cdp_endpoint"]) {
    assert.ok(legacy[boundary], `missing ${boundary} fixture`);
  }
  assert.equal(legacy.command.request_id, legacy.response.request_id);
  assert.equal(legacy.cdp_endpoint.mode, "chrome");
});
