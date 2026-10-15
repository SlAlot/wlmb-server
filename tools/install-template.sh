#!/bin/sh
# 网络面板一次性安装器；不常驻，不修改拨号、无线或防火墙配置。
set -eu
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
umask 077
VERSION='@VERSION@'
KEY_SHA='@KEY_SHA256@'
BASE="https://github.com/SlAlot/wlmb-server/releases/download/v$VERSION"
OFFLINE=''
CHECK=0
YES=0
TEMP=''
LOCKED=0
NEW_KEY=0
INSTALL_STARTED=0
TRUST='/etc/apk/keys/wlmb-server-signing.pem'
LOCK='/tmp/wlmb-installer.lock'
fail() { printf '\n安装未完成：%s\n' "$*" >&2; exit 1; }
help() {
  printf '网络面板 %s 安装器\n用法：sh install.sh [--check] [--offline 文件目录] [--yes]\n' "$VERSION"
  printf '%s\n' '--check 仅下载校验，不安装、不导入系统公钥。' '--offline 使用同版安装包和公钥，不联网补依赖。' '--yes 明确同意安装及首次信任发布公钥，供自动化使用。'
}
while [ "$#" -gt 0 ]; do
  case "$1" in
    --help|-h) help; exit 0 ;;
    --check) CHECK=1; shift ;;
    --yes) YES=1; shift ;;
    --offline) [ "$#" -ge 2 ] || fail '--offline 缺少目录'; OFFLINE=$2; shift 2 ;;
    *) fail "未知参数：$1" ;;
  esac
done
if [ "$CHECK" = 0 ]; then [ "$(id -u)" = 0 ] || fail '请使用路由器root账户运行'; fi
for tool in sha256sum awk sed grep df mktemp ubus uci; do
  command -v "$tool" >/dev/null 2>&1 || fail "缺少OpenWrt工具：$tool"
done
[ "$(uname -s)" = Linux ] || fail '仅适用于OpenWrt／ImmortalWrt'
MANAGER=''
ABI=''
if command -v apk >/dev/null 2>&1; then
  ABI=$(apk --print-arch 2>/dev/null || true)
  [ -z "$ABI" ] || MANAGER=apk
fi
if [ -z "$MANAGER" ] && command -v opkg >/dev/null 2>&1; then
  ABI=$(opkg print-architecture | awk '$1=="arch" && ($2=="aarch64_cortex-a53" || $2=="x86_64") {print $2}')
  MANAGER=opkg
fi
case "$ABI" in
  aarch64_cortex-a53) ARCH=aarch64 ;;
  x86_64) ARCH=x86_64 ;;
  *) fail "尚未提供此设备ABI的安装包：${ABI:-未识别}；不强制安装" ;;
esac
case "$MANAGER:$ARCH" in
  apk:aarch64) EXT=apk; PKG_SHA='@APK_AARCH64_SHA256@'; PKG_BYTES=@APK_AARCH64_BYTES@ ;;
  apk:x86_64) EXT=apk; PKG_SHA='@APK_X86_64_SHA256@'; PKG_BYTES=@APK_X86_64_BYTES@ ;;
  opkg:aarch64) EXT=ipk; PKG_SHA='@IPK_AARCH64_SHA256@'; PKG_BYTES=@IPK_AARCH64_BYTES@ ;;
  opkg:x86_64) EXT=ipk; PKG_SHA='@IPK_X86_64_SHA256@'; PKG_BYTES=@IPK_X86_64_BYTES@ ;;
  *) fail '没有识别到可用的包管理器' ;;
