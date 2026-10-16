#!/bin/sh
# 网络面板一次性安装器；不常驻，不修改拨号、无线或防火墙配置。
set -eu
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
umask 077
VERSION='@VERSION@'
KEY_SHA='@KEY_SHA256@'
DOWNLOAD_PREFIX='@DOWNLOAD_PREFIX@'
BASE="${DOWNLOAD_PREFIX}https://github.com/SlAlot/wlmb-server/releases/download/v$VERSION"
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
  printf '网络面板 %s 安装器\n用法：sh @SCRIPT_NAME@ [--check] [--offline 文件目录] [--yes]\n' "$VERSION"
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
if [ -n "$OFFLINE" ]; then
  [ -d "$OFFLINE" ] || fail '离线目录不存在'
  OFFLINE=$(cd "$OFFLINE" && pwd -P) || fail '无法读取离线目录'
fi
if [ "$CHECK" = 0 ]; then [ "$(id -u)" = 0 ] || fail '请使用路由器root账户运行'; fi
for tool in sha256sum awk sed grep df mktemp ubus uci; do
  command -v "$tool" >/dev/null 2>&1 || fail "缺少OpenWrt工具：$tool"
done
[ "$(uname -s)" = Linux ] || fail '仅适用于OpenWrt／ImmortalWrt'
MANAGER=''
ABI=''
CANDIDATES=''
if command -v apk >/dev/null 2>&1; then
  ABI=$(apk --print-arch 2>/dev/null || true)
  if [ -n "$ABI" ]; then
    MANAGER=apk
    DEFAULT_ARCH=$ABI
    # 默认CPU架构不等于固件包ABI；和APK自身一样优先读取持久架构配置。
    if [ -f "/etc/apk/arch" ]; then
      configured=$(awk 'NF==1 {print $1}' "/etc/apk/arch") || fail '读取APK架构配置失败'
      ABI=$(printf '%s\n' "$configured" | awk '($1=="aarch64_cortex-a53" || $1=="aarch64_generic" || $1=="x86_64") && !seen[$1]++ {print $1}')
      [ -n "$ABI" ] || ABI=$configured
    fi
    CANDIDATES=$ABI
    for candidate in $CANDIDATES; do
      case "$DEFAULT_ARCH:$candidate" in
        aarch64:aarch64_cortex-a53|aarch64_cortex-a53:aarch64_cortex-a53|aarch64_generic:aarch64_cortex-a53|aarch64:aarch64_generic|aarch64_generic:aarch64_generic|aarch64_cortex-a53:aarch64_generic|x86_64:x86_64) ;;
        *) fail "APK默认架构与可用包ABI尚未覆盖或不一致：$DEFAULT_ARCH / $ABI" ;;
      esac
    done
  fi
fi
if [ -z "$MANAGER" ] && command -v opkg >/dev/null 2>&1; then
  configured=$(opkg print-architecture) || fail '读取OPKG架构配置失败'
  # 同族多 ABI 是兼容列表；按正优先级降序，同分时固定优先具体架构。
  CANDIDATES=$(printf '%s\n' "$configured" | awk '
    BEGIN {rank["aarch64_cortex-a53"]=2; rank["aarch64_generic"]=1; rank["x86_64"]=1}
    $1=="arch" && $2 in rank {
      if (NF!=3 || $3 !~ /^[0-9]+$/ || $3+0>2147483647) {bad=1; next}
      if ($3+0>0 && (!( $2 in priority ) || $3+0>priority[$2])) priority[$2]=$3+0
    }
    END {
      if (bad) exit 1
      for (i=0;i<3;i++) {
        best=""; score=-1
        for (name in priority) if (priority[name]>score || (priority[name]==score && rank[name]>rank[best])) {best=name; score=priority[name]}
        if (best=="") break
        print best; delete priority[best]
      }
    }') || fail 'OPKG架构优先级格式异常；不猜测安装包'
  MANAGER=opkg
fi
[ -n "$CANDIDATES" ] || fail '尚未提供此设备ABI的安装包：未识别；不强制安装'
CPU=$(uname -m)
for candidate in $CANDIDATES; do
  case "$CPU:$candidate" in
    aarch64:aarch64_cortex-a53|aarch64:aarch64_generic|arm64:aarch64_cortex-a53|arm64:aarch64_generic|x86_64:x86_64|amd64:x86_64) ;;
    *) fail "CPU与候选ABI不兼容：$CPU / $candidate；不跨架构尝试" ;;
  esac
done
select_package() {
  ABI=$1
  case "$ABI" in
  aarch64_cortex-a53) ARCH=aarch64 ;;
  aarch64_generic) ARCH=aarch64_generic ;;
  x86_64) ARCH=x86_64 ;;
  *) fail "尚未提供此设备ABI的安装包：${ABI:-未识别}；不强制安装" ;;
