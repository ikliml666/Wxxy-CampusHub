//! 校园网归属判定（NetZone）：零新依赖的 std 探测，M4 路由接入的分流依据。
//!
//! # 判据设计（主判据 → 辅判据两层，逐级兜底）
//!
//! - **主判据（UDP connect 选路）**：`UdpSocket::bind("0.0.0.0:0")` 后 `connect`
//!   到校内固定 IP [`PROBE_TARGET`]——UDP connect **不实际发包**，仅让内核做一次
//!   路由选路（无 DNS、无网络往返、微秒级），随后 `local_addr()` 即返回「去往目标
//!   时会使用的本机源 IP」（std 技巧，移植自 Wxxy-CampusLogin 的网关可达性思路但
//!   不依赖 netsh）。源 IP 落在 10.0.0.0/8（[`classify`]）→ [`NetZone::Campus`]。
//!   判据选 /8 而非 CampusLogin 的 /18：/18 是对网关 10.2.x.x 细分网段的收紧，而
//!   10/8 为 RFC1918 私网，校外网络不可能给主机分配 10/8 源地址，放宽到 /8 不引入
//!   误判面，且能覆盖宿舍区/办公区不同 10.x 段。
//! - **辅判据 A（接口地址枚举，覆盖代理 TUN 抢路由）**：本机实测（2026-09-20），
//!   代理工具的 TUN 虚拟网卡（Meta Tunnel，198.18.0.0/15 网段）默认路由跃点数为
//!   0，会抢占 UDP 选路——选路源 IP 落在 TUN 网段而非真实校园网地址，主判据因此
//!   误判。兜底：`netsh interface ip show addresses` 枚举全部接口的 IPv4，任一接口
//!   在 10/8 → [`NetZone::Campus`]（10/8 不可能出现在家用/VMware/TUN 网段）。
//! - **辅判据 B（netsh SSID，仅主判据失败且 A 无 10/8 时）**：当前 SSID 含 "wxxy"
//!   （i-wxxy / iwxxy-2 / iwxxy-3，CampusLogin 同源名单）→ 升级
//!   [`NetZone::Campus`]。仅单向佐证：SSID 不匹配不降级（有线时 netsh wlan 查不到）。
//! - 主判据给出明确非 10/8 源 IP 且 A 无 10/8 → [`NetZone::OffCampus`]；其余情形
//!   （connect 失败 + 佐证不可用）→ [`NetZone::Unknown`]。
//!
//! # 缓存与阻塞
//!
//! 判定结果缓存 60s TTL（进程级 `static`，`Mutex::new(None)` 为 const 可直接初始
//! 化，不引 lazy_static）；netsh 查询单独缓存同一 TTL。netsh 为同步子进程调用
//! （CREATE_NO_WINDOW 防黑窗），调用方若在 async 上下文应放 `spawn_blocking`。

use std::net::{IpAddr, Ipv4Addr};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 校园网归属三态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetZone {
    /// 在校园网内（选路源 IP 10/8 / 接口地址 10/8 / SSID 匹配）。
    Campus,
    /// 确定在校外（选路源 IP 明确非 10/8，且所有接口无 10/8）。
    OffCampus,
    /// 无法判定（connect 失败且各辅判据不可用）。
    Unknown,
}

/// 主判据探测目标（校内固定 IP:端口；选 10.3.100.110:80 计费服务器——M4 探针目标
/// 同源，选路结果与后续 WebVPN 代理请求一致）。字面 IP，无 DNS。
const PROBE_TARGET: &str = "10.3.100.110:80";

/// 探测整体超时兜底（任务规格 300ms）：字面 IP 的 UDP connect 本身不阻塞，此超时
/// 只防奇异平台行为，超时按无结果处理。
const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

/// 结果缓存 TTL（CampusLogin subnet.rs:43-60 同款 60s）。
const CACHE_TTL: Duration = Duration::from_secs(60);

/// 判定结果缓存：(写入时刻, 判定结果)。
static ZONE_CACHE: Mutex<Option<(Instant, NetZone)>> = Mutex::new(None);

/// netsh 查询缓存条目：SSID 与接口枚举共用形态，None 结果不缓存以便下次重试。
type NetshCache = Mutex<Option<(Instant, Option<String>)>>;

/// netsh SSID 查询缓存。
static SSID_CACHE: Mutex<Option<(Instant, Option<String>)>> = Mutex::new(None);

/// netsh 接口 IPv4 列表缓存（None = 查询失败；空串 = 查询成功但无 IPv4）。
static IFACE_CACHE: Mutex<Option<(Instant, Option<String>)>> = Mutex::new(None);

