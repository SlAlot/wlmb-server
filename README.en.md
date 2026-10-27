# Network Panel · wlmb-server

[简体中文](README.md) · [Downloads](https://github.com/SlAlot/wlmb-server/releases/latest) · [API](docs/API.en.md)

**Network Panel 1.1.3** by **SlAlot** is a bounded Rust modem service with an independent LuCI JavaScript/CSS interface for OpenWrt. The first public release is 1.0.0. Default port: **1117**. It does not take over dialing. QModem's public identity facts, AT formats and shared interfaces inform the independently implemented adapters.

1.1.3 adds the Luoye Xiaotong modified modem adapter and improves passive neighbor/carrier queries over QModem shared AT, cache identity binding and failure backoff. Read access does not grant modem control permissions. Existing ABI fallback, GL.iNet compatibility, Cortex-A53, armsr/armv8 and x86_64 packages remain available. Official and accelerated installers select this release after publication.

Source is publicly available under a [custom non-commercial attribution license](LICENSE). **Commercial use requires separate permission**; retain author and source notices. This is source-available, not MIT/GPL or OSI-approved open source. Third-party materials retain their own licenses.

## One-command installation

Run as root in the router's SSH session or terminal, read the notice and confirm once:

```sh
wget -O /tmp/wlmb-install.sh https://github.com/SlAlot/wlmb-server/releases/latest/download/install.sh && sh /tmp/wlmb-install.sh
```

The complete script is downloaded before execution. It detects APK/opkg and the package ABI, checks embedded version-specific hashes and verifies APK signatures with the pinned key before importing it. Existing configuration is preserved; no dialing, WiFi or firewall configuration changes or router reboot are performed. Bootstrap requires trust in this official script download; do not run copies from unknown sources.

Optional third-party download route (gh-proxy.com is not operated by this project):

```sh
wget -O /tmp/wlmb-install-gh-proxy.sh https://gh-proxy.com/https://github.com/SlAlot/wlmb-server/releases/latest/download/install-gh-proxy.sh && sh /tmp/wlmb-install-gh-proxy.sh
```

The separate script pins its release's package hashes and APK signing key; it does not weaken verification or contact the proxy with router credentials. The initial script still requires a trusted download; independently compare it with the official Release's SHA256SUMS where possible. This is a proxy, not a permanent archive or availability guarantee. Use the official or offline route if unavailable.

Use `--check` to download and verify without installation or persistent key import. For offline installation, download the same Release's script, matching package and APK public key into one router directory, then run `sh /tmp/wlmb-offline/install.sh --offline /tmp/wlmb-offline`. Dependencies must already exist for offline use. `--yes` explicitly consents to installation and key trust for automation. Unknown ABIs, conflicting keys, downgrade attempts and failed checks stop the installer. See the [installation guide](docs/INSTALL.zh-CN.md).

APK ABI detection prioritizes the firmware's `/etc/apk/arch`. `apk --print-arch` can report only the default CPU family (for example `aarch64`) while the actual configured package ABI is `aarch64_cortex-a53`. No architecture override is used.

## Manual installation

Download from [Releases](https://github.com/SlAlot/wlmb-server/releases/latest) and verify `SHA256SUMS`. Use the actual firmware's package manager and ABI: 24.10 normally uses IPK/opkg; 25.12 normally uses APK/apk. Provided ABIs: `aarch64_cortex-a53`, `aarch64_generic` and `x86_64`. For armsr/armv8 choose the filename containing `aarch64_generic`, not the Cortex-A53 package. Other architectures/ABIs are not covered. APK here is not an Android application.

```sh
opkg install /tmp/wlmb-server-1.1.3-aarch64.ipk
# On APK firmware, only after verifying the trusted download and checksum:
apk add /tmp/wlmb-server-1.1.3-aarch64.apk
```

Official APK assets from 1.0.3 are signed. Before the first APK installation, download `wlmb-server-signing.pem` from the official Release, verify its file SHA-256 below against this README, then copy it to `/etc/apk/keys/wlmb-server-signing.pem` with mode 644. Only then use LuCI upload or the command above; new routers do not automatically trust our key. Do not replace firmware keys or disable signature validation. See the [step-by-step installation guide](docs/APK-INSTALL.zh-CN.md).

Public key file SHA-256: `402d9e009fb2064df25dfccec6855b037c039c22c61a95febc9f400d4b53d8ec`.

Use your firmware's own feed for LuCI, rpcd, rpcd-mod-iwinfo and uci dependencies. Open **LuCI → Services → 网络面板**. Installation enables the plugin and boot startup. Do not bypass architecture checks or expose port 1117 to the Internet. IPK users do not need an APK key. No package feed is provided yet.

## Features and limits

Automatic shared/standalone selection; modem signal, serving cells, carriers, neighbors, operator, subscription and temperature; WiFi and device controls; traffic accounting and limits; typed band/cell locks; SMS/read-state and optional PushPlus forwarding. SMS events are handled first, with a default 10-second fallback. Disabled optional tasks release their resources.

The catalog maps 91 identities and 131 bus profiles. Implemented paths cover temperature for 89 identities, band locks for 81, cell locks for 63, neighbors for 64 and carrier aggregation for 71. **These are code coverage counts, not blanket hardware/firmware certification.** Unknown data stays unknown, uncertain writes are not replayed, and RF resets are never automatic. See [compatibility](docs/MODEM-COMPATIBILITY.md) and [audit](docs/RELEASE-AUDIT.md).

Shared mode reuses existing QModem interfaces; it does not take over, patch, upgrade or reconfigure QModem. Older/custom firmware mixing direct and shared AT access can still cause contention or delays. The plugin preserves bounded cache validity and rejects recognized crossed replies, but does not add special adaptations or promise to eliminate other clients' blocking.

The Luoye Xiaotong modified modem adapter uses published local snapshots and allowlisted passive neighbor transactions through the original tool, without a fixed management address or direct serial access. Modem writes and SMS remain disabled for this provider pending hardware validation; see [scope and limitations](docs/LUOYE-MODEM.md). When this manager is absent, auto mode prefers QModem, then GL.iNet shared CPU/USB AT services, otherwise protected independent discovery. GL.iNet supports verified PDU reading and forwarding; unverified events, sending/deletion and modem locking writes remain disabled. Missing WiFi runtime interfaces produce read-only/unknown state, not a false disabled status. LuCI on port 8080 does not change the plugin API on 1117. This is capability-based compatibility, not certification of every GL.iNet model; see [GL.iNet](docs/GLINET.md).

## WeChat and other clients

Open “网络面板助手” in WeChat, join the router WiFi, select **openwrt**, and enter your management address with port 1117. The mini-program is a separate closed-source commercial client. Its paid features do not restrict the plugin WebUI.

This repository contains **only the OpenWrt plugin**, excluding the mini-program, commercial Web client, licensing service, authorization WASM and signing secrets. Independent API clients are not required to publish their own source; commercial use of the plugin still requires permission.

## Build and release

Firmware developers: add `src-git wlmb https://github.com/SlAlot/wlmb-server.git;main` to the active feed configuration, update/install feeds, then select `wlmb-server` under **LuCI → Applications** as built-in. The standard recipe builds Rust from source for ARM64/x86_64 using OpenWrt's Rust tooling. See the [firmware integration guide (Chinese)](docs/FIRMWARE-INTEGRATION.zh-CN.md) for prerequisites and commands.

```sh
cargo test --locked --all-features --manifest-path backend/Cargo.toml
cargo clippy --locked --all-targets --all-features --manifest-path backend/Cargo.toml -- -D warnings
```

Matching `vX.Y.Z` tags trigger Actions checks, ARM64/x86_64 builds and IPK/APK packaging. Verified assets, checksums, commit metadata and compatibility reports are uploaded only after required jobs pass. Existing assets are not overwritten. See [build instructions](docs/BUILD.md).

[Changelog](CHANGELOG.md) · [1.1.3 notes](docs/RELEASE-1.1.3.md) · [security](docs/SECURITY.md) · [performance](docs/PERFORMANCE.md) · [QModem references](docs/QMODEM.md) · [frontend notices](docs/FRONTEND-SOURCES.md)
