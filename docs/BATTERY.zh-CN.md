# OpenWrt 电池接入

从网络面板 1.1.5 起，Rust 后端在已有 `POST /api/status` 的 `system.battery` 返回统一电池数据。仍使用设备实际 LAN 地址的 1117 端口，不新增端口、远程写入入口或任意脚本执行。LuCI 的 `network-panel.status` 返回同一份数据。

## 标准节点与开发者适配

1. 没有开发者数据文件时，自动发现 OpenWrt 的 `/sys/class/power_supply/*`：只选择 `type=Battery` 且未明确拔出的电池，读取 `capacity` 和 `status`，不固定电池名称。`capacity` 必须为 0～100 整数；USB／AC 电源不作为电池。属性含义见[内核 power_supply 文档](https://docs.kernel.org/power/power_supply_class.html)。
2. 特殊驱动或厂商固件没有这些节点时，由固件开发者使用自己的采集服务生成 `/var/run/wlmb-server/battery.json`。后端不猜测厂商命令、串口或驱动协议。
3. 多块有效电池时不选第一块、不直接相加；开发者应明确选择或归一化后输出数据。无电池设备不会伪装成 0% 或 100%。

固定数据文件存在时，它是优先且明确的数据源。文件错误、不可信或过期时返回不可用，**不悄悄改用另一块电池**。删除文件后，下次系统采样恢复节点自动发现。

## 数据文件

```json
{"version":1,"percent":76,"charging":true,"updatedAt":1767225600}
```

示例时间戳只说明格式，实际必须填写本次真实采集时间。

| 字段 | 规则 |
| --- | --- |
| `version` | 整数 `1` |
| `percent` | 0～100 整数；0 是有效电量；未知填 `null`，不可填字符串、负数、255 或假零 |
| `charging` | 正在充电为 `true`，确认未充电为 `false`；未知填 `null` 或省略，不根据电量猜测 |
| `updatedAt` | 本次测量的 Unix 秒时间戳，不是毫秒；只在取得新测量时更新，不给旧缓存续期 |

拒绝重复字段、额外字段和超过 1024 字节的数据。超过 60 秒或明显来自未来的报告不可用。建议采集服务每 15～30 秒更新，实际频率取决于设备采集成本；不要求高频执行 AT。

目录必须是 root 所有的真实目录，目录与文件不能允许组或其他用户写入；文件必须是 root 所有的普通文件，不能是符号链接、硬链接或管道。推荐目录 `0700`、文件 `0600`，使用运行目录，避免反复写闪存。

## 原子发布示例

以下片段由开发者的 root 采集服务执行。两个变量应先由已经适配的固件接口获取，不能用固定示例值冒充实时数据：

```sh
# 上游采集成功后验证结果，不把失败转换成 0。
case "$battery_percent" in ''|*[!0-9]*) exit 1 ;; esac
[ "${#battery_percent}" -le 3 ] && [ "$battery_percent" -le 100 ] || exit 1
# 去掉十进制前导零，保证输出合法 JSON。
battery_percent=$(printf '%s' "$battery_percent" | sed 's/^0*//;s/^$/0/')
case "$battery_charging" in true|false|null) ;; *) exit 1 ;; esac
[ ! -L /var/run/wlmb-server ] || exit 1
mkdir -p /var/run/wlmb-server || exit 1
chmod 700 /var/run/wlmb-server || exit 1
umask 077
battery_tmp=$(mktemp /var/run/wlmb-server/.battery.XXXXXX) || exit 1
trap 'rm -f "$battery_tmp"' EXIT HUP INT TERM
battery_time=$(date +%s) || exit 1
printf '{"version":1,"percent":%s,"charging":%s,"updatedAt":%s}\n' \
  "$battery_percent" "$battery_charging" "$battery_time" > "$battery_tmp" || exit 1
chmod 600 "$battery_tmp" || exit 1
mv -f "$battery_tmp" /var/run/wlmb-server/battery.json || exit 1
```

先写临时文件再在同目录重命名，不直接覆盖正式文件。采集失败时停止续期，让消费者看到过期状态，或明确发布 `percent:null`。更新无需重启网络面板、拨号或无线网络。

## 客户端读取

使用原登录流程取得 Token，查询已有接口：

```sh
curl -X POST 'http://设备LAN地址:1117/api/status' \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $TOKEN" -d '{}'
```

成功响应的局部结构：

```json
{
  "ok":true,
  "data":{
    "capabilities":{"battery":true},
    "system":{"battery":{
      "supported":true,"available":true,"percent":76,"charging":true,
      "source":"provider","observedAtMs":1767225600000,"stale":false,"reason":null
    }}
  }
}
```

`source` 为 `provider` 或 `power-supply`。`observedAtMs` 是测量／读取时间，不是 HTTP 请求时间。后端约每 3 秒更新共用系统快照，多客户端不叠加查询；标准节点的时间是软件读取时刻，不冒充硬件采样时刻。

仅在 `system.stale !== true`、`battery.available === true`、`battery.stale === false` 且 `percent` 为 0～100 整数时显示。`charging:null` 代表未知。`capabilities.battery` 表示当前可读；老插件缺字段时视为未提供，不从路由存在推断支持。

不可用时 `percent`、`charging` 为 `null`，`available:false`、`stale:true`。`supported:true` 仍可表示发现电池／配置了来源，但当前值不能使用。排查原因：

| reason | 含义 |
| --- | --- |
| BATTERY_NOT_DETECTED | 没有检测到电池 |
| BATTERY_AMBIGUOUS | 多块电池，等待明确适配 |
| BATTERY_VALUE_UNAVAILABLE | 来源存在，读数不可用 |
| BATTERY_REPORT_INVALID | 报告格式或数值无效 |
| BATTERY_REPORT_UNTRUSTED | 类型、归属、权限或读取校验失败 |
| BATTERY_REPORT_EXPIRED | 报告过期或时间无效 |
| BATTERY_SOURCE_STALE | 系统采集停止或缓存过期 |
| BATTERY_DISCOVERY_LIMIT | 电源节点超出有界发现上限 |

1117 接口继续要求登录鉴权及局域网限制。请求不能指定文件路径、执行采集命令或修改充放电策略。电池采集失败不影响其他系统、模组和短信能力。