/// 校园网 SSID 特征（小写匹配）：i-wxxy / iwxxy-2 / iwxxy-3（CampusLogin 配置同源）。
const SSID_MARKER: &str = "wxxy";

/// 判定当前网络归属（带 60s TTL 缓存；同步阻塞，async 侧请放 spawn_blocking）。
pub fn detect() -> NetZone {
    if let Some(zone) = cache_get(&ZONE_CACHE) {
        return zone;
    }
    let zone = detect_uncached();
    cache_put(&ZONE_CACHE, zone);
    zone
}

/// 逐级判定（无缓存）。
fn detect_uncached() -> NetZone {
    let routed = probe_source_ip();
    if routed.is_some_and(|ip| classify(ip) == NetZone::Campus) {
        return NetZone::Campus;
    }
    // 辅判据 A：任一接口 IPv4 ∈ 10/8 → 校内（覆盖代理 TUN 抢占默认路由、
    // 以及 connect 失败但接口已带校园网地址的场景）
    if interface_ipv4s().iter().any(|ip| is_private_ten(*ip)) {
        return NetZone::Campus;
    }
    // 选路有明确非 10/8 源 + 接口无 10/8 → 确定校外
    if routed.is_some() {
        return NetZone::OffCampus;
    }
    // 主判据失败 + 接口无 10/8 → SSID 单向佐证（不匹配不降级，保持 Unknown）
    match ssid_query() {
        Some(ssid) if ssid_is_campus(&ssid) => NetZone::Campus,
        _ => NetZone::Unknown,
    }
}

/// 源 IP → 归属（纯函数，独立可测）。
///
/// 10.0.0.0/8 → Campus；其余 IPv4 与一切 IPv6 → OffCampus（主判据目标是 IPv4，
/// 源必为 IPv4，V6 分支仅为完备）。
pub fn classify(ip: IpAddr) -> NetZone {
    match ip {
        IpAddr::V4(v4) if is_private_ten(v4) => NetZone::Campus,
        _ => NetZone::OffCampus,
    }
}

/// 10.0.0.0/8 判定（首字节为 10）。
fn is_private_ten(ip: Ipv4Addr) -> bool {
    ip.octets()[0] == 10
}

/// UDP connect 选路探测（返回去往 [`PROBE_TARGET`] 的本机源 IP；失败/超时 → None）。
fn probe_source_ip() -> Option<IpAddr> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = (|| {
            let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
            // 不实际发包：仅设置默认目标并触发内核路由选路
            sock.connect(PROBE_TARGET).ok()?;
            sock.local_addr().ok().map(|a| a.ip())
        })();
        let _ = tx.send(r);
    });
    rx.recv_timeout(PROBE_TIMEOUT).ok().flatten()
}

/// 全部接口的 IPv4 地址（netsh `interface ip show addresses`，60s TTL 缓存；
/// 查询失败返回空）。
fn interface_ipv4s() -> Vec<Ipv4Addr> {
    let cached = netsh_cached(&IFACE_CACHE, || {
        let out = spawn_netsh()
            .args(["interface", "ip", "show", "addresses"])
            .output()
            .ok()?;
        out.status.success().then(|| {
            // netsh 输出在中文 Windows 为 GBK：GBK 兼容 ASCII，IPv4 点分四段无损
            String::from_utf8_lossy(&out.stdout).into_owned()
        })
    });
    cached
        .as_deref()
        .map(parse_ipv4s_from_netsh)
        .unwrap_or_default()
}

/// 查当前 WiFi SSID（netsh `wlan show interfaces`，60s TTL 缓存；失败/未连 None）。
fn ssid_query() -> Option<String> {
    let cached = netsh_cached(&SSID_CACHE, || {
        let out = spawn_netsh().args(["wlan", "show", "interfaces"]).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    })?;
    parse_ssid_from_netsh(&cached)
}

/// netsh 查询统一缓存壳：命中未过期缓存直接返回；否则执行查询并缓存 Some 结果
/// （None 不缓存，下次重试；CampusLogin subnet.rs:52-77 同款语义）。
fn netsh_cached(
    cell: &Mutex<Option<(Instant, Option<String>)>>,
    query: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if let Some((ts, val)) = cell.lock().ok().and_then(|g| g.as_ref().cloned()) {
        if ts.elapsed() < CACHE_TTL {
            return val;
        }
    }
    let val = query();
    if let Ok(mut guard) = cell.lock() {
        *guard = Some((Instant::now(), val.clone()));
    }
    val
}

/// netsh 命令（Windows 下带 CREATE_NO_WINDOW，GUI 应用 spawn 子进程不闪黑窗；
/// std 自带 CommandExt，零新依赖）。
#[cfg(windows)]
fn spawn_netsh() -> Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new("netsh");
    // CREATE_NO_WINDOW = 0x08000000
    cmd.creation_flags(0x0800_0000);
    cmd
}