esac
PACKAGE="wlmb-server-$VERSION-$ARCH.$EXT"
verify() { printf '%s  %s\n' "$2" "$1" | sha256sum -c - >/dev/null 2>&1; }
space() {
  available=$(df -Pk "$1" | awk 'END {print $4}')
  case "$available" in ''|*[!0-9]*) fail "无法检查可用空间：$1" ;; esac
  [ "$available" -ge "$2" ] || fail "$1 可用空间不足，需要至少 $2 KiB"
}
CURRENT=''
if [ -x /usr/sbin/wlmb-server ]; then
  CURRENT=$(/usr/sbin/wlmb-server --version 2>/dev/null | sed -n 's/^WLMB_BUILD=\([^;]*\);.*/\1/p')
  if [ -n "$CURRENT" ] && awk -v a="$CURRENT" -v b="$VERSION" 'BEGIN {split(a,x,"."); split(b,y,"."); for(i=1;i<=3;i++){if(x[i]+0>y[i]+0)exit 0; if(x[i]+0<y[i]+0)exit 1}exit 1}'; then
    fail "当前版本 $CURRENT 高于安装器版本 $VERSION，请使用新版安装器"
  fi
fi
printf '网络面板 %s · %s · %s\n' "$VERSION" "$MANAGER" "$ABI"
[ -z "$CURRENT" ] || printf '当前版本：%s；保留已有插件配置。\n' "$CURRENT"
if [ "$CHECK" = 0 ] && [ "$YES" = 0 ]; then
  printf '%s\n' '将安装网络面板；APK首次自动核对并信任SlAlot发布公钥。' '只使用固件现有依赖源；不改拨号、WiFi，不重启设备。'
  printf '是否继续？[y/N] '
  answer=''
  IFS= read -r answer || fail '未收到确认；自动化调用需明确使用 --yes'
  case "$answer" in y|Y|yes|YES) ;; *) printf '已取消，未安装、未导入公钥。\n'; exit 0 ;; esac
fi
space /tmp "$((PKG_BYTES / 1024 + 16384))"
if [ "$CHECK" = 0 ]; then space / 8192; fi
mkdir "$LOCK" 2>/dev/null || fail '另一个安装器可能正在运行；确认无安装任务后再重试'
LOCKED=1
cleanup() {
  result=$?
  trap - 0
  if [ "$NEW_KEY" = 1 ] && [ "$INSTALL_STARTED" = 0 ] && [ -f "$TRUST" ] && [ ! -L "$TRUST" ]; then
    if verify "$TRUST" "$KEY_SHA"; then rm -f "$TRUST"; fi
  fi
  case "$TEMP" in /tmp/wlmb-installer.*) [ ! -d "$TEMP" ] || rm -rf "$TEMP" ;; esac
  if [ "$LOCKED" = 1 ]; then rmdir "$LOCK" 2>/dev/null || true; fi
  exit "$result"
}
trap cleanup 0
trap 'exit 130' INT
trap 'exit 143' TERM
TEMP=$(mktemp -d /tmp/wlmb-installer.XXXXXX) || fail '无法创建临时目录'
fetch() {
  name=$1
  target=$2
  if [ -n "$OFFLINE" ]; then
    [ -f "$OFFLINE/$name" ] && [ ! -L "$OFFLINE/$name" ] || fail "离线目录缺少普通文件：$name"
    cp "$OFFLINE/$name" "$target" || fail "无法读取离线文件：$name"
  elif command -v curl >/dev/null 2>&1; then
    curl --fail --location --proto '=https' --proto-redir '=https' --connect-timeout 10 --max-time 120 --retry 2 --output "$target" "$BASE/$name" || fail '下载失败；请检查网络，或使用离线安装方式'
  elif command -v wget >/dev/null 2>&1; then
    wget -T 60 -O "$target" "$BASE/$name" || fail '下载失败；请检查HTTPS证书和网络，或使用离线安装方式'
  else
    fail '没有curl或wget；请使用 --offline 文件目录'
  fi
  [ -s "$target" ] || fail "下载文件为空：$name"
}
printf '正在准备 %s…\n' "$PACKAGE"
fetch "$PACKAGE" "$TEMP/$PACKAGE"
verify "$TEMP/$PACKAGE" "$PKG_SHA" || fail '安装包摘要不匹配，停止安装；请重新从正式发布页下载'
if [ "$MANAGER" = apk ]; then
  fetch wlmb-server-signing.pem "$TEMP/wlmb-server-signing.pem"
  verify "$TEMP/wlmb-server-signing.pem" "$KEY_SHA" || fail '公钥指纹不匹配，停止安装'
  mkdir "$TEMP/keys"
  cp "$TEMP/wlmb-server-signing.pem" "$TEMP/keys/wlmb-server-signing.pem"
  apk --keys-dir "$TEMP/keys" verify "$TEMP/$PACKAGE" || fail 'APK签名验证失败；不跳过校验'
  [ ! -L /etc/apk/keys ] && [ ! -L "$TRUST" ] || fail '系统公钥路径是符号链接，请手动检查'
  if [ -e "$TRUST" ]; then verify "$TRUST" "$KEY_SHA" || fail '已存在不同的同名公钥，拒绝自动替换'; fi
