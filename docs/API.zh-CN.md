# 网络面板 1.0 接入 API

服务：wlmb-server；协议：NETWORK-PANEL-OPENWRT/1；契约版本1；端口1117。1.0.0插件源码按非商业使用及来源声明许可公开，兼容现有小程序接入；客户端实现和商业授权不在公开范围。机器可读定义见[api-contract.json](api-contract.json)。

## 地址、认证与响应

落叶小通魔改的modem数据源使用`modems[].infoTransport=luoye-modem-cache`，仍走原登录和状态接口，不依赖管理地址缺省值。参数时间基于原文件发布时刻，标记`observationTimeBasis=source-file-publication`；邻区有独立时间戳，未知事务结果时`radio.neighborState.automaticRetryPaused=true`。短信、AT控制台和锁定能力在此来源关闭，客户端必须遵循capabilities，详见[接入说明](LUOYE-MODEM.md)。

使用设备实际LAN管理地址，例如 `http://192.168.1.1:1117`；访客网络可能隔离LAN。不要把默认地址当成所有设备地址，不应公开暴露端口。

`GET /health`无需认证，返回backend/version/protocol等公开状态。其他接口为JSON POST；根账户通过系统登录核验后获取插件令牌，后续用 `Authorization: Bearer <token>`。令牌1800秒过期并绑定连接IP，不是系统root会话，客户端不应持久保存路由器密码。

```json
{"username":"root","password":"用户自行输入"}
```

响应为 `{"ok":true,"data":{...}}`，或 `{"ok":false,"error":{"code":"...","message":"...","automaticRetry":false}}`。请求最大16384字节；错误码和能力是控制依据，不能从HTTP状态码推断写操作可安全重试。

## 读取接口

| 路径 | 参数 | 说明 |
| --- | --- | --- |
| /api/login | username,password | 创建插件会话 |
| /api/logout | 无 | 销毁会话 |
| /api/status | modem可选 | 当前系统／模组／信号／载波／邻区／能力缓存 |
| /api/settings | 无 | 插件配置与通道模式 |
| /api/diagnostics | 无 | 脱敏错误与运行边界 |
| /api/control_result | job | 只查询回执，不再次执行 |
| /api/sms_list | modem | 共用短信缓存，不发新查询 |
| /api/sms_forward_settings | 无 | 开关、有效间隔、tokenSet与运行状态，绝不返回Token |

status的modems含id、model、platform和通道信息；selectedModem指定当前模组。路由器名称取system.board.model，不用模组型号代替。未知、过期或未支持的数据不得填假零或借用其他载波的信号。

频段预设source=reviewed-model-supplement表示型号补充清单，supplement.NR列出统一NR增补项；不改变NRSA、NRNSA的核验范围，不等同于negotiated-firmware。

radio.refreshDeferred表示主测量查询因通道暂忙而延后；仍在15秒有效期内时保留原测量和原时间戳，partial为true，不把忙碌误报为已断网。超期或身份／解析异常仍标为stale，客户端不得自行延长有效期。

radio.serving是服务测量；radio.carriers是已报告载波库存，两者不能简单合并成额外载波。role／roleKnown、channelType以及bandCandidates应保留；NR服务SSB频点不是CA中心频点。邻区带候选频段时，不应直接视为确定频段用于锁定。

原生 QModem 查询通过 radio.carrierState／neighborState 区分服务载波、已报告库存、空测量和查询错误。serving-only 不表示已读取全部聚合载波；neighborState.status=empty 不表示附近没有基站。queryStatus=error 时可能保留同一驻留身份下的历史记录，必须遵守各记录原 observedAtMs／stale，不将其当作新测量。capabilities.neighbors 在原生路径仅表示经核验的被动读取，不授予主动搜网或锁定权限。详见 [QModem 接入说明](QMODEM.md)。

签约速率、APN、QCI、模组温度、运营商按对应字段及stale状态呈现。capabilities按功能逐项控制，接口存在并不等于能力已开放。

1.1.2新增`modems[].infoTransport=qmodem-rpc`表示使用QModem原生信息查询，不授予AT写入权限。原生快照的`providerCached`和`sourceCacheAgeUnknown`说明上游缓存年龄无法确认。

`modems[].infoInventorySource=qmodem.send_at` 表示借用 QModem 共享 AT 传输，由插件独立解析信号、载波及邻区；不表示可任意发送 AT、锁定或操作短信。参数编码按实际 ubus 签名检测。原始测量及独立有效期继续适用；`radio.infoSupplementDeferred=true` 表示本轮通道不确定，已停止追加查询，没有自动重试。

