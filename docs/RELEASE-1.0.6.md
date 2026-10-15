# 网络面板 1.0.6

修复一键安装最后一步误报“服务未就绪”。安装器此前未携带登录会话调用受保护的LuCI接口，服务已运行也可能收到SESSION_EXPIRED；不是下载、APK验签或包写入失败。

- 使用Rust本机只读健康检查，通过根用户权限保护的控制套接字获取响应，并核对运行版本。
- 不索取管理密码，不放宽网页接口权限，不发送AT命令，不修改拨号、WiFi和防火墙，也不重启设备。
- 服务无响应、运行旧版或异常返回仍不能报成功；保留安装结果，提示检查服务和日志。
- 安装器回归增加受保护RPC拒绝、旧版进程及延迟就绪等场景；继续严格校验同版摘要、APK签名和固定发布公钥。

已安装1.0.5且服务正在运行的用户，不需要因为旧提示卸载或反复重装。使用旧版安装脚本不会自动获得本次修复，应重新下载最新install.sh。

[一键安装教程](https://github.com/SlAlot/wlmb-server/blob/v1.0.6/docs/INSTALL.zh-CN.md) · [项目说明](https://github.com/SlAlot/wlmb-server/blob/v1.0.6/README.md) · [兼容范围](https://github.com/SlAlot/wlmb-server/blob/v1.0.6/docs/MODEM-COMPATIBILITY.md)

本次不扩大模组或硬件实测范围，包ABI仍为aarch64_cortex-a53和x86_64。旧发布和公钥不变，小程序仍闭源，仅通过插件API接入。
