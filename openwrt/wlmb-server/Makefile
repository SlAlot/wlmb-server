# 使用 OpenWrt 标准包规则与官方 Rust 构建辅助，从当前 Feed 源码编译。
include $(TOPDIR)/rules.mk

WLMB_FEED_DIR:=$(abspath $(realpath $(CURDIR))/../..)

PKG_NAME:=wlmb-server
PKG_VERSION:=1.1.3
PKG_RELEASE:=1
PKG_MAINTAINER:=SlAlot
PKG_LICENSE:=LicenseRef-WLMB-NonCommercial
PKG_LICENSE_FILES:=LICENSE
PKG_BUILD_DEPENDS:=rust/host
PKG_BUILD_PARALLEL:=1
PKG_BUILD_DIR:=$(BUILD_DIR)/$(PKG_NAME)-$(PKG_VERSION)
PKG_FILE_DEPENDS:=$(WLMB_FEED_DIR)/backend/Cargo.toml \
	$(WLMB_FEED_DIR)/backend/Cargo.lock $(WLMB_FEED_DIR)/backend/src \
	$(WLMB_FEED_DIR)/docs/model-bands.json $(WLMB_FEED_DIR)/frontend \
	$(WLMB_FEED_DIR)/packaging/files $(WLMB_FEED_DIR)/LICENSE

include $(INCLUDE_DIR)/package.mk

RUST_INCLUDE_DIR:=$(TOPDIR)/feeds/packages/lang/rust
RUST_PKG_FEATURES:=pushplus-https
RUST_PKG_LOCKED:=1
# 允许先索引当前 Feed；缺少 Rust 辅助时在编译阶段明确报错。
-include $(RUST_INCLUDE_DIR)/rust-package.mk

define Package/wlmb-server
  SECTION:=luci
  CATEGORY:=LuCI
  SUBMENU:=Applications
  TITLE:=Network Panel - modem information and WebUI
  URL:=https://github.com/SlAlot/wlmb-server
  DEPENDS:=@(aarch64||x86_64) +libgcc +ubus +rpcd +rpcd-mod-iwinfo +luci-base +uci
endef

define Package/wlmb-server/description
  Network Panel Rust backend and LuCI WebUI, built from the current feed.
  Shared modem interfaces and protected standalone discovery; no dialer takeover.
endef

define Package/wlmb-server/conffiles
/etc/config/network_panel
endef

define Build/Prepare
	$(INSTALL_DIR) $(PKG_BUILD_DIR)/backend/src $(PKG_BUILD_DIR)/docs $(PKG_BUILD_DIR)/frontend $(PKG_BUILD_DIR)/packaging
	$(CP) $(WLMB_FEED_DIR)/backend/Cargo.toml $(WLMB_FEED_DIR)/backend/Cargo.lock $(PKG_BUILD_DIR)/backend/
	$(CP) $(WLMB_FEED_DIR)/backend/src/. $(PKG_BUILD_DIR)/backend/src/
	$(CP) $(WLMB_FEED_DIR)/docs/model-bands.json $(PKG_BUILD_DIR)/docs/
	$(CP) $(WLMB_FEED_DIR)/frontend/. $(PKG_BUILD_DIR)/frontend/
	$(CP) $(WLMB_FEED_DIR)/packaging/. $(PKG_BUILD_DIR)/packaging/
	$(CP) $(WLMB_FEED_DIR)/LICENSE $(PKG_BUILD_DIR)/
endef

define Build/Configure
endef

define Build/Compile
	test -f "$(RUST_INCLUDE_DIR)/rust-package.mk" || { echo '请更新 packages Feed 并安装 rust 构建依赖'; exit 1; }
	$(call Build/Compile/Cargo,backend)
endef

define Package/wlmb-server/install
	$(INSTALL_DIR) $(1)/usr/sbin $(1)/usr/libexec/rpcd $(1)/usr/share/wlmb-server
	$(INSTALL_BIN) $(PKG_INSTALL_DIR)/bin/wlmb-server $(1)/usr/sbin/wlmb-server
	$(LN) /usr/sbin/wlmb-server $(1)/usr/libexec/rpcd/network-panel
	$(CP) $(PKG_BUILD_DIR)/frontend/. $(1)/
	$(CP) $(PKG_BUILD_DIR)/packaging/files/. $(1)/
	$(INSTALL_BIN) $(PKG_BUILD_DIR)/packaging/files/etc/init.d/wlmb-server $(1)/etc/init.d/wlmb-server
	$(INSTALL_CONF) $(PKG_BUILD_DIR)/packaging/files/etc/config/network_panel $(1)/etc/config/network_panel
	$(INSTALL_DATA) $(PKG_BUILD_DIR)/LICENSE $(1)/usr/share/wlmb-server/LICENSE
endef

define Package/wlmb-server/postinst
#!/bin/sh
[ -z "$${IPKG_INSTROOT}" ] || exit 0
/etc/init.d/wlmb-server stop >/dev/null 2>&1 || true
if [ "$$(uci -q get network_panel.main.boot_enabled)" = "0" ]; then
	/etc/init.d/wlmb-server disable
else
	/etc/init.d/wlmb-server enable
fi
/etc/init.d/rpcd reload >/dev/null 2>&1 || /etc/init.d/rpcd restart >/dev/null 2>&1
/etc/init.d/wlmb-server start
endef

$(eval $(call BuildPackage,wlmb-server))
