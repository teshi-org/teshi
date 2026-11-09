# ACP session preferences

Teshi remembers the last successfully confirmed session options for each
installed ACP agent: the model, the other agent-advertised options (session
mode, thinking level, agent-side approval, and any other select or boolean
option), and Teshi's local approval mode (Manual/Auto/Bypass). New
conversations and reconnects restore these choices after session creation,
before sending a prompt. Existing conversations keep their own values.

Preferences live beneath the ACP store (including a `TESHI_ACP_STORE_DIR`
override), keyed by the hex-encoded agent ID:

- `model-preferences/<hex-encoded-agent-id>.json`: the confirmed model value.
- `session-options/<hex-encoded-agent-id>.json`: an object mapping each other
  confirmed option ID to its value.
- `approval-preferences/<hex-encoded-agent-id>.json`: the local approval mode.

Only values the agent confirmed are saved. Rejected changes do not replace a
saved preference. The model is restored first, because it can change which
values the agent offers for dependent options such as thinking levels; the
other options then follow the agent's advertised order.

If a saved value is unavailable or rejected, Teshi reports it and keeps the
agent's default for that option. A restoration timeout leaves the remote value
uncertain and requires reconnecting before a prompt can be sent. Corrupt files
are reported rather than silently rewritten. Selecting a value successfully
replaces its preference; deleting an agent's preference files restores default
behavior on the next connection.

Direct environment-only launches without an installed agent ID do not have
per-agent preference files, and their approval mode stays application-wide.
