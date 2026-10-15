# LAN traffic rate sources / 局域网速率来源

WiFi uses explicit iwinfo station RX/TX byte counters, not PHY bitrate.
Ethernet uses read-only bridge forwarding entries and physical port byte counters.
The parser is independently implemented against the [Linux bridge UAPI](https://github.com/torvalds/linux/blob/master/include/uapi/linux/if_bridge.h); no kernel implementation is copied.

Optional `status.system.lan.ports[]` rows contain `interface`, `source` (ethernet-port-counter-delta),
`scope` (port), `peerCount` (learned MAC count, not guaranteed online clients) and nullable `rates`.
`downloadBytesPerSecond` is port TX toward LAN; `uploadBytesPerSecond` is port RX.
These include LAN/broadcast/multicast, not just internet traffic or exact per-IP accounting.
Sampling reuses the approximately 12–18-second LAN cadence. First/reset/missing/stale samples are null,
not invented zero. No nft rules, sysctls, conntrack reads, capture or acceleration changes.

Exactly one learned peer on exactly one observed physical port may attach rates to an already
observed ARP device, explicitly as `rateScope: port`, `rateInterface`, `rateSource`.
Multiple learned peers (including retained FDB history), cross-port MAC ambiguity and owner changes
do not assign the whole port as individual-device traffic. WebUI also exposes a separate port card.

Only up/forwarding physical Ethernet ports qualify. Known wireless interfaces, phy80211/WEXT devices,
virtual/non-bridged interfaces and malformed/unavailable/changing FDBs fail closed. Limits: 16 ports,
64 KiB FDB input. Identity includes ifindex/owner; a changed owner receives a fresh baseline.
This is host-level support, independent of modem vendor and shared/standalone AT mode. Drivers without
accurate port counters still require hardware acceptance; offload is never disabled to invent data.

有线显示网口收发字节差值，包含局域网/广播，不冒充严格单设备或仅互联网计费。
学习到多个MAC时只显示端口总速率；首次/重置/模糊归属/未支持驱动显示未读取。
不改防火墙/加速/网络，不抢AT，不加常驻进程。共享和独立模式使用同一主机采集代码。