`system.lan.deviceBlockReason`说明上网控制不可用原因：`FLOW_OFFLOAD_ENABLED`为已启用流量加速，`FLOW_OFFLOAD_UNCONFIRMED`为加速状态未确认，`FIREWALL_UNAVAILABLE`为防火墙不可读，`LAN_INTERFACE_UNCONFIRMED`／`WAN_INTERFACE_UNCONFIRMED`为接口未确认，`FORWARD_PATH_AMBIGUOUS`为转发路径不明确，`HOST_STATE_STALE`为控制快照过期。能力可用时为null；客户端仍以`deviceBlockSupported`及过期标记控制操作，不自行更改加速设置。

## 异步动作与确认

长动作返回 `{"job":"...","state":"pending"}`。只轮询control_result，可得到pending、done/result或error/error。执行连接中断时不要自动重发。

- refresh：显式刷新模组，仍遵循后端限流。
- lock_status：modem、force，读取配置。1.0.1追加`cellFields.NR/LTE`（`bandRequired`／`scsRequired`），`presets.cellRanges.NR/LTE`提供与写入校验一致的`band/first/last`候选范围。客户端仅在候选唯一时补齐频段，SCS只能使用明确测量或用户确认值。旧服务缺少字段时保守保留必填项，不能因此开放新写协议。
- lock_prepare → lock_execute：modem、operation；execute附revision、confirmation。
- host_prepare → host_execute：operation；execute附revision、confirmation。
- at_prepare → at_execute：modem、command；execute附confirmation。
- sms_refresh：modem，手动刷新，通常有30秒保护；后台收取缓存优先复用。
- sms_prepare → sms_execute：modem、operation；execute附confirmation。
- sms_read_state：modem、id、unread，仅更新插件独立已读账本。
- sms_settings_prepare → sms_settings_execute：modem、enabled、confirmation；仅实际核验的IMS配置和配置权限可用。
- sms_forward_configure：enabled、receiveEnabled、modem、pollSeconds、token、clearToken，需认证，返回异步回执。

prepare的确认绑定会话、内容、身份，通常30秒有效、只能消费一次；回执也绑定原会话。需要重新读取和确认才能再次执行。

## 锁定与主机操作

```json
{"rat":"NR","mode":"band","bands":[78]}
{"rat":"NR","mode":"cell","entries":[{"band":78,"arfcn":627264,"pci":488,"scsType":1}]}
{"rat":"NR","mode":"auto","entries":[]}
```

NR_NSA只用于频段偏好；小区为NR或LTE。SCS是否必需由协议决定，不猜默认。可选target=band明确只解除频段锁，不串成小区解锁。保存回读成功与射频实际生效分开，插件不自动飞行模式或重启；acceptedAtMs之后的新鲜驻留条件只能显示符合条件，不能证明因果或重启持久。

主机operation类型为wifi_edit、wifi_unify、device_block、traffic_limit。仅操作核验的普通LAN AP和插件自有规则；复杂AP／STA／MLO不假装可编辑。端口速率rateScope=port不是精准逐IP统计，详见[LAN-RATES](LAN-RATES.md)。

## 短信与转发

短信id是指纹，不是原始SIM槽位。字段含phone/content/timestamp/direction/unread/readStateSource/complete；backend.source和deleteScope标明来源。qmodem-record只删除数据库记录，不保证删除SIM副本。

发送operation为 `{"type":"send","number":"号码","content":"正文"}`，删除为 `{"type":"delete","id":"指纹"}`。发送／删除无自动重试；不完整长短信不转发。独立已读不改模组标记。

转发默认关闭，有效pollSeconds5–300、默认10；扫描完成后计时，事件优先，忙碌退避。空token保留原值；clearToken要求转发关闭，可保留独立收取。默认不推历史。已排队短信可合并为一次PushPlus请求（最多8条、按正文大小缩小），不额外等待；counts.accepted／pending／unknown均按短信条数而非HTTP请求数统计。详细来源、延迟和资源限制见[SMS](SMS.md)。

## 安全与客户端边界

LAN HTTP未加密，只用可信LAN。不要记录密码、Token、正文或原始会话；不要以版本号替代capabilities，不把stale数据当当前值，不通过任意shell／UCI／串口路径绕过后端。小程序收费授权在独立客户端内，本接口没有专业版付费门禁。

[安全](SECURITY.md) · [QModem参考](QMODEM.md) · [功能范围](FEATURES.md)
