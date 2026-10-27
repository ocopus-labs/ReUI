// ReUI for OpenCode: shows OpenCode in the ReUI island and lets you answer permission
// requests from it. Installed by `reui-bridge install opencode`; removed by
// `reui-bridge uninstall opencode`. Generated file: edits are overwritten on reinstall.
//
// Each event is handed to reui-bridge as the same hook-shaped JSON Claude Code and Codex
// send, so ReUI treats all three agents alike. Permission requests wait for the island's
// answer; if ReUI isn't running or nobody answers, OpenCode asks you itself as usual.

const BRIDGE = "__REUI_BRIDGE__";

function send(payload, wait) {
  try {
    const proc = Bun.spawn([BRIDGE, "opencode"], { stdin: "pipe", stdout: wait ? "pipe" : "ignore", stderr: "ignore" });
    proc.stdin.write(JSON.stringify(payload));
    proc.stdin.end();
    if (!wait) return Promise.resolve(null);
    return new Response(proc.stdout).text().then(
      (out) => {
        try {
          return out.trim() ? JSON.parse(out) : null;
        } catch {
          return null;
        }
      },
      () => null,
    );
  } catch {
    return Promise.resolve(null);
  }
}

export const ReUIPlugin = async ({ directory }) => {
  const at = (sessionID, event, extra = {}) => ({ session_id: sessionID, cwd: directory, hook_event_name: event, ...extra });

  return {
    "chat.message": async (input, output) => {
      const prompt = (output?.parts ?? []).filter((p) => p.type === "text" && !p.synthetic).map((p) => p.text).join("\n");
      if (prompt) send(at(input.sessionID, "UserPromptSubmit", { prompt }));
    },

    "tool.execute.before": async (input, output) => {
      send(at(input.sessionID, "PreToolUse", { tool_name: input.tool, tool_input: output?.args ?? {} }));
    },

    "tool.execute.after": async (input) => {
      send(at(input.sessionID, "PostToolUse", { tool_name: input.tool }));
    },

    "permission.ask": async (input, output) => {
      const meta = input.metadata ?? {};
      const pattern = Array.isArray(input.pattern) ? input.pattern.join(" ") : input.pattern;
      const reply = await send(
        at(input.sessionID, "PermissionRequest", {
          tool_name: input.type,
          tool_input: { ...meta, command: meta.command ?? pattern, description: input.title },
        }),
        true,
      );
      const behavior = reply?.hookSpecificOutput?.decision?.behavior;
      // Anything else leaves OpenCode's own prompt in charge.
      if (behavior === "allow" || behavior === "deny") output.status = behavior;
    },

    event: async ({ event }) => {
      const p = event.properties ?? {};
      switch (event.type) {
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
        case "question.asked":
          send(at(p.sessionID, "Notification", { notification_type: "agent_needs_input" }));
          break;
      }
    },
  };
};
