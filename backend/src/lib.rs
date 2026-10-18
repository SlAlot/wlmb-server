//! 网络面板服务模块与统一错误类型；对外错误说明不泄露底层敏感回复。
pub mod actions;
pub mod at_channel;
pub mod band_inventory;
pub mod band_locks;
pub mod blob;
pub mod boundary;
pub mod catalog;
#[cfg(unix)]
pub mod compat_sms;
pub mod connection_mode;
pub mod control_runtime;
pub mod controls;
pub mod events;
#[cfg(unix)]
pub mod glinet;
pub mod host_controls;
pub mod http;
pub mod independent_at;
#[cfg(unix)]
pub mod lan;
pub mod lan_ports;
pub mod lock_protocol;
pub mod modem_inventory;
#[cfg(unix)]
pub mod native_host;
#[cfg(unix)]
pub mod native_ubus;
pub mod network_operator;
#[cfg(unix)]
pub mod private_store;
pub mod qmodem_info;
#[cfg(unix)]
pub mod qmodem_queue;
pub mod qmodem_sms;
pub mod quectel_lock;
pub mod radio;
pub mod radio_controls;
#[cfg(unix)]
pub mod rpc;
pub mod sampler;
pub mod sampling;
pub mod security;
#[cfg(unix)]
pub mod service;
pub mod session_qos;
#[cfg(unix)]
pub mod shared_sms;
pub mod sms;
pub mod sms_forward;
pub mod sms_notice;
pub mod sms_pdu;
pub mod sms_read_state;
pub mod sms_runtime;
pub mod sms_session;
pub mod sms_settings;
pub mod sms_text;
pub mod telemetry;
pub mod temperature;
pub mod traffic;
#[cfg(unix)]
pub mod transport;
pub mod vendor_cell_lock;

