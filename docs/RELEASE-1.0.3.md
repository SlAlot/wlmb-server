# 网络面板 1.0.3

修复APK缺少签名安装流程、LuCI上传提示`UNTRUSTED signature`的问题。

- 两架构正式APK使用SlAlot专用ECDSA P-256密钥签名，私钥不进入源码、安装包或工作流附件。
- 密钥仅在限制`v*`标签的GitHub发布环境中提供。普通分支与PR没有密钥，全部检查及双架构构建通过后才签名。
- 正式发布前验签，并确认无可信公钥及篡改包均被拒绝。缺少私钥或公钥不匹配时阻止发布。
- Release增加公钥和安装教程，全部纳入摘要及提交绑定清单。
- Rust、API版本、WebUI资源和安装包统一为1.0.3，不改变拨号、WiFi或其他网络配置。

**首次仍需导入公钥**，签名不意味着所有固件会自动信任。导入后，LuCI可正常上传本次及后续同钥签名APK，无需跳过检查。旧的未签名包不适用。

[首次安装教程](APK-INSTALL.zh-CN.md)。IPK用户按原有opkg流程安装，不需要APK公钥。当前ABI为`aarch64_cortex-a53`与`x86_64`。安装默认启动服务及自启，不接管拨号。

兼容报告继续区分协议实现与硬件实测，不宣称所有模组与固件均已验收。仓库仅包含插件，小程序仍是通过API接入的独立闭源客户端。

[项目说明](https://github.com/SlAlot/wlmb-server/blob/v1.0.3/README.md) · [API](https://github.com/SlAlot/wlmb-server/blob/v1.0.3/docs/API.zh-CN.md) · [兼容范围](https://github.com/SlAlot/wlmb-server/blob/v1.0.3/docs/MODEM-COMPATIBILITY.md)
