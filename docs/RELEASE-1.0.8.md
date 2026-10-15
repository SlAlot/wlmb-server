# 网络面板 1.0.8

本次改进24.10系统接口与QModem共享通道兼容，新增独立的gh-proxy.com加速安装脚本。1.0.7为内部验证阶段，未发布正式Release；正式版使用1.0.8，避免已安装测试修订的设备遇到版本降级。

- QModem省略use_ubus默认配置时，识别服务已打开的唯一端口，发送前仍核验真实占用，不打开或抢占串口。
- 仅无线status接口兼容字节完全相同的重复字段；冲突字段及其他ubus对象仍严格拒绝。
- WiFi配置使用UCI、network.wireless和iwinfo，不依赖网页地址。保留开放网络、已有加密和驱动报告的制式；复杂多AP及MLO不冒充可编辑。
- 采集绑定模组实例，暂时争用只保留未过期快照，时间戳不刷新；无效回复不会当作新测量。
- 新增install-gh-proxy.sh，固定同版包摘要和APK公钥，保留官方install.sh和离线安装；两个脚本均进入SHA256SUMS。

加速安装：

```sh
wget -O /tmp/wlmb-install-gh-proxy.sh https://gh-proxy.com/https://github.com/SlAlot/wlmb-server/releases/latest/download/install-gh-proxy.sh && sh /tmp/wlmb-install-gh-proxy.sh
```

第三方代理不是项目自营服务或永久备份，初始脚本仍需可信来源和独立核对。代理失败不绕过校验，改用官方或离线入口。安装不改拨号、WiFi、QModem或防火墙配置，不重启设备；已有插件配置保留。

本次不扩大模组协议或硬件验收承诺。旧版共享服务缺少事务或已审查队列时，短信发送／删除继续受保护，不因识别到模组而强行开放。当前设备只读验证不能代替全型号、真实短信发送或长期负载验收。源码及制品仅包含OpenWrt插件，小程序与商业授权仍闭源。

[项目说明](https://github.com/SlAlot/wlmb-server/blob/v1.0.8/README.md) · [安装教程](https://github.com/SlAlot/wlmb-server/blob/v1.0.8/docs/INSTALL.zh-CN.md) · [兼容范围](https://github.com/SlAlot/wlmb-server/blob/v1.0.8/docs/MODEM-COMPATIBILITY.md)
