# 网络面板 1.1.2

修复部分QModem固件能正常显示信号、网络面板却读不到模组信息的问题；同时改善已连接设备上网控制的不可用提示。

## 本次更新

- 按接口签名自动兼容，不固定QModem版本或模组型号。共享AT已就绪时保持原路径；未就绪时复用QModem原生信息接口，不修改QModem配置、不抢占串口。
- 读取接口实际提供的4G／5G信号、频段、小区、载波、签约速率和模组温度；缺失项不填假数据，CQI不冒充QCI。
- 共用缓存并限制查询频率；保留实例核验、过期标记和脱敏错误。原生接口本身可能返回缓存，不保证每次都是即时测量。
- 上网控制说明具体不可用原因。开启流量加速时仍保留保护限制，不自动关闭软硬件卸载，不改防火墙或拨号。
- 原生信息可读不代表短信收发、邻区扫描、锁频或锁小区均已开放，仍按独立能力判断。

## 安装或更新

官方入口：

```sh
wget -O /tmp/wlmb-install.sh https://github.com/SlAlot/wlmb-server/releases/latest/download/install.sh && sh /tmp/wlmb-install.sh
```

可选第三方加速入口：

```sh
wget -O /tmp/wlmb-install-gh-proxy.sh https://gh-proxy.com/https://github.com/SlAlot/wlmb-server/releases/latest/download/install-gh-proxy.sh && sh /tmp/wlmb-install-gh-proxy.sh
```

继续提供Cortex-A53、armsr/armv8和x86_64的APK与IPK。保留现有配置；APK使用原固定公钥验签，安装器校验版本固定摘要。第三方加速可能存在缓存延迟，以脚本及安装结果显示的1.1.2版本为准。

H5000M反馈已确认QModem原生查询能够返回信息，但本次新包仍需用户实机复测；自动化测试不等于全型号硬件认证。小程序连接1117端口失败需要单独检查，不承诺此次兼容更新能够解决网络权限或端口连通性问题。