/// 非 Windows 平台直接返回（本应用实际只发 Windows 包，分支仅为编译可移植）。
#[cfg(not(windows))]
fn spawn_netsh() -> Command {
    Command::new("netsh")
}

/// 从 `netsh wlan show interfaces` 输出解析当前 SSID（纯函数供离线单测）。
///
/// 规则（CampusLogin subnet.rs:59-77 同款）：取以 `SSID` 开头且非 `BSSID` 的行的
/// 第一个冒号后内容；空串 / 含「不在」「disconnected」「not connected」视为未连接。
fn parse_ssid_from_netsh(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let t = line.trim();
        if t.starts_with("SSID") && !t.starts_with("BSSID") {
            let Some(colon) = t.find(':') else { continue };
            let ssid = t[colon + 1..].trim();
            if !ssid.is_empty()
                && !ssid.contains("不在")
                && !ssid.contains("disconnected")
                && !ssid.contains("not connected")
            {
                return Some(ssid.to_string());
            }
        }
    }
    None
}

/// 从 `netsh interface ip show addresses` 输出提取点分四段 IPv4（纯函数供离线
/// 单测）。**不做行关键字过滤**：中文输出 IP 行是「IP 地址:」而非「IPv4 地址:」
/// （2026-09-20 本机实测），英文/中文/旧版格式各异；逐行提取首个点分四段——
/// 子网掩码（255.x）/网关（校内网关同为 10.x）/DNS 行被一并收进也不影响
/// 「任一地址 ∈ 10/8」的判定语义（家用路由不会用 10/8，误收只会强化校内证据）；
/// IPv6 行（fe80::…）无点分四段自然跳过。
fn parse_ipv4s_from_netsh(stdout: &str) -> Vec<Ipv4Addr> {
    stdout.lines().filter_map(first_ipv4_in).collect()
}

/// 行内提取首个点分四段 IPv4（手写扫描，零 regex 依赖）。
fn first_ipv4_in(s: &str) -> Option<Ipv4Addr> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let mut octets = [0u8; 4];
            let mut pos = i;
            let mut ok = true;
            for (k, o) in octets.iter_mut().enumerate() {
                if k > 0 {
                    if pos < b.len() && b[pos] == b'.' {
                        pos += 1;
                    } else {
                        ok = false;
                        break;
                    }
                }
                let start = pos;
                while pos < b.len() && b[pos].is_ascii_digit() {
                    pos += 1;
                }
                // 数字段 1~3 位且 ≤255（u8 解析即含上界校验）
                if pos - start == 0 || pos - start > 3 {
                    ok = false;
                    break;
                }
                match s[start..pos].parse::<u8>() {
                    Ok(v) => *o = v,
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                return Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]));
            }
        }
        i += 1;
    }
    None
}

/// SSID 是否为校园网（纯函数供离线单测，ASCII 小写包含匹配）。
pub fn ssid_is_campus(ssid: &str) -> bool {
    ssid.to_ascii_lowercase().contains(SSID_MARKER)
}

fn cache_get(cell: &Mutex<Option<(Instant, NetZone)>>) -> Option<NetZone> {
    let guard = cell.lock().ok()?;
    let (ts, zone) = guard.as_ref()?;
    (ts.elapsed() < CACHE_TTL).then(|| *zone)
}