pub const PROTOCOL: &str = "NETWORK-PANEL-OPENWRT/1";
pub const API_PORT: u16 = 1117;
pub const MAX_BODY: usize = 16_384;
pub const MAX_AT_RESPONSE: usize = 65_536;
pub const MAX_SAMPLE_QUERIES: usize = 10;
pub const VERSION: &str = "1.1.2";
pub const BUILD_MARKER: &str = if cfg!(feature = "pushplus-https") {
    "WLMB_BUILD=1.1.2;pushplus-https=1"
} else {
    "WLMB_BUILD=1.1.2;pushplus-https=0"
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelError {
    pub code: &'static str,
    pub status: u16,
}
impl PanelError {
    pub fn new(code: &'static str, status: u16) -> Self {
        Self { code, status }
    }
    pub fn details(&self) -> serde_json::Value {
        let uncertain = matches!(
            self.code,
            "SMS_SEND_UNKNOWN"
                | "SMS_DELETE_UNKNOWN"
                | "SMS_DELETE_PARTIAL"
                | "SMS_INPUT_UNKNOWN"
                | "AT_TIMEOUT"
                | "AT_WRITE_UNKNOWN"
                | "LOCK_RESULT_UNKNOWN"
                | "WIFI_RELOAD_UNKNOWN"
                | "WIFI_NETWORK_CHANGED"
                | "WIFI_READBACK_MISMATCH"
        );
        let category = if uncertain {
            "uncertain"
        } else {
            match self.code {
                "SMS_SHARED_READ_ONLY"
                | "LOCK_UNSUPPORTED"
                | "LOCK_BAND_PROTOCOL_UNSUPPORTED"
                | "LOCK_VARIANT_UNSUPPORTED"
                | "SMS_SETTINGS_UNSUPPORTED"
                | "FEATURE_NOT_READY" => "pending",
                "AT_UNSUPPORTED"
                | "LOCK_REJECTED"
                | "SMS_PRESERVE_UNSUPPORTED"
                | "SMS_ENCODING_UNSUPPORTED"
                | "SMS_BINARY_UNSUPPORTED"
                | "SMS_TEXT_ENCODING_UNSUPPORTED" => "unsupported",
                "AT_BUSY"
                | "SMS_BUSY"
                | "SYSTEM_BUSY"
                | "LOCK_BUSY"
                | "AT_COOLDOWN"
                | "SMS_REFRESH_COOLDOWN" => "busy",
                "HOST_CHANGED"
                | "LOCK_CHANGED"
                | "MODEM_CHANGED"
                | "SMS_CHANGED"
                | "SMS_STORAGE_CHANGED"
                | "SHARED_QUEUE_CHANGED" => "changed",
                "CONFIRMATION_INVALID" | "CONFIRMATION_EXPIRED" | "LOCK_CONFIRMATION_REQUIRED" => {
                    "confirmation"
                }
                "SMS_TEXT_VERIFY_PENDING" | "SMS_MULTIPART_INCOMPLETE" => "pending-read",
                _ if self.status == 401 || self.status == 403 => "authorization",
                _ if self.status >= 500 => "unavailable",
                _ => "validation",
            }
        };
        serde_json::json!({"code":self.code,"message":self.message(),"category":category,"operationMayHaveApplied":uncertain,"automaticRetry":false})
    }
    pub fn message(&self) -> &'static str {
        match self.code {
            "SMS_SETTINGS_UNSUPPORTED" => {
                "此模组没有已核验的 IMS 短信开关协议；不等于无法收短信，未发送配置命令"
            }
            "SMS_SETTINGS_UNAVAILABLE" => "短信 IMS 配置尚未可靠读取，未执行修改",
            "SMS_SETTINGS_UNCONFIRMED" => "短信配置结果未确认，未自动重试；请重新读取状态",
            "AT_WRITE_DISABLED" => "请先在插件设置中保存开启允许配置型操作，每次修改仍需确认",
            "LOCK_BAND_UNSUPPORTED" => "所选频段不在此模组的已核验预设中，请重新读取选项",
            "LOCK_BAND_CELL_CONFLICT" => {
                "此固件频段与小区共用配置，恢复频段可能解除小区；本次未写入，请先确认是否解除小区"
            }
            "LOCK_UNSUPPORTED" => "此模组或固件尚未开放已核验锁定协议，未执行操作",
            "AT_UNSUPPORTED" => "当前指令被模组或通道拒绝，不等于所有短信功能均不支持",
            "SMS_SHARED_READ_ONLY" => {
                "当前共享通道未确认完整短信事务保护，仅开放读取与转发；不会抢占供网端口或自动升级 QModem"
            }
            "SHARED_QUEUE_CHANGED" => {
                "共享通道的版本或队列契约已变化，已停止操作；请重新检查兼容性，不要重复提交"
            }
            "SMS_INPUT_UNKNOWN" => {
                "模组可能仍在等待短信内容，已隔离通道；发送结果不确定，请核对状态，不要再次发送"
            }
            "LOCK_VARIANT_UNSUPPORTED" => "此固件的锁定回复与已实现协议不同，未尝试其他写入格式",
            "LOCK_BAND_PROTOCOL_UNSUPPORTED" => {
                "此平台的锁频段协议尚未接入，不代表模组不支持锁频段"
            }
            "LOCK_UNVERIFIED" => "锁定配置回复未能可靠解析，未开放修改，请检查诊断记录",
            "LOCK_BAND_INVENTORY_REQUIRED" => {
                "尚未读取到此模组可靠的支持频段范围，未提交锁定；请先刷新配置"
            }
            "LOCK_REQUIRES_LTE_MODE" => {
                "此模组在自动制式下会忽略频段限制，请先在设备后台选择 4G；插件不会代你更改选网"
            }
            "LOCK_TOO_MANY_BANDS" => "此模组一次最多选择 10 个频段，请减少选择后再保存",
            "LOCK_CHANGED" => "锁定配置已被其他操作更改，本次未写入；请重新读取后确认",
            "LOCK_CONFIRMATION_REQUIRED" => "操作确认已失效，请重新读取并确认，不会自动重发",
            "LOCK_REJECTED" => "模组拒绝了锁定命令，未自动切换协议、重启或重试",
            "LOCK_RESULT_UNKNOWN" => "锁定命令结果未确认；请先读取状态，不要重复提交",
            "MODEM_CHANGED" | "MODEM_NOT_FOUND" => "模组已更换或离线，请刷新设备列表后重新选择",
            "AT_NOT_READY" | "AT_CHANNEL_UNAVAILABLE" => {
                "模组通道暂不可用，请检查接口状态；插件不会抢占供网端口"
            }
            "AT_COOLDOWN" => "上次通道响应不确定，正在保护等待；请稍后读取状态，不要重发修改",
            "WIFI_CHANNEL_UNSUPPORTED" => "该信道未由当前无线驱动开放，请重新读取可选信道",
            "WIFI_BANDWIDTH_UNSUPPORTED" => "该频宽未由当前无线驱动报告，请重新读取可选频宽",
            "WIFI_MODE_UNSUPPORTED" => "设备未开放该 WiFi 代际，请重新读取可选模式",
            "WIFI_SECURITY_UNSUPPORTED" => "系统未确认支持此加密方式，请重新读取选项",
            "WIFI_PASSWORD_REQUIRED" => "切换加密方式需要重新填写 8–63 位密码",
            "WIFI_PASSWORD_REQUIRES_SECURITY" => "当前为开放网络，请先选择加密方式再设置密码",
            "WIFI7_SECURITY_REQUIRED" => "WiFi 7 请搭配 WPA3 或 WPA2 / WPA3，或先选择 WiFi 6",
            "WIFI_UNIFY_SECURITY_REQUIRED" => "请先将两个频段设置为一致的加密方式，再开启双频合一",
            "AT_PORT_CONFLICT" => "AT 端点被其他程序占用；插件不会抢占，请选择已确认空闲的 AT 口",
            "AT_CHANNEL_QUARANTINED" => "通道上次响应不确定，已隔离；不要重复提交写操作",
            "SMS_PDU_MODE_REQUIRED" => {
                "此操作需要当前通道使用 PDU 格式；插件不会自动修改短信模式，可读取的文本短信不受此限制"
            }
            "SMS_STORAGE_UNAVAILABLE" => "短信存储区未能可靠读取；未自动更换存储区或删除短信",
            "SMS_TEXT_SETTINGS_REQUIRED" => {
                "文本短信需要设备当前使用 UCS2 和完整消息头；插件不会自动修改这些设置"
            }
            "SMS_TEXT_VERIFY_PENDING" => {
                "正在分批核验文本短信，请稍后刷新；未核验的消息不会显示或转发"
            }
            "SMS_TEXT_INVALID" => "文本短信回复不完整或前后不一致，未猜测正文；请稍后重新读取",
            "SMS_TEXT_ENCODING_UNSUPPORTED" => {
                "当前文本存储含暂不支持的编码、分段或草稿；未误分类或转发"
            }
            "SMS_DELETE_UNKNOWN" => "短信删除结果未确认；没有自动重试，请先重新读取短信",
            "SMS_PRESERVE_UNSUPPORTED" => {
                "固件拒绝保留未读状态的短信查询；未回退到会标为已读的命令，这不代表模组没有短信功能"
            }
            "SMS_MULTIPART_INCOMPLETE" => "长短信分段尚未完整，不转发不完整正文，请稍后再读取",
            "SMS_MULTIPART_AMBIGUOUS" => "长短信分段无法可靠关联，未合并或转发",
            "SMS_BINARY_UNSUPPORTED" | "SMS_ENCODING_UNSUPPORTED" => {
                "当前存储区含暂不支持的短信编码，未伪造正文或转发"
            }
            "SMS_SEND_TOO_LONG" => "当前仅支持单条短信，最多 70 个 UTF-16 单元；不会自动拆分计费",
            "SMS_SEND_UNKNOWN" | "AT_TIMEOUT" | "AT_WRITE_UNKNOWN" => {
                "操作结果未确认；没有自动重发，请先核对设备状态"
            }
            "SMS_DELETE_PARTIAL" => "长短信部分分段已删除，其余未确认；不要整批重发，请重新读取",
            "SMS_CHANGED" => "设备短信已变化，未删除；请重新读取并选择",
            "SMS_REFRESH_COOLDOWN" => "短信查询仍在保护间隔内，请稍后刷新；优先复用后台已有缓存",
            "SMS_READ_STATE_FAILED" => "插件短信已读记录保存失败，未报告保存成功",
            "SMS_READ_STATE_FULL" => "插件短信已读记录已满，可将旧记录标为未读后重试",
            "SMS_READ_STATE_INVALID" => "仅能修改当前收件短信的插件已读状态",
            "SMS_BUSY" | "AT_BUSY" | "LOCK_BUSY" | "SYSTEM_BUSY" => {
                "模组正在执行其他查询，请稍后再试"
            }
            "PUSHPLUS_TOKEN_INVALID" => "请填写有效的 PushPlus Token",
            "PUSHPLUS_TLS_NOT_BUILT" => "当前构建未包含 PushPlus HTTPS 功能",
            "SMS_FORWARD_HISTORY_FULL" => {
                "转发去重记录已满，已停止新增转发；请检查记录，勿直接清空以免重复推送"
            }
            "SMS_FORWARD_JOURNAL_FAILED" => "转发去重记录未能持久保存，未发送到 PushPlus",
            "FEATURE_NOT_READY" => "当前运行模式尚未接入此功能",
            "SERVICE_STOPPING" => "网络面板正在停止，未接受新操作；请等待服务重新启动",
            "WIFI_PENDING_CHANGES" => {
                "设备有其他未保存的网络设置，请先在后台保存或撤销后再编辑 WiFi"
            }
            "WIFI_NETWORK_CHANGED" => {
                "WiFi 已保存，但网络配置同时发生变化，未重新加载；请在后台核对"
            }
            "WIFI_RELOAD_UNKNOWN" => {
                "WiFi 配置已保存，重新加载结果未确认；请检查连接，不要重复提交"
            }
            "WIFI_READBACK_MISMATCH" => "WiFi 保存结果与回读不一致，请在设备后台核对",
            "HOST_CHANGED" => "设备设置已变化，本次操作未执行；请刷新后重新确认",
            _ => self.code,
        }
    }
}
pub type Result<T> = std::result::Result<T, PanelError>;
