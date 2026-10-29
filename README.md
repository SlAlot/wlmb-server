# 网络面板 · fnOS 分支

飞牛原生应用，由 **SlAlot** 维护。复用本项目 Rust 模组采集、协议解析和信号面板，提供独立的飞牛应用入口。

本分支为 `fnos`；[OpenWrt 版在 main 分支](https://github.com/SlAlot/wlmb-server/tree/main)，互不替代。当前原生包版本 **0.1.1**，基于核心 1.1.3。

## 当前功能

- 模组识别、实时信号、服务小区、频段、载波和邻区查看。
- RSRP、RSRQ、SINR、PCI、频点、带宽和温度按模组实际返回显示；未返回或过期数据不会伪造。
- 复用网络面板的深浅主题、手机布局和信号图表。
- 从飞牛桌面或应用中心打开，使用现有飞牛管理员账号验证。
- 在飞牛应用中心启动、停止、升级和卸载。

当前为参数查看版，未开放短信、锁频、锁小区、任意 AT 指令、拨号和主机 Wi-Fi 设置。Wi-Fi AP 使用设备自己的热点管理应用。应用不修改 LAN 地址、路由、DNS 或模组拨号配置。

## 安装与使用

1. 从本分支 Actions 下载对应架构的 `.fpk` 测试包，解压构建附件后在飞牛应用中心手动安装。
2. 打开桌面的 **网络面板**，输入现有飞牛管理员账号和密码。
3. 等待自动识别模组并读取参数。默认访问端口为 **1117**，与 OpenWrt 版使用相同端口号。

已在一台 Photonicat 1、fnOS 1.1.31、RM500U-CNV NCM 模式下读取到实时信号与温度。其他机型、模组及系统版本需要继续验证；通用协议目录不等于所有型号均完成实测。

串口由其他程序占用时停止读取，不抢占端口。普通 NAS 上若未发现唯一且可核验的 AT 端点，会显示尚未识别，而不会探测未知诊断或 GPS 通道。

## 开发与打包

```sh
cargo build --manifest-path backend/Cargo.toml --bin wlmb-fnos --features fnos --locked --release --target aarch64-unknown-linux-musl
python3 tools/prepare-fnos.py backend/target/aarch64-unknown-linux-musl/release/wlmb-fnos dist/fnos-native
fnpack build --directory dist/fnos-native
```

`fnos/` 是原生应用结构；`backend/src/fnos.rs` 提供飞牛适配；`tools/prepare-fnos.py` 汇集二进制、现有前端、图标和许可。构建工具需安装 Rust 对应 target 和链接器。

本分支推送后运行 **Build fnOS native app**，检查权限和前端后生成 ARM64 与 x86_64 测试包附件，不自动宣称商店审核通过。

## 飞牛开发规范

按[飞牛应用开放平台](https://developer.fnnas.com/docs/guide)准备 manifest、应用权限、资源、桌面入口和完整生命周期脚本。

- 应用 ID 为 `wlmb-fnos`，桌面入口仅管理员可见。
- 使用飞牛提供的应用目录、数据目录及用户环境变量。
- HTTP 网页进程以专用应用用户运行。需要串口占用核验及管理员验证的 Rust 后端独立运行，通过私有 Unix Socket 提供受限读取接口，不直接监听网络端口。
- 不请求用户文件授权，不添加系统软件源，不安装 OpenWrt ubus/UCI，不覆盖飞牛原生网页或网络配置。
- 升级与卸载先停止自身进程；不卸载设备驱动，不清除 NAS 网络配置。
- 已生成原生测试包；上架商店仍需完成审核材料、更多系统版本及硬件兼容性测试。

详见 [fnOS 适配与验证](docs/FNOS.md)。

## 许可

继续使用项目原有[非商业使用与来源声明许可](LICENSE)，保留 SlAlot 作者署名和第三方声明。本分支是网络面板的飞牛适配版，不是飞牛官方应用。
