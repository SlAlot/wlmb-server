# 网络面板 1.0.0

作者：SlAlot。首个源码公开版本，仅发布OpenWrt插件；小程序、商业Web和授权服务不在仓库中。

Rust后端＋独立LuCI WebUI，端口1117，不接管拨号。包括自动共享／独立通道、信号／载波／邻区、WiFi和设备、流量账本与限额、锁频／锁小区、短信及PushPlus转发。支持深浅模式、手机布局及PC折叠导航。

## 下载

| 包ABI | opkg固件 | apk固件 |
| --- | --- | --- |
| aarch64_cortex-a53 | wlmb-server-1.0.0-aarch64.ipk | wlmb-server-1.0.0-aarch64.apk |
| x86_64 | wlmb-server-1.0.0-x86_64.ipk | wlmb-server-1.0.0-x86_64.apk |

以实际包管理器和ABI为准，其他架构不可强装。安装后进入LuCI“服务→网络面板”，默认开机启动。当前包未签名，请核对可信来源及SHA256SUMS；APK固件需显式允许未签名包。

## 许可与客户端

免费非商业使用须保留SlAlot署名、来源和许可，未经书面授权禁止商用。采用LicenseRef-WLMB-NonCommercial源码可用许可，不是MIT／GPL或OSI认可的开源许可；第三方许可不变。

微信“网络面板助手”是独立闭源商业客户端，通过公开API接入；插件WebUI不依赖付费授权。连接时选择openwrt并填写自己的管理地址和1117端口。

## 边界

目录91个身份／131份配置，不代表全固件认证。温度协议89个、锁频81个、锁小区63个、邻区64个、CA71个。短信受编码／存储／事件及共享服务能力约束，事件优先、默认10秒兜底。

短信送达、锁定射频生效、WiFi重载和长期负载仍需对应硬件验收。不以版本号替代能力门禁，不自动飞行模式、拨号或重启。

Actions检查及两架构打包通过才上传制品。release.json记录准确提交，manifest记录安装文件摘要，SHA256SUMS用于校对本次下载内容。

[说明](../README.md) · [兼容](RELEASE-AUDIT.md) · [API](API.zh-CN.md) · [构建](BUILD.md) · [安全](SECURITY.md)
