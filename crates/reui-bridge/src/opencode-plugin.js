// ReUI for OpenCode: shows OpenCode in the ReUI island and lets you answer its permission
// requests and questions from it. Installed by `reui-bridge install opencode`; removed by
// `reui-bridge uninstall opencode`. Generated file: edits are overwritten on reinstall,
// and ReUI replaces an older copy of it when it starts.
// reui-plugin: 2
//
// Each event is handed to reui-bridge as the same hook-shaped JSON Claude Code and Codex
// send, so ReUI treats all three agents alike. OpenCode (1.3 and later) announces what it
// needs on its event bus (`permission.asked`, `question.asked`) and keeps its own prompt
// up meanwhile: the island's answer goes back through OpenCode's server, and whichever
// answer comes first wins (a late one gets "not found", which is ignored). When you answer
// in OpenCode instead (`*.replied`, `question.rejected`), the waiting bridge is stopped and
// the island's card goes. Every failure leaves OpenCode's own prompt in charge.

const BRIDGE = "__REUI_BRIDGE__";

// Only `ReUIPlugin` is exported: OpenCode runs every export of a plugin file as a plugin.

/// Requests the island has, by OpenCode's request id: the bridge waiting for its answer.
const waiting = new Map();

function spawn(payload, wait) {
  const proc = Bun.spawn([BRIDGE, "opencode"], { stdin: "pipe", stdout: wait ? "pipe" : "ignore", stderr: "ignore" });
  proc.stdin.write(JSON.stringify(payload));
  proc.stdin.end();
  return proc;
}

/// Fire and forget.
function send(payload) {
  try {
    spawn(payload, false);
  } catch {}
}

/// Hands `payload` to the bridge and resolves to the island's answer, or null (no answer,
/// ReUI not running, answered in OpenCode, any error).
async function ask(id, payload) {
  try {
    const proc = spawn(payload, true);
    waiting.set(id, proc);
    const out = await new Response(proc.stdout).text();
    if (waiting.get(id) !== proc) return null;
    waiting.delete(id);
    return out.trim() ? JSON.parse(out) : null;
  } catch {
    waiting.delete(id);
    return null;
  }
}

/// Answered in OpenCode: the island's card goes with its bridge.
function settled(id) {
  const proc = waiting.get(id);
  if (!proc) return;
  waiting.delete(id);
  try {
    proc.kill();
  } catch {}
}

/// The island's permission answer as OpenCode's reply: allow with the agent's own "always"
/// suggestion → always, allow → once, deny → reject (with what the model is told).
function permissionReply(answer) {
  const d = answer?.hookSpecificOutput?.decision;
  if (d?.behavior === "allow") return { reply: d.updatedPermissions?.length ? "always" : "once" };
  if (d?.behavior === "deny") return { reply: "reject", message: typeof d.message === "string" ? d.message : undefined };
  return null;
}

/// The island's question answer: one list of labels (or the typed text) per question.
function questionAnswers(answer) {
  const a = answer?.hookSpecificOutput?.answers;
  return Array.isArray(a) && a.every((q) => Array.isArray(q) && q.every((s) => typeof s === "string")) ? a : null;
}

/// What a permission request is about, in the shape ReUI describes tools from: the
/// metadata OpenCode sends (an edit's `filepath` and `diff`), with the patterns (a shell
/// call's commands, one per line) as the command when it has none.
function permissionInput(p) {
  const meta = p.metadata ?? {};
  const patterns = Array.isArray(p.patterns) ? p.patterns.join("\n") : "";
  return { ...meta, command: meta.command ?? patterns, filePath: meta.filePath ?? meta.filepath };
}

export const ReUIPlugin = async ({ client, directory, serverUrl }) => {
  const at = (sessionID, event, extra = {}) => ({ session_id: sessionID, cwd: directory, hook_event_name: event, ...extra });
  // OpenCode's v1 client has no question methods; its underlying HTTP client reaches every
  // route, in-process when OpenCode runs without a server port.
  const http = typeof client?._client?.post === "function" ? client._client : null;
  // Without it, a server port OpenCode listens on (never the placeholder it reports when
  // it has none).
  const server = !http && serverUrl && String(serverUrl) !== "http://localhost:4096/" ? String(serverUrl) : null;

  async function post(url, path, body) {
    try {
      if (http) return await http.post({ url, path, body, headers: { "Content-Type": "application/json" } });
      if (server) {
        const route = Object.entries(path).reduce((u, [k, v]) => u.replace(`{${k}}`, encodeURIComponent(v)), url);
        return await fetch(new URL(route, server), { method: "POST", headers: { "Content-Type": "application/json", "x-opencode-directory": encodeURIComponent(directory) }, body: JSON.stringify(body) });
      }
    } catch {}
    return null;
  }

  async function permission(p) {
    const answer = await ask(
      p.id,
      at(p.sessionID, "PermissionRequest", {
        tool_name: p.permission,
        tool_input: permissionInput(p),
        // OpenCode can remember the answer for these patterns: ReUI's "Always".
        ...(p.always?.length ? { permission_suggestions: [{ type: "opencode", patterns: p.always }] } : {}),
      }),
    );
    const reply = permissionReply(answer);
    if (!reply) return;
    try {
      // A deny with what the model should hear goes through the newer route, which takes a
      // message; everything else (and a deny it can't send) through the v1 client's.
      if (reply.message && (http || server)) {
        const res = await post("/permission/{requestID}/reply", { requestID: p.id }, reply);
        if (res && !res.error && res.ok !== false) return;
      }
      await client.postSessionIdPermissionsPermissionId({ path: { id: p.sessionID, permissionID: p.id }, body: { response: reply.reply } });
    } catch {}
  }

  async function question(p) {
    const answerable = !!(http || server);
    const payload = at(p.sessionID, "Question", { request_id: p.id, questions: p.questions ?? [], answerable });
    if (!answerable) return send(payload);
    const answers = questionAnswers(await ask(p.id, payload));
    if (answers) await post("/question/{requestID}/reply", { requestID: p.id }, { answers });
  }

  return {
    "chat.message": async (input, output) => {
      const prompt = (output?.parts ?? []).filter((p) => p.type === "text" && !p.synthetic).map((p) => p.text).join("\n");
      if (prompt) send(at(input.sessionID, "UserPromptSubmit", { prompt }));
    },

    "tool.execute.before": async (input, output) => {
      send(at(input.sessionID, "PreToolUse", { tool_name: input.tool, tool_input: output?.args ?? {}, tool_use_id: input.callID }));
    },

    "tool.execute.after": async (input) => {
      send(at(input.sessionID, "PostToolUse", { tool_name: input.tool, tool_use_id: input.callID }));
    },

    // Called without being awaited: what waits for the island runs on its own.
    event: async ({ event }) => {
      const p = event?.properties ?? {};
      switch (event?.type) {
        case "permission.asked":
          void permission(p);
          break;
        case "question.asked":
          void question(p);
          break;
        case "permission.replied":
        case "question.replied":
        case "question.rejected":
          settled(p.requestID);
          break;
        case "session.idle":
          send(at(p.sessionID, "Stop"));
          break;
        case "session.error":
          send(at(p.sessionID, "StopFailure", { error: p.error?.data?.message ?? p.error?.name ?? "" }));
          break;
        case "session.compacted":
          send(at(p.sessionID, "PostCompact"));
          break;
        case "session.deleted":
          send(at(p.info?.id ?? p.sessionID, "SessionEnd"));
          break;
      }
    },
  };
};
