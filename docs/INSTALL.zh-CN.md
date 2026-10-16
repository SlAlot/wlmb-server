# 一键安装与离线安装

网络面板1.0.4起提供一次性安装器。后端仍为Rust；安装脚本只是包管理器引导，不常驻、不轮询模组。

## 推荐：复制一条命令

通过SSH或路由器自带终端，以root登录，复制执行：

```sh
wget -O /tmp/wlmb-install.sh https://github.com/SlAlot/wlmb-server/releases/latest/download/install.sh && sh /tmp/wlmb-install.sh
```

看清版本和信任提示，输入`y`确认一次即可。下载完整脚本后才执行，不使用边下载边执行的管道。请只从上述SlAlot官方仓库获取脚本；首次运行脚本本身依赖你对发布来源和HTTPS的信任。

安装器会识别apk／opkg与包管理器ABI，选择对应包，按脚本内固定摘要校验；APK再核对固定公钥指纹、验证签名，确认后导入公钥并安装。只添加项目自己的公钥，不替换已有固件公钥，不关闭签名检查。同名公钥不同时停止，不自动换钥。

安装完成后显示**LuCI → 服务 → 网络面板**，以及小程序连接地址。安装默认启动服务及自启；已有插件配置保留，不修改拨号、WiFi、防火墙配置，不重启设备。后续同钥签名APK可直接通过LuCI上传升级。

当前支持ABI `aarch64_cortex-a53`、`aarch64_generic`、`x86_64`。armsr/armv8使用aarch64_generic，手动下载时请选择文件名含aarch64_generic的IPK或APK；现有aarch64后缀仍对应Cortex-A53包。不是所有ARM64固件都使用同一ABI；只尝试固件明确支持且与内核CPU家族一致的候选，不强制架构、不猜测未知ABI。

APK优先读取/etc/apk/arch中的固件配置；apk --print-arch可能只返回aarch64等默认CPU架构，不能据此误判或强装。1.1.1起同族多ABI不再误报不支持：OPKG按正优先级从高到低，同分时优先具体架构；APK保留配置中的兼容候选顺序。跨CPU家族、无已提供安装包或异常优先级仍停止。

## 可选：gh-proxy.com加速安装

第三方代理只负责下载公开发行文件，不是项目自营服务或永久备份。按[站点文档](https://gh-proxy.com/docs/github-accelerator)的前缀方式访问，不使用仓库主页的#片段地址：

```sh
wget -O /tmp/wlmb-install-gh-proxy.sh https://gh-proxy.com/https://github.com/SlAlot/wlmb-server/releases/latest/download/install-gh-proxy.sh && sh /tmp/wlmb-install-gh-proxy.sh
```

每次Release额外附带install-gh-proxy.sh，与官方install.sh同时生成并纳入SHA256SUMS，站点可列出这个公开资产；本项目不控制站点何时采集、缓存或刷新。脚本只从固定代理入口下载自身版本的包及公钥，保持相同的ABI、摘要、公钥与APK验签、依赖和就绪检查。支持相同的--check、--offline和--yes参数，不接受任意镜像地址，不自动切换到未知镜像。

首次脚本下载依赖对渠道的信任，脚本和校验清单都从同一第三方下载不能构成独立认证。建议通过官方Release独立核对SHA256SUMS或直接取得脚本；APK随后还须通过固定公钥验签。IPK只有摘要核对，不宣称具有APK的签名保证。代理不可用时用官方或离线入口，不关闭TLS、摘要或签名检查。安装器不向代理发送管理密码、令牌或私人配置。

## 只检查，不安装

下载脚本后可以先检查：

```sh
sh /tmp/wlmb-install.sh --check
```

该模式识别兼容候选、下载首个可获取的同版包并校验，APK使用临时公钥目录验签；不导入系统公钥，不进行依赖模拟、安装或软件源刷新。检查成功只证明这些校验通过，不保证依赖可安装，也不代表全模组硬件验收。

## 路由器不能访问GitHub：离线方式

1. 电脑从同一[Release](https://github.com/SlAlot/wlmb-server/releases/latest)下载`install.sh`、匹配架构的APK／IPK；APK另下载`wlmb-server-signing.pem`。
2. 将这些文件上传到路由器的同一目录，例如`/tmp/wlmb-offline`。
3. 执行下方命令，确认一次即可。离线目录可以有空格；不要改文件名，不混用不同版本。

```sh
sh /tmp/wlmb-offline/install.sh --offline /tmp/wlmb-offline
```

离线模式不联网补依赖。缺少rpcd、rpcd-mod-iwinfo、luci-base或uci时，先通过固件自身软件源安装依赖，再重试。不替换固件软件源。

也可以先执行`sh /tmp/wlmb-offline/install.sh --offline /tmp/wlmb-offline --check`。

## 自动化与失败处理

`--yes`表示明确同意安装和首次信任项目公钥，自动化调用可使用；普通用户按默认提示确认即可。没有确认或输入其他内容时不安装。

安装前检查临时空间和根文件系统空间，拒绝降级到旧安装器版本。对候选包逐个校验并调用opkg --noaction或apk --simulate；包不可获取或模拟失败时尝试下一个同族兼容候选，首个通过后只正式安装一次。在线全部候选预检失败时仅刷新一次固件现有源的索引后复查，不执行系统升级，也不替换源；离线模式不刷新。

摘要、签名、公钥、跨架构或空间异常立即停止，不能靠换候选绕过安全检查。所有候选均失败后汇总报错，预检失败会移除本次刚添加的项目公钥；已存在且匹配的公钥不删除。包管理器正式安装后若失败或同版服务未就绪，可能已写入文件，不再换候选重装，保留输出和信任公钥用于排查，不自动强制回滚或删除用户数据。

1.0.6起安装后使用`/usr/sbin/wlmb-server --check-ready`检查根用户专用本地套接字的真实响应和运行版本；不无会话调用LuCI接口，也不要求提供管理密码。命令退出码为0表示同版服务已响应，非0表示未就绪，不代表包未安装。可运行`/etc/init.d/wlmb-server status`及`logread -e wlmb-server`排查，不必反复安装。1.0.5出现的旧“服务未就绪”提示可能只是无会话检查误报；请重新下载最新安装脚本。

安装器只安装对应版本，不后台自动更新；每份脚本固定到自身版本和安装包摘要，`latest`只是获取最新版脚本的入口。脚本、公钥与安装包应从同一正式发布获取。卸载插件不会删除已导入的系统公钥。

如需完全手动操作，见[APK公钥导入与上传安装](APK-INSTALL.zh-CN.md)。IPK可使用固件的opkg直接安装本地包；其来源依赖可信下载与摘要，不能将APK签名保证套用到IPK。