fi
if [ "$CHECK" = 1 ]; then
  printf '检查通过：版本、ABI、摘要%s；未安装、未导入系统公钥。\n' "$(if [ "$MANAGER" = apk ]; then printf '及APK签名'; fi)"
  exit 0
fi
if [ "$MANAGER" = apk ] && [ ! -e "$TRUST" ]; then
  mkdir -p /etc/apk/keys
  cp "$TEMP/wlmb-server-signing.pem" "$TRUST" || fail '公钥导入失败'
  NEW_KEY=1
  chmod 644 "$TRUST"
fi
apk_action() {
  if [ -n "$OFFLINE" ]; then apk --no-network "$@"; else apk "$@"; fi
}
preflight() {
  if [ "$MANAGER" = apk ]; then apk_action add --simulate "$TEMP/$PACKAGE"; else opkg --noaction install "$TEMP/$PACKAGE"; fi
}
if [ "$MANAGER" = opkg ] && [ -n "$OFFLINE" ]; then
  for dependency in rpcd rpcd-mod-iwinfo luci-base uci; do
    opkg status "$dependency" | grep -Eq '^Status:.* installed$' || fail "离线安装缺少依赖 $dependency，请先通过固件自身源补齐"
  done
fi
printf '正在检查固件依赖…\n'
if ! preflight; then
  [ -z "$OFFLINE" ] || fail '离线依赖检查失败；未开始安装，不联网补依赖'
  printf '尝试刷新固件现有软件源索引，不执行全系统升级…\n'
  "$MANAGER" update || fail '固件软件源刷新失败；不替换软件源，请检查源和网络'
  preflight || fail '依赖检查仍失败，未开始安装；请查看上方包管理器提示'
fi
INSTALL_STARTED=1
printf '正在安装网络面板…\n'
if [ "$MANAGER" = apk ]; then
  apk_action add "$TEMP/$PACKAGE" || fail '包管理器安装失败；可能已写入部分文件，请查看上方提示，不自动强制回滚'
else
  opkg install "$TEMP/$PACKAGE" || fail '包管理器安装失败；可能已写入部分文件，请查看上方提示，不自动强制回滚'
fi
marker=$(/usr/sbin/wlmb-server --version 2>/dev/null || true)
case "$marker" in "WLMB_BUILD=$VERSION;"*) ;; *) fail '安装后版本未匹配；请检查包管理器输出' ;; esac
ready=0
for attempt in 1 2 3 4 5 6 7 8 9 10; do
  if ubus call network-panel service_status 2>/dev/null | grep -Eq '"running"[[:space:]]*:[[:space:]]*true'; then ready=1; break; fi
  sleep 1
done
[ "$ready" = 1 ] || fail '安装包已写入，但服务未就绪；请在LuCI启动项检查，或运行 logread -e wlmb-server'
printf '\n安装完成：网络面板 %s\n后台入口：LuCI → 服务 → 网络面板\n' "$VERSION"
address=$(uci -q get network.lan.ipaddr 2>/dev/null || true)
address=${address%%/*}
case "$address" in ''|*[!0-9.]*) address='你的路由器管理地址' ;; esac
printf '小程序选择 openwrt，连接地址：%s:1117\n' "$address"
printf '未修改拨号、WiFi和防火墙配置，未重启设备。\n'
