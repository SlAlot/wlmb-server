# Network Panel 1.0 API

Service wlmb-server, port 1117, protocol NETWORK-PANEL-OPENWRT/1, contract 1. Plugin source is available under a non-commercial attribution license; independent commercial clients are excluded. [Machine-readable contract](api-contract.json) lists every endpoint and parameter.

Use the actual trusted LAN address, not an assumed default. GET /health is public; other endpoints use JSON POST. /api/login accepts the user's root username/password and returns a plugin token. Send Authorization: Bearer token, logout when done. Tokens expire after 1800 seconds and are peer-IP-bound, not system root sessions. Requests are limited to 16384 bytes. HTTP is unencrypted; never expose this service publicly.

Responses are ok/data or ok=false/error. Capabilities, explicit error semantics and field freshness govern behavior. Missing data is not zero. A serving measurement is not another carrier; preserve carrier role, frequency type and candidate bands. A reported API does not prove support on every modem.

radio.refreshDeferred means the primary query was deferred by transient channel contention. An unexpired 15-second measurement retains its original timestamp and partial=true; this does not renew freshness. Expiry, changed identities and parsing failures still invalidate the data.

Reads: status(modem optional), settings, diagnostics, control_result(job), sms_list(modem), sms_forward_settings. status returns cached system/radio/capabilities; router model comes from system.board, not the modem name. Diagnostic data is redacted.

Writes: refresh, lock_status, lock_prepare/execute, host_prepare/execute, at_prepare/execute, sms_refresh, sms_read_state, sms_prepare/execute, sms_settings_prepare/execute, sms_forward_configure. Exact parameters are in the contract. Prepared writes use one-use owner/content/identity-bound confirmations, usually 30 seconds. Long actions return a job; control_result returns pending, done/result or error. Reading a receipt cannot replay the action. Never resubmit an uncertain write automatically.

Locks distinguish band preferences and cell entries. NR_NSA is band-only. NR cell frequency is serving SSB, not CA center; required SCS is protocol-specific. Saved readback, current RF conditions and reboot persistence are separate. No automatic flight mode or resets.

1.0.1 adds `cellFields.NR/LTE` (`bandRequired`, `scsRequired`) to lock status/presets and `presets.cellRanges.NR/LTE` (`band`, `first`, `last`). These describe the selected modem protocol, not a transport-specific feature. Only unique band candidates and explicitly reported SCS may be filled automatically; ambiguous/missing mandatory fields need user input. Older servers without this metadata require conservative fields. Manual PCI/channel entry does not require the cell to appear in a scan.

Host types: wifi_edit, wifi_unify, device_block, traffic_limit. Only verified ordinary LAN APs and plugin-owned rules are mutable. MLO/ambiguous configurations remain gated. Ethernet rateScope=port is not precise per-IP internet accounting.

SMS IDs are fingerprints. Local read state is plugin-owned. The backend/deleteScope fields distinguish modem storage from QModem database records; database deletion does not guarantee SIM deletion. Sends/deletes require preparation and are not replayed. Forwarding defaults OFF, fallback 10 seconds after scan completion (minimum 5 / maximum 300), event-first with backoff. Empty token preserves credentials; clearToken requires forwarding disabled. Initial/re-enabled history is baselined, incomplete multipart messages are not forwarded. See [SMS](SMS.md).

Already queued messages from one modem may share a bounded PushPlus request (up to eight, reduced to fit the escaped payload), without waiting to fill a batch. Counts refer to messages rather than HTTP requests or confirmed user delivery. Every fingerprint is durably reserved before sending; an uncertain batch is not automatically replayed.

Do not log credentials/SMS, bypass capabilities, invent missing values or accept arbitrary shell/UCI/serial paths. The commercial mini-program's paid authorization is separate; this plugin API has no professional-license gate.

[Chinese detailed API](API.zh-CN.md) · [security](SECURITY.md) · [rate sources](LAN-RATES.md)
