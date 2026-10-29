# Network Panel for fnOS

This `fnos` branch adds a native fnOS package with a Rust radio backend and the existing signal UI. The OpenWrt edition remains on [main](https://github.com/SlAlot/wlmb-server/tree/main).

Version 0.1.0 provides modem identification and read-only signal, serving-cell, carrier and neighbor information. It uses existing NAS administrator credentials and app-center lifecycle controls. Modem writes, SMS, dial-up and host Wi-Fi configuration are not enabled in this version.

See [the branch README](README.md) and [fnOS adaptation notes](docs/FNOS.md). The project's original non-commercial attribution license continues to apply.
