//! RSA golden 对拍测试（Task 6 Step 1）。
//!
//! golden 值已由计划评审员用独立 BigInt 实现对拍 `docs/cas-recon/rsa30.js` 实测固化，
//! 逐字使用，不得改动。

use campus_auth::rsa::{cas_token_header, rsa_encrypt_hex};
use campus_auth::CampusAuthError;

const GOLDEN: &[(&str, &str)] = &[
    ("lyasp1726500000000", "7678b328a692e0745cd039bb6d0cba0af71f54111ec105b11965fc7430ee46abedb4457bdaf1f79a5c0b0ca4a67af838c5ef1152cfe63baf71021030ea8790575e97847312f2815281dca867461d693e626c2969309ce465b45dc67437f73f3f31944b66dca4a02377082a1bd8678f2435433cac8bc2bcb8bb4cc34a70fc2826"),
    ("Test@Password123",   "7fd7ab7bc2860811f82876860a634f4cf1097ed9e6d0098ac680eb875b1c0fa965758a69bc240a402a982d4293a675295cf4a4df659374600f009052f6ff3d97fb905a9b506efd82c131fb016ec6f9b6639d0419fa615ff53e55e984a22ffc4612124b9ce813b16add489e7b0ef500f40915774e145ee72dca83776519618228"),
];

#[test]
fn golden_pairs_match_js_reference() {
    for (plaintext, expected_hex) in GOLDEN {
        let got = rsa_encrypt_hex(plaintext).expect("RSA 加密应成功");
        assert_eq!(&got, expected_hex, "明文 {plaintext:?} 的密文与 golden 不符");
    }
}

#[test]
fn non_ascii_rejected() {
    // P2-11：JS charCodeAt 语义对非 ASCII 不等价，必须显式拒绝
    let err = rsa_encrypt_hex("密码123").expect_err("非 ASCII 输入应返回 Err");
    assert!(
        matches!(err, CampusAuthError::Rsa(_)),
        "非 ASCII 拒绝应归为 Rsa 错误，实际 {err:?}"
    );
}

#[test]
fn token_header_matches_golden_timestamp() {
    // token 头 = RSA("lyasp" + 毫秒时间戳)；golden 第一对的明文正是 1726500000000
    let now_ms: u64 = 1726500000000;
    let token = cas_token_header(now_ms).expect("token 加密应成功");
    assert_eq!(token, GOLDEN[0].1);
    // 与直接加密同串结果一致
    assert_eq!(
        token,
        rsa_encrypt_hex("lyasp1726500000000").expect("加密应成功")
    );
}