fn cache_put(cell: &Mutex<Option<(Instant, NetZone)>>, zone: NetZone) {
    if let Ok(mut guard) = cell.lock() {
        *guard = Some((Instant::now(), zone));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- classify ----------

    #[test]
    fn campus_source_ips() {
        assert_eq!(classify("10.2.65.46".parse().unwrap()), NetZone::Campus);
        assert_eq!(classify("10.3.100.110".parse().unwrap()), NetZone::Campus);
        // 10/8 边界
        assert_eq!(classify("10.0.0.0".parse().unwrap()), NetZone::Campus);
        assert_eq!(classify("10.255.255.255".parse().unwrap()), NetZone::Campus);
    }

    #[test]
    fn off_campus_source_ips() {
        assert_eq!(classify("192.168.1.5".parse().unwrap()), NetZone::OffCampus);
        assert_eq!(classify("8.8.8.8".parse().unwrap()), NetZone::OffCampus);
        assert_eq!(classify("172.16.0.1".parse().unwrap()), NetZone::OffCampus);
        assert_eq!(classify("127.0.0.1".parse().unwrap()), NetZone::OffCampus);
        assert_eq!(classify("169.254.1.2".parse().unwrap()), NetZone::OffCampus);
        // 代理 TUN 网段（198.18.0.0/15）不在 10/8
        assert_eq!(classify("198.18.0.1".parse().unwrap()), NetZone::OffCampus);
        assert_eq!(
            classify("2001:db8::1".parse().unwrap()),
            NetZone::OffCampus
        );
    }

    // ---------- parse_ssid_from_netsh ----------

    #[test]
    fn netsh_parse_connected_ssid() {
        let out = "\r\n目前有 1 个接口:\r\n\r\n    名称: WLAN\r\n    描述: Wi-Fi 6\r\n    GUID: xxx\r\n    SSID: i-wxxy\r\n    BSSID: aa:bb:cc:dd:ee:ff\r\n    网络类型: 基础结构\r\n";
        assert_eq!(parse_ssid_from_netsh(out), Some("i-wxxy".to_string()));
    }

    #[test]
    fn netsh_parse_english_output() {
        let out = "    SSID: iwxxy-3\r\n    BSSID: aa:bb:cc:dd:ee:ff\r\n";
        assert_eq!(parse_ssid_from_netsh(out), Some("iwxxy-3".to_string()));
    }

    #[test]
    fn netsh_parse_disconnected_is_none() {
        // 中文系统断开时 SSID 值为空或有「不在」字样；英文 disconnected
        assert_eq!(parse_ssid_from_netsh("    SSID: \r\n    BSSID: x\r\n"), None);
        assert_eq!(parse_ssid_from_netsh("    SSID: 不在\r\n"), None);
        assert_eq!(parse_ssid_from_netsh("    SSID: disconnected\r\n"), None);
        assert_eq!(parse_ssid_from_netsh(""), None);
        // BSSID 行不参与（前缀 BSSID 被排除）
        assert_eq!(parse_ssid_from_netsh("    BSSID: aa:bb\r\n"), None);
    }

    // ---------- parse_ipv4s_from_netsh ----------

    #[test]
    fn iface_parse_chinese_output() {
        // 2026-09-20 本机实测形态：中文输出 IP 行为「IP 地址:」
        let out = "接口 \"以太网\" 的配置\r\n    DHCP 已启用:                          是\r\n    IP 地址:                           10.2.65.46\r\n    子网前缀:                        10.2.64.0/18 (掩码 255.255.192.0)\r\n    默认网关:                         10.2.127.254\r\n\r\n接口 \"VMware Network Adapter VMnet1\" 的配置\r\n    IP 地址:                           192.168.83.1\r\n";
        let ips = parse_ipv4s_from_netsh(out);
        assert!(ips.contains(&"10.2.65.46".parse::<Ipv4Addr>().unwrap()));
        assert!(ips.contains(&"192.168.83.1".parse::<Ipv4Addr>().unwrap()));
    }

    #[test]
    fn iface_parse_english_output() {
        let out = "Configuration for interface \"Ethernet\"\r\n    DHCP enabled:                         Yes\r\n    IP Address:                           10.2.65.46\r\n    Subnet Prefix:                        10.2.64.0/18 (mask 255.255.192.0)\r\n    Default Gateway:                      10.2.127.254\r\n";
        let ips = parse_ipv4s_from_netsh(out);
        assert!(ips.contains(&"10.2.65.46".parse::<Ipv4Addr>().unwrap()));
    }

    #[test]
    fn iface_parse_skips_ipv6_and_empty() {
        assert!(parse_ipv4s_from_netsh("").is_empty());
        // fe80::1%12 无点分四段，不产出
        let out = "    IP Address:                           fe80::1%12\r\n    InterfaceMetric:                      25\r\n";
        assert!(parse_ipv4s_from_netsh(out).is_empty());
    }

    // ---------- ssid_is_campus ----------

    #[test]
    fn ssid_marker_matches_campus_list() {
        assert!(ssid_is_campus("i-wxxy"));
        assert!(ssid_is_campus("iwxxy-2"));
        assert!(ssid_is_campus("iwxxy-3"));
        assert!(ssid_is_campus("I-WXXY")); // 大小写不敏感
        assert!(!ssid_is_campus("Home_5G"));
        assert!(!ssid_is_campus("CMCC-EDU"));
        assert!(!ssid_is_campus(""));
    }

    // ---------- detect（live） ----------

    /// 本机在校园网内时应判 Campus（UDP connect 不发包、netsh 只读，无副作用；
    /// 验收命令：`cargo test -p campus-hub --lib -- --ignored net_zone`）。
    #[test]
    #[ignore = "live：需在校园网环境，仅主智能体验收时 -- --ignored 运行"]
    fn detect_live_on_campus() {
        let zone = detect();
        println!("detect() = {zone:?}");
        assert_eq!(zone, NetZone::Campus);
    }
}
