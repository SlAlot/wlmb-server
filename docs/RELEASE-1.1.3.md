# 网络面板 1.1.3

## 本次更新

- 适配落叶小通魔改的 modem。
- 改善QModem共享模式的邻区与载波读取：已适配协议通过原send_at接口传输，网络面板自行解析；实际方法签名、配置和型号匹配后才启用，不按QModem版本硬编码。
- 加强配置与驻留身份核验。忙碌、掉线、超时或错帧不继续叠加查询；缓存保留原时间，不能把旧数据刷新为新测量。
- 统一读取中、失败、缓存过期与自动重试暂停提示；前后端静态资源更新，兼容原小程序登录与状态API。

## 接入范围

落叶小通魔改的modem参数读取原组件已发布缓存，邻区只借用原事务工具执行固定被动查询，不直接打开串口、不写死管理地址。该来源没有提供锁频、锁小区和短信操作契约，因此这些能力保持关闭；WiFi及设备列表仍按系统能力判断。缓存停止发布时如实过期，不绕过原管理器补采集。详见[适配说明](LUOYE-MODEM.md)。

邻区是否实际返回4G／5G测量仍受模组固件限制，不混入主动搜网，不把CA或服务小区伪装成邻区。工具超时不等于原任务取消，未知结果暂停自动重试，不盲目重发。

## 安装或更新

官方入口：

```sh
wget -O /tmp/wlmb-install.sh https://github.com/SlAlot/wlmb-server/releases/latest/download/install.sh && sh /tmp/wlmb-install.sh
```

可选第三方加速入口：

```sh
wget -O /tmp/wlmb-install-gh-proxy.sh https://gh-proxy.com/https://github.com/SlAlot/wlmb-server/releases/latest/download/install-gh-proxy.sh && sh /tmp/wlmb-install-gh-proxy.sh
```

提供aarch64_cortex-a53、aarch64_generic和x86_64三种ABI的APK／IPK，保留原发布公钥和安装校验，不覆盖旧版制品。第三方加速可能有缓存延迟，以脚本及安装结果的1.1.3版本为准。

发布须通过Linux全功能测试、双架构HTTPS构建和制品校验；自动化检查不是全模组硬件验收。落叶小通魔改的modem适配仍需原固件实测，旧QModem与其他客户端的串口争用也不会被本插件内部串行彻底消除。源码仅包含OpenWrt插件，商业小程序及授权实现不公开。