esac
case "$MANAGER:$ARCH" in
  apk:aarch64) EXT=apk; PKG_SHA='@APK_AARCH64_SHA256@'; PKG_BYTES=@APK_AARCH64_BYTES@ ;;
  apk:aarch64_generic) EXT=apk; PKG_SHA='@APK_AARCH64_GENERIC_SHA256@'; PKG_BYTES=@APK_AARCH64_GENERIC_BYTES@ ;;
  apk:x86_64) EXT=apk; PKG_SHA='@APK_X86_64_SHA256@'; PKG_BYTES=@APK_X86_64_BYTES@ ;;
  opkg:aarch64) EXT=ipk; PKG_SHA='@IPK_AARCH64_SHA256@'; PKG_BYTES=@IPK_AARCH64_BYTES@ ;;
  opkg:aarch64_generic) EXT=ipk; PKG_SHA='@IPK_AARCH64_GENERIC_SHA256@'; PKG_BYTES=@IPK_AARCH64_GENERIC_BYTES@ ;;
  opkg:x86_64) EXT=ipk; PKG_SHA='@IPK_X86_64_SHA256@'; PKG_BYTES=@IPK_X86_64_BYTES@ ;;
  *) fail '没有识别到可用的包管理器' ;;
esac
PACKAGE="wlmb-server-$VERSION-$ARCH.$EXT"
}
TOTAL_BYTES=0
for candidate in $CANDIDATES; do
  select_package "$candidate"
  TOTAL_BYTES=$((TOTAL_BYTES + PKG_BYTES))
done
verify() { printf '%s  %s\n' "$2" "$1" | sha256sum -c - >/dev/null 2>&1; }
space() {
  available=$(df -Pk "$1" | awk 'END {print $4}')
  case "$available" in ''|*[!0-9]*) fail "无法检查可用空间：$1" ;; esac
  [ "$available" -ge "$2" ] || fail "$1 可用空间不足，需要至少 $2 KiB"
}
CURRENT=''
if [ -x "/usr/sbin/wlmb-server" ]; then
  CURRENT=$("/usr/sbin/wlmb-server" --version 2>/dev/null | sed -n 's/^WLMB_BUILD=\([^;]*\);.*/\1/p')
  if [ -n "$CURRENT" ] && awk -v a="$CURRENT" -v b="$VERSION" 'BEGIN {split(a,x,"."); split(b,y,"."); for(i=1;i<=3;i++){if(x[i]+0>y[i]+0)exit 0; if(x[i]+0<y[i]+0)exit 1}exit 1}'; then
    fail "当前版本 $CURRENT 高于安装器版本 $VERSION，请使用新版安装器"
  fi
fi
printf '网络面板 %s · %s\n兼容ABI候选（按固件优先级）：\n%s\n' "$VERSION" "$MANAGER" "$CANDIDATES"
if [ -n "$DOWNLOAD_PREFIX" ] && [ -z "$OFFLINE" ]; then
  printf '%s\n' '下载通道：第三方gh-proxy.com代理；仍核对固定版本摘要及APK发布公钥，不绕过验签。'
fi
[ -z "$CURRENT" ] || printf '当前版本：%s；保留已有插件配置。\n' "$CURRENT"
if [ "$CHECK" = 0 ] && [ "$YES" = 0 ]; then
  printf '%s\n' '将安装网络面板；APK首次自动核对并信任SlAlot发布公钥。' '只使用固件现有依赖源；不改拨号、WiFi，不重启设备。'
  printf '是否继续？[y/N] '
  answer=''
  IFS= read -r answer || fail '未收到确认；自动化调用需明确使用 --yes'
  case "$answer" in y|Y|yes|YES) ;; *) printf '已取消，未安装、未导入公钥。\n'; exit 0 ;; esac
fi
space /tmp "$((TOTAL_BYTES / 1024 + 16384))"
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
    [ ! -L "$OFFLINE/$name" ] || fail "离线文件是符号链接：$name"
    [ -f "$OFFLINE/$name" ] || { printf '离线目录缺少文件：%s\n' "$name" >&2; return 1; }
    cp "$OFFLINE/$name" "$target" || { printf '无法读取离线文件：%s\n' "$name" >&2; return 1; }
  elif command -v curl >/dev/null 2>&1; then
    curl --fail --location --proto '=https' --proto-redir '=https' --connect-timeout 10 --max-time 120 --retry 2 --output "$target" "$BASE/$name" || { rm -f "$target"; return 1; }
  elif command -v wget >/dev/null 2>&1; then
    wget -T 60 -O "$target" "$BASE/$name" || { rm -f "$target"; return 1; }
  else
    fail '没有curl或wget；请使用 --offline 文件目录'
  fi
  [ -s "$target" ] || fail "下载文件为空：$name"
}
if [ "$MANAGER" = apk ]; then
  fetch wlmb-server-signing.pem "$TEMP/wlmb-server-signing.pem" || fail '发布公钥下载失败；请检查网络或离线目录'
  verify "$TEMP/wlmb-server-signing.pem" "$KEY_SHA" || fail '公钥指纹不匹配，停止安装'
  mkdir "$TEMP/keys"
  cp "$TEMP/wlmb-server-signing.pem" "$TEMP/keys/wlmb-server-signing.pem"
  [ ! -L "/etc/apk/keys" ] && [ ! -L "$TRUST" ] || fail '系统公钥路径是符号链接，请手动检查'
  if [ -e "$TRUST" ]; then verify "$TRUST" "$KEY_SHA" || fail '已存在不同的同名公钥，拒绝自动替换'; fi
