import fs from "node:fs";
import { spawn } from "node:child_process";
import path from "node:path";

const entry = process.env.TESHI_CODEBUDDY_ENTRY;
const args = JSON.parse(process.env.TESHI_CODEBUDDY_ARGS_JSON ?? "[]");
const logPath = process.env.TESHI_CODEBUDDY_PROTOCOL_LOG;
if (!entry || !logPath) {
  process.stderr.write("missing CodeBuddy proxy configuration\n");
  process.exit(2);
}

const log = (event) => {
  fs.appendFileSync(
    logPath,
    `${JSON.stringify({ time: new Date().toISOString(), proxyPid: process.pid, ...event })}\n`,
  );
};
const agentEnv = {
  ...process.env,
  APPDATA: process.env.TESHI_CODEBUDDY_APPDATA,
  LOCALAPPDATA: process.env.TESHI_CODEBUDDY_LOCALAPPDATA,
};
// Resolve the installed profile only at launch. Never serialize credentials
// into generated sidecars or protocol evidence.
const manifest = path.join(agentEnv.LOCALAPPDATA, "teshi", "acp", "installed.json");
const profile = JSON.parse(fs.readFileSync(manifest, "utf8"))
  .find((agent) => agent.id === "codebuddy-code");
if (!profile) throw new Error("CodeBuddy ACP profile is unavailable");
Object.assign(agentEnv, profile.env ?? {});
for (const [name, source] of Object.entries(profile.env_from ?? {})) {
  if (!(source in process.env)) throw new Error(`Missing CodeBuddy environment reference: ${source}`);
  agentEnv[name] = process.env[source];
}
const child = spawn(process.execPath, [entry, ...args], {
  stdio: ["pipe", "pipe", "pipe"],
  env: agentEnv,
});
log({ event: "spawn", childPid: child.pid });

function observe(stream, direction) {
  let pending = "";
  stream.on("data", (chunk) => {
    pending += chunk.toString();
    let newline;
    while ((newline = pending.indexOf("\n")) >= 0) {
      const line = pending.slice(0, newline);
      pending = pending.slice(newline + 1);
      try {
        const message = JSON.parse(line);
        const update = message.params?.update;
        log({
          direction,
          id: message.id,
          method: message.method,
          update: update?.sessionUpdate,
          characters: update?.content?.text?.length,
          toolStatus: update?.status,
          stopReason: message.result?.stopReason,
          errorCode: message.error?.code,
          errorCategory: message.error?.data?.category,
          hasResult: message.result !== undefined,
          bytes: Buffer.byteLength(line),
        });
      } catch {
        log({ direction, event: "non-json", bytes: Buffer.byteLength(line) });
      }
    }
  });
}

observe(process.stdin, "to-agent");
observe(child.stdout, "from-agent");
process.stdin.pipe(child.stdin);
child.stdout.pipe(process.stdout);
child.stderr.on("data", (chunk) => log({ event: "stderr", bytes: chunk.length }));
child.on("error", (error) => {
  log({ event: "spawn-error", code: error.code });
  process.exitCode = 2;
});
child.on("exit", (code, signal) => {
  log({ event: "agent-exit", code, signal });
  process.exit(code ?? 1);
});
process.stdin.on("end", () => {
  log({ event: "stdin-ended" });
  child.kill();
});
