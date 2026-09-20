//! M4 路由探针（live，#[ignore]）：Campus 判定 + Direct 路径回归。
//!
//! 校外场景无法离线模拟（需真实校外网络），本轮探针只覆盖校内侧三条事实：
//! 1. `net_zone::detect()` 在校园网内判 `Campus`；
//! 2. `route(BERSERKER_BASE, Campus, _) == Direct`（有无 WebVPN 会话均然）；
//! 3. Direct 路径可达性：裸 TCP 直连 `10.3.100.110:80` 发匿名目录请求，
//!    返回 200 且含 `feeitemList`（直连的本质就是内网 TCP 可达，用 std 零依赖实现）。
//!
//! OffCampus 的包装链路（WebVPN 登录 / 逐跳包装 / sso_token_via）依赖校外真实网络，
//! 留给用户真机验收；包装逻辑本身已由离线单测（route 决策表 + wrap golden + 桩 302 链）覆盖。
//!
//! 验收命令（校内运行）：
//! `cargo test -p campus-hub --test webvpn_route_probe_live -- --ignored --nocapture`

#![cfg(test)]

use campus_hub_lib::infra::net_zone::{self, NetZone};
use campus_synjones::routing::{route, NetZone as RouteZone, RouteDecision};
use campus_synjones::BERSERKER_BASE;
use std::io::{Read, Write};
use std::net::TcpStream;

#[test]
#[ignore = "live：需在校园网环境，仅主智能体验收时 -- --ignored 运行"]
fn campus_zone_routes_direct_and_reaches_feeitem() {
    // 1. 校内判定（netsh/UDP connect 只读，无副作用）
    let zone = net_zone::detect();
    println!("[probe] net_zone::detect() = {zone:?}");
    assert_eq!(zone, NetZone::Campus, "本探针须在校园网内运行");

    // 2. 路由决策：内网 base 在 Campus 下恒 Direct
    for has_vpn in [false, true] {
        let d = route(BERSERKER_BASE, RouteZone::Campus, has_vpn);
        assert!(matches!(d, RouteDecision::Direct), "Campus 必须直连，实际 {d:?}");
    }

    // 3. Direct 可达性：std TCP 直连匿名目录（Host 头给 IP，明文 http）
    let mut stream = TcpStream::connect("10.3.100.110:80").expect("内网 10.3.100.110:80 直连失败");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .ok();
    stream
        .write_all(
            b"GET /charge/feeitem?synAccessSource=app HTTP/1.1\r\n\
              Host: 10.3.100.110\r\n\
              Accept: application/json, text/plain, */*\r\n\
              Connection: close\r\n\
              \r\n",
        )
        .expect("写请求失败");
    let mut buf = String::new();
    stream.read_to_string(&mut buf).expect("读响应失败");
    let status_ok = buf.starts_with("HTTP/1.1 200") || buf.starts_with("HTTP/1.0 200");
    assert!(status_ok, "匿名目录应 200：{}", &buf[..buf.len().min(200)]);
    assert!(buf.contains("feeitemList"), "响应应含 feeitemList");
    println!("[probe] direct feeitem OK（{} 字节）", buf.len());
}
