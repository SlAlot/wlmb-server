// 源于公开接口字段的脱敏结构样例；保留GW及SA／NSA分项，不当作实机已验证。
pub fn current(lte_only: bool, locked: bool) -> String {
    let mut raw="!BAND: 00, All bands\n0 - GW: 0000000000000001\n1 - LTE: 0000000000000005 0000000000000000\n".to_owned();
    if !lte_only {
        raw.push_str("3 - NRNSA: 0000000000000000 0000000000002000\n");
        raw.push_str(if locked {
            "4 - NRSA: 0000000000000000 0000000000002000\n"
        } else {
            "4 - NRSA: 0000010000000000 0000000000002000\n"
        });
    }
    raw.push_str("OK\n");
    raw
}
pub fn available(lte_only: bool) -> String {
    let mut raw="!BAND:\nAvailable:\n0 - GW:\n0000000000000001 - GSM850\n1 - LTE:\n0000000000000001 - B1\n0000000000000004 - B3\n0000000000000010 - B5\n".to_owned();
    if !lte_only {
        raw.push_str("3 - NRNSA:\n0000010000000000 - n41\n0000000000002000 - n78\n4 - NRSA:\n0000010000000000 - n41\n0000000000002000 - n78\n0000000000004000 - n79\n");
    }
    raw.push_str("OK\n");
    raw
}
