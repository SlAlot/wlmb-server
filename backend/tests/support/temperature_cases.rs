// 根据公开命令格式构造的脱敏温度样例，不冒充逐型号真实设备采集。
pub fn reply(model: &str, command: &str) -> (&'static str, f64) {
    match command {
        "AT^CHIPTEMP?" if model == "mt5700m-cn" => (
            "^CHIPTEMP: 100,100,100,100,100,420,100,100,420,430,100,100\r\nOK\r\n",
            43.,
        ),
        "AT^CHIPTEMP?" if wlmb_server::catalog::platform(model) == "unisoc" => {
            ("^CHIPTEMP: 1,1,1,1,1,42\r\nOK\r\n", 42.)
        }
        "AT^CHIPTEMP?" => ("^CHIPTEMP: 1,1,1,1,1,420\r\nOK\r\n", 42.),
        "AT+QTEMP" if wlmb_server::catalog::platform(model) == "unisoc" => (
            "+QTEMP: \"soc-thermal\",\"50\"\r\n+QTEMP: \"pa-thermal\",\"49\"\r\n+QTEMP: \"board-thermal\",\"49\"\r\nOK\r\n",
            50.,
        ),
        "AT+QTEMP" => (
            "+QTEMP: \"cpuss-usr\",110\n+QTEMP: \"mdm-usr\",44\n+QTEMP: \"pa-therm1-usr\",115\nOK\n",
            44.,
        ),
        "AT+MTSM=1" | "AT+MTSM=1,6" => ("+MTSM: 42\r\nOK\r\n", 42.),
        "AT+GTLADC" => ("+GTLADC: cpu 45000\r\nOK\r\n", 45.),
        "AT+GTSENRDTEMP=1" => ("+GTSENRDTEMP: 1,43000\r\nOK\r\n", 43.),
        "AT+TEMP" if wlmb_server::catalog::platform(model) == "unisoc" => {
            ("+TEMP: \"soc-thmzone\",\"45000\"\r\nOK\r\n", 45.)
        }
        "AT+TEMP" => ("+TEMP: \"cpu0-0-usr\",\"44\"\r\nOK\r\n", 44.),
        "AT+TEMP?" | "AT^TEMP?" => ("TSENS: 33C\r\nOK\r\n", 33.),
        "AT!GSTATUS?" => ("!GSTATUS:\nCurrent Time: 2863 Temperature: 31\nOK\n", 31.),
        "AT!PCTEMP?" => ("Temperature: 35.25 C\r\nOK\r\n", 35.25),
        "AT+CPMUTEMP" => ("+CPMUTEMP: 46\r\nOK\r\n", 46.),
        "AT#TEMPSENS=2" => ("#TEMPSENS: TSENS,47\r\nOK\r\n", 47.),
        _ => panic!("缺少温度回归样例：{model} {command}"),
    }
}
