# ACP model preferences

Teshi remembers the last successfully selected model for each installed ACP
agent. New conversations and reconnects restore that choice after session
creation, before sending a prompt. Existing conversations keep their own model.

Preferences live in `model-preferences/<hex-encoded-agent-id>.json` beneath the
ACP store (including a `TESHI_ACP_STORE_DIR` override). Only the confirmed model
value is saved. Rejected changes do not replace the saved preference. Other
session options and Teshi approval policies are not persisted by this mechanism.

If a saved model is unavailable or rejected, Teshi reports the restoration error
and keeps the agent's default. A restoration timeout leaves the remote model
uncertain and requires reconnecting before a prompt can be sent. Corrupt files
are reported rather than silently rewritten. Selecting an available model
successfully replaces the preference; deleting the agent's preference file
restores default behavior on the next connection.

Direct environment-only launches without an installed agent ID do not have a
per-agent preference file.
