# WebUI与第三方来源

1.0.0 WebUI由SlAlot独立实现，采用本地JavaScript、CSS和Rust RPC，无React／Ant运行时或外部CDN。界面消费服务缓存，不直接采样硬件或构造AT写入。

主题、弹窗和响应式布局使用统一色板；图标包括Framework7 SVG轮廓、Lucide侧栏图标及作者自绘路径，不包含闭源小程序业务模块或商业授权。

完整版权及许可见[图标声明](ICON-SOURCES.md)和ui-notices.html。Rust依赖声明随包安装于/usr/share/wlmb-server/THIRD-PARTY-NOTICES，项目许可为同目录LICENSE。第三方原有权利不受项目非商业限制替代。

动画使用短时CSS／Web Animations，支持减少动态效果。离线DOM和尺寸约束测试不代表真实浏览器或微信真机视觉验收。
