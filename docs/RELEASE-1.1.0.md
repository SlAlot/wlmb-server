# 网络面板 1.1.0

本版新增GL.iNet共享接口的自动兼容，沿用Rust后端、独立WebUI和1117 API，不接管拨号，不升级或修改QModem。

## 自动识别

- 自动模式优先沿用QModem；无QModem时发现`modem.*.AT`及可用方法签名，按真实服务识别CPU／USB总线，不固定Mudi7或串口编号。没有共享服务时走已有独立通道发现与占用保护。
- 查询真实模组身份后复用既有目录及协议解析器，未知能力保持关闭。已知GL.iNet共享服务暂时失败时不自动抢占原厂串口。
- 补充RG650V-EU紧凑NR载波格式，独立核对每个载波的频点、PCI及信号，不猜测未知尾部字段，不虚增聚合数量。

## WiFi与短信

- WiFi优先使用系统运行接口，缺失时通过iwinfo发现AP并核对配置关系；分开显示配置和实测状态，不把未确认状态当关闭，不把配置WiFi7冒充正在WiFi7运行。保留2.4／5／6GHz只读信息；无法核验厂商写入流程时不开放编辑。
- GL.iNet复用当前PDU设置读取短信，接入插件已读、重组、去重及PushPlus转发。开启接收或转发后默认10秒兜底检查，扫描完成后计时，忙碌时退避；两项关闭后不轮询短信、不创建推送TLS客户端。不会修改CNMI、编码或短信存储。
- 尚未核验GL.iNet事件通知及写入事务，因此GL.iNet短信发送／删除、锁频锁小区写入暂不开放。既有QModem及独立模式继续按各自协议和能力门禁运行；不把“自动识别”宣传成全功能、全固件保证。
- LuCI端口可以是80或8080；小程序填写插件的管理地址和1117端口，不通过LuCI网页抓取数据。

## 安装

提供`aarch64_cortex-a53`、`aarch64_generic`及`x86_64`三种包ABI，共六个IPK／APK。APK正式签名、公钥与SHA-256校验保持不变，安装器自动选择包管理器与ABI，保留插件配置，不改拨号／WiFi，不重启设备。

官方入口：

```sh
wget -O /tmp/wlmb-install.sh https://github.com/SlAlot/wlmb-server/releases/latest/download/install.sh && sh /tmp/wlmb-install.sh
```

可选第三方加速入口：

```sh
wget -O /tmp/wlmb-install-gh-proxy.sh https://gh-proxy.com/https://github.com/SlAlot/wlmb-server/releases/latest/download/install-gh-proxy.sh && sh /tmp/wlmb-install-gh-proxy.sh
```

加速站并非项目自营，首次脚本需来自可信下载，代理失败时使用官方或离线安装，不关闭校验。

## 验证范围

离线测试覆盖自动选择组合、CPU／USB服务发现、模组目录复用、原生ubus查询帧、错误／实例切换、空短信及合成PDU、关闭任务资源、无线歧义映射及前端状态。GitHub Actions检查、构建和签名结果见本次发布工作流；协议样本来自用户提供的Mudi7／RG650V-EU输出，并非安装后在线验收。真实短信送达、各固件的无线写入及所有GL.iNet硬件仍需分别实测。

公开仓库与安装包仅包含OpenWrt插件，不包含闭源小程序、授权服务或私钥。源码继续按非商业及来源声明许可提供。