fi
apk_action() {
  if [ -n "$OFFLINE" ]; then apk --no-network "$@"; else apk "$@"; fi
}
preflight() {
  if [ "$MANAGER" = apk ]; then apk_action add --simulate "$TEMP/$PACKAGE"; else opkg --noaction install "$TEMP/$PACKAGE"; fi
}
if [ "$CHECK" = 0 ] && [ "$MANAGER" = opkg ] && [ -n "$OFFLINE" ]; then
  for dependency in rpcd rpcd-mod-iwinfo luci-base uci; do
    opkg status "$dependency" | grep -Eq '^Status:.* installed$' || fail "离线安装缺少依赖 $dependency，请先通过固件自身源补齐"
  done
fi
SELECTED=0
for pass in 1 2; do
  PREFLIGHT_ATTEMPTED=0
  for candidate in $CANDIDATES; do
    select_package "$candidate"
    printf '正在检查 %s（ABI：%s）…\n' "$PACKAGE" "$ABI"
    if [ ! -f "$TEMP/$PACKAGE" ]; then
      if ! fetch "$PACKAGE" "$TEMP/$PACKAGE"; then
        printf '候选包无法获取：%s；继续检查其他兼容候选。\n' "$PACKAGE" >&2
        continue
      fi
    fi
    # 安全校验失败立即终止，不能通过换包掩盖篡改或签名异常。
    verify "$TEMP/$PACKAGE" "$PKG_SHA" || fail "安装包摘要不匹配：$PACKAGE；停止安装"
    if [ "$MANAGER" = apk ]; then
      apk --keys-dir "$TEMP/keys" verify "$TEMP/$PACKAGE" || fail 'APK签名验证失败；不跳过校验'
    fi
    if [ "$CHECK" = 1 ]; then SELECTED=1; break; fi
    if [ "$MANAGER" = apk ] && [ ! -e "$TRUST" ]; then
      mkdir -p "/etc/apk/keys"
      cp "$TEMP/wlmb-server-signing.pem" "$TRUST" || fail '公钥导入失败'
      NEW_KEY=1
      chmod 644 "$TRUST"
    fi
    PREFLIGHT_ATTEMPTED=1
    if preflight; then SELECTED=1; break; fi
    printf '候选模拟安装未通过：%s；尚未正式安装，继续检查其他兼容候选。\n' "$ABI" >&2
  done
  [ "$SELECTED" = 0 ] || break
  [ "$pass" = 1 ] && [ -z "$OFFLINE" ] && [ "$PREFLIGHT_ATTEMPTED" = 1 ] || break
  printf '全部候选暂未通过，刷新一次固件现有源索引后复查；不执行全系统升级…\n'
  "$MANAGER" update || fail '固件软件源刷新失败；不替换软件源，请检查源和网络'
done
[ "$SELECTED" = 1 ] || fail '所有兼容ABI候选均未通过获取或模拟安装检查；未开始安装，请查看上方各候选错误'
if [ "$CHECK" = 1 ]; then
  printf '检查通过：%s的版本、ABI、摘要%s；未模拟依赖安装、未安装、未导入系统公钥。\n' "$ABI" "$(if [ "$MANAGER" = apk ]; then printf '及APK签名'; fi)"
  exit 0
fi
INSTALL_STARTED=1
printf '正在安装网络面板…\n'
if [ "$MANAGER" = apk ]; then
  apk_action add "$TEMP/$PACKAGE" || fail '包管理器安装失败；可能已写入部分文件，请查看上方提示，不自动强制回滚'
else
  opkg install "$TEMP/$PACKAGE" || fail '包管理器安装失败；可能已写入部分文件，请查看上方提示，不自动强制回滚'
fi
marker=$("/usr/sbin/wlmb-server" --version 2>/dev/null || true)
case "$marker" in "WLMB_BUILD=$VERSION;"*) ;; *) fail '安装后版本未匹配；请检查包管理器输出' ;; esac
ready=0
for attempt in 1 2 3 4 5 6 7 8 9 10; do
  # 本地套接字校验权限、服务响应与版本；不向受保护的网页接口发送无会话请求。
  if "/usr/sbin/wlmb-server" --check-ready >/dev/null 2>&1; then ready=1; break; fi
  sleep 1
done
[ "$ready" = 1 ] || fail '安装包已写入，但同版服务尚未就绪；无需重复安装，请运行 /etc/init.d/wlmb-server status 和 logread -e wlmb-server 排查'
printf '\n安装完成：网络面板 %s\n后台入口：LuCI → 服务 → 网络面板\n' "$VERSION"
address=$(uci -q get network.lan.ipaddr 2>/dev/null || true)
address=${address%%/*}
case "$address" in ''|*[!0-9.]*) address='你的路由器管理地址' ;; esac
printf '小程序选择 openwrt，连接地址：%s:1117\n' "$address"
printf '未修改拨号、WiFi和防火墙配置，未重启设备。\n'
