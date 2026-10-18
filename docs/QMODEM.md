# QModem 参考来源与独立实现

网络面板根据qmodem为参考开发接入，Rust代码由本项目独立实现；不包含QModem商业受限实现。

## 参考范围

- 型号／厂商／平台／USB与PCIe配置事实及支持频段资料。
- 各平台公开AT命令名称、参数格式、回复字段与能力差异。
- at-daemon的公开调用格式，以及经审查的队列／事务契约。
- rpcd `qmodem` 对象的 `cell_info`、`network_info`、`base_info` 信息查询格式。
- qmodem.sms第三版的数据库、managed发送、删除和URC事件接口。

锁定、解析、缓存、身份复核、权限、去重、持久账本、HTTP/RPC和WebUI为自有实现。不复制示例伪数据、硬编码邻区或第三方商业授权代码。

## 固定参考与更新

上游：https://github.com/FUjr/QModem 。已审查基线提交 `c49654efc870f53712ee8e25bf181722eb1466d9`。

`qmodem-upstream.json`保存型号／平台事实和源文件哈希；`model-bands.json`保存频段事实，不等于写协议已支持。通过 `tools/qmodem-audit.cjs` 和 `.github/workflows/qmodem-audit.yml` 检测上游变化，变化先进入报告，由维护者审查后再更新基线；不能自动复制或执行新命令。

短信v3接口另于2026-10-14核对公开main的对象、方法、模式和字段，版本及签名运行时必须匹配；并非假定所有已安装QModem均为v3。

## 共享与独立

检测到QModem时核验共享服务，不接管它的拨号或串口；未知ABI和占用拒绝不安全操作。没有QModem时使用自有通道，仍必须核验接口空闲。两者共用协议解析和能力门禁。

不能用 `at-daemon list` 为空判断 QModem 无法采集。部分固件通过自己的 rpcd 查询信息，串口未常驻在 at-daemon 中。插件检查公开方法签名：共享 AT 已就绪时保持原路径；未就绪且支持 `cell_info(config_section)` 时，按配置节自动使用 QModem 原生信息接口，不根据版本号或某个机型硬编码切换。

原生路径归一化 4G／5G SA／NSA 的信号、频段、小区与明确关联的载波信息；可选读取签约速率与模组温度。CQI 不冒充 QCI，缺失字段不补造，WiFi和网口速率仍来自系统接口。后台至少间隔十秒完成一次原生采集，温度查询缓存一分钟，所有客户端共用快照。原生回复可能来自 QModem 缓存，标记 `providerCached` 和 `sourceCacheAgeUnknown`；`observedAtMs` 是插件本次查询起始时间，不承诺模组的原始测量时间。该采集间隔与短信事件／兜底检查无关。

原生信息可读不等于锁频、邻区扫描、短信收发或删除可用。这些操作继续走已有独立能力检查，不通过任意 `send_at` 绕过安全门禁。模组实例或采集通道变化后丢弃旧结果；可选信息失败不清空已有主信号。

QModem未提供可用短信服务时可使用安全共享AT；已选数据库后端遇到错误不自动切换来源。插件不会升级QModem、改其短信模式或认领其转发器队列。

[模组兼容](MODEM-COMPATIBILITY.md) · [短信](SMS.md)
