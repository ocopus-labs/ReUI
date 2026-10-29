// Runs ReUI's OpenCode plugin (crates/reui-bridge/src/opencode-plugin.js) against a fake
// OpenCode: a fake `Bun.spawn` stands in for reui-bridge (answering as the island would)
// and a fake `client` records what the plugin posts back. No OpenCode, Bun or ReUI needed.
//
//   node tools/opencode-plugin-test.mjs
//
// The event and reply shapes are OpenCode 1.19's (packages/schema/src/v1/{permission,
// question}.ts, server/routes/instance/httpapi/groups/{permission,question,session}.ts).

import assert from "node:assert/strict";
import { copyFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const source = new URL("../crates/reui-bridge/src/opencode-plugin.js", import.meta.url);
const copy = join(mkdtempSync(join(tmpdir(), "reui-plugin-")), "reui.mjs");
copyFileSync(source, copy);
const { ReUIPlugin, ...others } = await import(pathToFileURL(copy));
assert.deepEqual(Object.keys(others), [], "OpenCode runs every export as a plugin");

/// The bridge: each spawn records its payload and answers with `next()` (a JSON object,
/// null for no output, or "hang" to wait until killed).
let spawned = [];
let answers = [];
globalThis.Bun = {
  spawn(argv, opts) {
    let input = "";
    let kill;
    const killed = new Promise((resolve) => (kill = resolve));
    const answer = opts.stdout === "pipe" ? answers.shift() : undefined;
    const proc = {
      argv,
      opts,
      killed: false,
      stdin: { write: (s) => (input += s), end: () => spawned.push({ payload: JSON.parse(input), proc }) },
      stdout:
        opts.stdout === "pipe"
          ? new ReadableStream({
              async start(c) {
                if (answer === "hang") await killed;
                else if (answer) c.enqueue(new TextEncoder().encode(JSON.stringify(answer)));
                c.close();
              },
            })
          : null,
      kill() {
        proc.killed = true;
        kill();
      },
    };
    return proc;
  },
};

function fakeClient({ http = true, failPost = false } = {}) {
  const calls = [];
  const client = {
    postSessionIdPermissionsPermissionId: async (o) => (calls.push(["v1", o]), { data: true }),
  };
  if (http) client._client = { post: async (o) => (calls.push(["post", o]), failPost ? { error: { name: "NotFoundError" } } : { data: true }) };
  return { client, calls };
}

const settle = () => new Promise((r) => setTimeout(r, 20));
const allow = { hookSpecificOutput: { hookEventName: "PermissionRequest", decision: { behavior: "allow" } } };
const always = { hookSpecificOutput: { hookEventName: "PermissionRequest", decision: { behavior: "allow", updatedPermissions: [{ type: "opencode" }] } } };
const deny = (message) => ({ hookSpecificOutput: { hookEventName: "PermissionRequest", decision: { behavior: "deny", message } } });
const asked = { id: "per_1", sessionID: "ses_1", permission: "bash", patterns: ["git push"], metadata: { description: "Push" }, always: ["git push *"] };
const question = { id: "que_1", sessionID: "ses_1", questions: [{ question: "Which?", header: "Pick", options: [{ label: "A", description: "" }], multiple: false }] };

async function plugin(fake) {
  spawned = [];
  return ReUIPlugin({ client: fake.client, directory: "D:/proj", serverUrl: new URL("http://localhost:4096") });
}

const tests = {
  async "a permission goes to the island and its answer to OpenCode's v1 route"() {
    const fake = fakeClient();
    const hooks = await plugin(fake);
    answers = [allow];
    await hooks.event({ event: { type: "permission.asked", properties: asked } });
    await settle();
    const { payload, proc } = spawned[0];
    assert.deepEqual(proc.argv.slice(1), ["opencode"]);
    assert.equal(payload.hook_event_name, "PermissionRequest");
    assert.equal(payload.tool_name, "bash");
    assert.deepEqual(payload.tool_input, { description: "Push", command: "git push" });
    assert.deepEqual(payload.permission_suggestions, [{ type: "opencode", patterns: ["git push *"] }]);
    assert.deepEqual(fake.calls, [["v1", { path: { id: "ses_1", permissionID: "per_1" }, body: { response: "once" } }]]);
  },
  async "always and deny with a message"() {
    let fake = fakeClient();
    let hooks = await plugin(fake);
    answers = [always];
    await hooks.event({ event: { type: "permission.asked", properties: asked } });
    await settle();
    assert.equal(fake.calls[0][1].body.response, "always");

    fake = fakeClient();
    hooks = await plugin(fake);
    answers = [deny("Use the release script")];
    await hooks.event({ event: { type: "permission.asked", properties: asked } });
    await settle();
    assert.deepEqual(fake.calls, [["post", { url: "/permission/{requestID}/reply", path: { requestID: "per_1" }, body: { reply: "reject", message: "Use the release script" }, headers: { "Content-Type": "application/json" } }]]);

    // Without the HTTP client the deny still goes, without its message.
    fake = fakeClient({ http: false });
    hooks = await plugin(fake);
    answers = [deny("x")];
    await hooks.event({ event: { type: "permission.asked", properties: asked } });
    await settle();
    assert.deepEqual(fake.calls, [["v1", { path: { id: "ses_1", permissionID: "per_1" }, body: { response: "reject" } }]]);
  },
  async "no answer leaves OpenCode's prompt alone"() {
    const fake = fakeClient();
    const hooks = await plugin(fake);
    answers = [null, { hookSpecificOutput: { decision: { behavior: "ask" } } }];
    await hooks.event({ event: { type: "permission.asked", properties: asked } });
    await hooks.event({ event: { type: "permission.asked", properties: { ...asked, id: "per_2" } } });
    await settle();
    assert.deepEqual(fake.calls, []);
  },
  async "answered in OpenCode: the bridge is stopped and nothing is posted"() {
    const fake = fakeClient();
    const hooks = await plugin(fake);
    answers = ["hang", "hang"];
    await hooks.event({ event: { type: "permission.asked", properties: asked } });
    await hooks.event({ event: { type: "question.asked", properties: question } });
    await settle();
    await hooks.event({ event: { type: "permission.replied", properties: { sessionID: "ses_1", requestID: "per_1", reply: "once" } } });
    await hooks.event({ event: { type: "question.rejected", properties: { sessionID: "ses_1", requestID: "que_1" } } });
    await settle();
    assert.ok(spawned.every((s) => s.proc.killed));
    assert.deepEqual(fake.calls, []);
  },
  async "a question's answers go to /question/{id}/reply"() {
    const fake = fakeClient();
    const hooks = await plugin(fake);
    answers = [{ hookSpecificOutput: { hookEventName: "Question", answers: [["A"]] } }];
    await hooks.event({ event: { type: "question.asked", properties: question } });
    await settle();
    const { payload, proc } = spawned[0];
    assert.equal(proc.opts.stdout, "pipe");
    assert.deepEqual(payload, { session_id: "ses_1", cwd: "D:/proj", hook_event_name: "Question", request_id: "que_1", questions: question.questions, answerable: true });
    assert.deepEqual(fake.calls, [["post", { url: "/question/{requestID}/reply", path: { requestID: "que_1" }, body: { answers: [["A"]] }, headers: { "Content-Type": "application/json" } }]]);
  },
  async "a lost race (NotFound) or a failing client is ignored"() {
    const fake = fakeClient({ failPost: true });
    const hooks = await plugin(fake);
    answers = [{ hookSpecificOutput: { answers: [["A"]] } }, deny("no")];
    await hooks.event({ event: { type: "question.asked", properties: question } });
    await hooks.event({ event: { type: "permission.asked", properties: asked } });
    await settle();
    // The deny's message route failed: the v1 reject follows.
    assert.deepEqual(fake.calls.map((c) => c[0]), ["post", "post", "v1"]);
    const broken = { client: { _client: { post: () => Promise.reject(new Error("down")) }, postSessionIdPermissionsPermissionId: () => Promise.reject(new Error("down")) } };
    const h = await plugin(broken);
    answers = [{ hookSpecificOutput: { answers: [["A"]] } }, allow];
    await h.event({ event: { type: "question.asked", properties: question } });
    await h.event({ event: { type: "permission.asked", properties: asked } });
    await settle();
  },
  async "without a way to answer, a question is shown read-only"() {
    const fake = fakeClient({ http: false });
    const hooks = await plugin(fake);
    await hooks.event({ event: { type: "question.asked", properties: question } });
    await settle();
    assert.equal(spawned[0].payload.answerable, false);
    assert.equal(spawned[0].proc.opts.stdout, "ignore", "nothing waits");
  },
  async "a broken bridge never throws into OpenCode"() {
    const real = globalThis.Bun.spawn;
    globalThis.Bun.spawn = () => {
      throw new Error("no such file");
    };
    try {
      const hooks = await plugin(fakeClient());
      await hooks.event({ event: { type: "permission.asked", properties: asked } });
      await hooks.event({ event: { type: "session.idle", properties: { sessionID: "ses_1" } } });
      await hooks["tool.execute.before"]({ tool: "bash", sessionID: "ses_1", callID: "c1" }, { args: {} });
      await hooks.event({ event: {} });
      await settle();
    } finally {
      globalThis.Bun.spawn = real;
    }
  },
  async "status events keep their hook shapes"() {
    const hooks = await plugin(fakeClient());
    await hooks["tool.execute.before"]({ tool: "bash", sessionID: "ses_1", callID: "c1" }, { args: { command: "ls" } });
    await hooks["tool.execute.after"]({ tool: "bash", sessionID: "ses_1", callID: "c1" });
    await hooks.event({ event: { type: "session.idle", properties: { sessionID: "ses_1" } } });
    assert.deepEqual(
      spawned.map((s) => [s.payload.hook_event_name, s.payload.tool_use_id]),
      [["PreToolUse", "c1"], ["PostToolUse", "c1"], ["Stop", undefined]],
    );
  },
};

let failed = 0;
process.on("unhandledRejection", (e) => {
  failed++;
  console.error("unhandled rejection:", e);
});
for (const [name, test] of Object.entries(tests)) {
  try {
    await test();
    console.log(`ok   ${name}`);
  } catch (e) {
    failed++;
    console.error(`FAIL ${name}\n${e.stack}`);
  }
}
await settle();
console.log(failed ? `${failed} failed` : "all passed");
process.exit(failed ? 1 : 0);
