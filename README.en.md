# Network Panel · wlmb-server

[简体中文](README.md) · [Downloads](https://github.com/SlAlot/wlmb-server/releases/latest) · [API](docs/API.en.md)

**Network Panel 1.0.1** by **SlAlot** is a bounded Rust modem service with an independent LuCI JavaScript/CSS interface for OpenWrt. The first public release is 1.0.0. Default port: **1117**. It does not take over dialing. QModem's public identity facts, AT formats and shared interfaces inform the independently implemented adapters.

Source is publicly available under a [custom non-commercial attribution license](LICENSE). **Commercial use requires separate permission**; retain author and source notices. This is source-available, not MIT/GPL or OSI-approved open source. Third-party materials retain their own licenses.

## Install

Download from [Releases](https://github.com/SlAlot/wlmb-server/releases/latest) and verify `SHA256SUMS`. Use the actual firmware's package manager and ABI: 24.10 normally uses IPK/opkg; 25.12 normally uses APK/apk. Currently provided: ARM64 ABI `aarch64_cortex-a53` and `x86_64`. Other architectures/ABIs are not covered. APK here is not an Android application.

```sh
opkg install /tmp/wlmb-server-1.0.1-aarch64.ipk
# On APK firmware, only after verifying the trusted download and checksum:
apk add --allow-untrusted /tmp/wlmb-server-1.0.1-aarch64.apk
```

Use your firmware's own feed for LuCI, rpcd, rpcd-mod-iwinfo and uci dependencies. Open **LuCI → Services → 网络面板**. Installation enables the plugin and boot startup. Packages are unsigned; do not bypass architecture checks or expose port 1117 to the Internet.

## Features and limits

Automatic shared/standalone selection; modem signal, serving cells, carriers, neighbors, operator, subscription and temperature; WiFi and device controls; traffic accounting and limits; typed band/cell locks; SMS/read-state and optional PushPlus forwarding. SMS events are handled first, with a default 10-second fallback. Disabled optional tasks release their resources.

The catalog maps 91 identities and 131 bus profiles. Implemented paths cover temperature for 89 identities, band locks for 81, cell locks for 63, neighbors for 64 and carrier aggregation for 71. **These are code coverage counts, not blanket hardware/firmware certification.** Unknown data stays unknown, uncertain writes are not replayed, and RF resets are never automatic. See [compatibility](docs/MODEM-COMPATIBILITY.md) and [audit](docs/RELEASE-AUDIT.md).

## WeChat and other clients

Open “网络面板助手” in WeChat, join the router WiFi, select **openwrt**, and enter your management address with port 1117. The mini-program is a separate closed-source commercial client. Its paid features do not restrict the plugin WebUI.

This repository contains **only the OpenWrt plugin**, excluding the mini-program, commercial Web client, licensing service, authorization WASM and signing secrets. Independent API clients are not required to publish their own source; commercial use of the plugin still requires permission.

## Build and release

```sh
cargo test --locked --all-features --manifest-path backend/Cargo.toml
cargo clippy --locked --all-targets --all-features --manifest-path backend/Cargo.toml -- -D warnings
```

Matching `vX.Y.Z` tags trigger Actions checks, ARM64/x86_64 builds and IPK/APK packaging. Verified assets, checksums, commit metadata and compatibility reports are uploaded only after required jobs pass. Existing assets are not overwritten. See [build instructions](docs/BUILD.md).

[Changelog](CHANGELOG.md) · [1.0.1 notes](docs/RELEASE-1.0.1.md) · [security](docs/SECURITY.md) · [performance](docs/PERFORMANCE.md) · [QModem references](docs/QMODEM.md) · [frontend notices](docs/FRONTEND-SOURCES.md)
