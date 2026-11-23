# Sios 本机流量采集

Sios 每六秒向已有本地 Unix 套接字发送一次只读请求。请求沿用 status 的帧格式。

    {"method":"status","args":{"view":"traffic-counters-v1"}}

成功响应的 data 包含 view、backend 和 system。system 包含 observedAtMs、uptimeSeconds、stale 以及 network。network 包含默认接口名、计数来源、从启动开始的上下行字节数和现有速率。首个观测的速率可以为空。采样时间超过十五秒或系统时间回拨时标记为过期。

服务从现有系统采样缓存投影这些字段。普通 status、HTTP 和 LuCI 保持既有接口。旧版服务忽略 view 并返回完整快照，其中包含同一组流量字段。
