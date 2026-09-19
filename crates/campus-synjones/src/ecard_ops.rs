//! 一卡通安全键盘（批 1：只做**取键盘 + 内存缓存**；写操作在批 4 接入）。
//!
//! # 协议事实（2026-09-19 live，探针 `tests/ecard_features_probe_live.rs` D3/D4）
//!
//! `GET /berserker-secure/keyboard?type=Number|Standard`（信封 [`Envelope::Berserker`]，
//! 响应 `{code, data, msg, success}`）：
//!
//! - `type=Number`：`data.numberKeyboard`（10 字符乱序串）+ `numberKeyboardImage`（10 张图）+ `uuid`；
//! - `type=Standard`：`numberKeyboard` / `upperLetterKeyboard` / `lowerLetterKeyboard` /
//!   `symbolKeyboard` 四组，各配同名 `<组>Image` 图数组，共享同一个 `uuid`。
//!
//! 键位语义：`<组>Keyboard` 串的第 i 个字符就是第 i 张图上的字符——用户点击位置 i，
//! 明文即 `keys[i]`（批 4 拼 `pwd = "1$1$" + 明文 + "$1$" + uuid` 用）。
//!
//! # 缓存红线（契约 §2.4，与电费 `passwordMap` 同构）
//!
//! 1. 真实 `uuid → 键位串` 映射**只在内存**：TTL 300 秒、至多 8 把、不落盘、不打日志、
//!    不进错误文案；`SecurePad`/`PadSecret` 的 `Debug` 手写打码。
//! 2. 前端只拿 [`SecurePad::pad_id`]（**本进程随机生成**，真实 uuid 绝不外泄）+ `keys`（渲染）+ `images`。
//! 3. [`take_pad`] 取走即删（消耗语义），批 4 拼 `pwd` 时从这里拿真实 uuid 与键位串。
//!
//! 键位组顺序固定为 **数字 → 大写 → 小写 → 符号**（前端按下标渲染，两边必须同序）。

use crate::client::{Envelope, SynjonesClient};
use crate::CampusSynjonesError;
use serde::Serialize;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 安全键盘端点。
pub const EP_KEYBOARD: &str = "/berserker-secure/keyboard";

/// 键盘映射的存活时长（契约 §2.4：5 分钟）。
pub const PAD_TTL: Duration = Duration::from_secs(300);
/// 最多同时缓存 8 把（超出淘汰最旧）。
pub const MAX_PADS: usize = 8;

/// 键盘种类（命令参数 `"number"` / `"standard"`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardKind {
    /// 数字键盘（10 键，密码/验证码输入）。
    Number,
    /// 全键盘（数字/大小写/符号四组）。
    Standard,
}

impl KeyboardKind {
    /// 服务端 `type` 参数值（实测大小写敏感）。
    fn type_param(self) -> &'static str {
        match self {
            KeyboardKind::Number => "Number",
            KeyboardKind::Standard => "Standard",
        }
    }

    /// 从命令参数解析（其余值一律拒绝，不猜）。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "number" => Some(KeyboardKind::Number),
            "standard" => Some(KeyboardKind::Standard),
            _ => None,
        }
    }
}

/// 交给前端的键盘：`pad_id` 是本进程随机 id（真实 uuid 在缓存里），`keys`/`images`
/// 供渲染（下标一一对应）。
#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurePad {
    /// 本进程随机 id（前端提交操作时原样带回）。
    pub pad_id: String,
    /// 键位显示字符（数字键盘 10 个；全键盘四组拼接）。
    pub keys: Vec<String>,
    /// 键位图片（base64 PNG，与 `keys` 同长；服务端未下发时为空表）。
    pub images: Vec<String>,
}

/// `Debug` 打码：键位字符与 pad_id 都不进日志/panic 消息。
impl std::fmt::Debug for SecurePad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecurePad {{ pad_id: ***, keys: <{} 键，已打码> }}", self.keys.len())
    }
}

/// 批 4 拼 `pwd` 用的秘密材料：真实服务端 uuid + 键位串（**绝不**回前端、不落盘、不打日志）。
#[derive(Clone, PartialEq)]
pub struct PadSecret {
    /// 服务端下发的键盘 uuid。
    pub uuid: String,
    /// 键位字符（下标即用户点击位置）。
    pub keys: Vec<String>,
}

impl std::fmt::Debug for PadSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PadSecret {{ uuid: ***, keys: <{} 键，已打码> }}", self.keys.len())
    }
}

struct PadEntry {
    pad_id: String,
    uuid: String,
    keys: Vec<String>,
    created: Instant,
}

static PADS: OnceLock<Mutex<Vec<PadEntry>>> = OnceLock::new();

fn pads() -> &'static Mutex<Vec<PadEntry>> {
    PADS.get_or_init(|| Mutex::new(Vec::new()))
}

/// pad_id 计数器（与纳秒时钟混合，进程内唯一且不可猜）。
fn next_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn new_pad_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // ponytail: 时间+计数器混排即可满足「进程内唯一、不可枚举」；不引 rand 依赖
    format!("pad{:016x}{:04x}", nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15), next_seq())
}

/// 入缓存：清过期 → 超量淘汰最旧 → 追加。返回分配的 pad_id。
fn store_pad(uuid: String, keys: Vec<String>) -> String {
    let pad_id = new_pad_id();
    let now = Instant::now();
    if let Ok(mut list) = pads().lock() {
        list.retain(|e| now.duration_since(e.created) < PAD_TTL);
        while list.len() >= MAX_PADS {
            list.remove(0);
        }
        list.push(PadEntry { pad_id: pad_id.clone(), uuid, keys, created: now });
    }
    pad_id
}

/// 取走即删（消耗语义）。过期 / 不存在 → None。
pub fn take_pad(pad_id: &str) -> Option<PadSecret> {
    let mut list = pads().lock().ok()?;
    let now = Instant::now();
    let pos = list.iter().position(|e| e.pad_id == pad_id)?;
    let e = list.remove(pos);
    if now.duration_since(e.created) >= PAD_TTL {
        return None;
    }
    Some(PadSecret { uuid: e.uuid, keys: e.keys })
}

// ---------------- 解析与取数 ----------------

/// 键位组的字段名（顺序即对外键位顺序：数字 → 大写 → 小写 → 符号）。
const KEY_GROUPS: [(&str, &str); 4] = [
    ("numberKeyboard", "numberKeyboardImage"),
    ("upperLetterKeyboard", "upperLetterKeyboardImage"),
    ("lowerLetterKeyboard", "lowerLetterKeyboardImage"),
    ("symbolKeyboard", "symbolKeyboardImage"),
];

/// 解析键盘 `data`：`uuid` + 全部键位组的字符串与图片（按 [`KEY_GROUPS`] 顺序拼接；
/// Number 型只有 numberKeyboard 一组，自然兼容）。键位串为空或无 uuid → None。
pub fn parse_keyboard(data: &Value) -> Option<(String, Vec<String>, Vec<String>)> {
    let uuid = crate::ecard::text_of(Some(data.get("uuid")?));
    if uuid.is_empty() {
        return None;
    }
    let mut keys: Vec<String> = Vec::new();
    let mut images: Vec<String> = Vec::new();
    for (k_field, img_field) in KEY_GROUPS {
        let Some(raw) = data.get(k_field) else { continue };
        // 实测形态：键位是「N 个字符的字符串」，图片是与键数等长的数组；数组形态一并容忍（类型漂移先例）
        let group_keys: Vec<String> = match raw {
            Value::String(s) => s.chars().map(|c| c.to_string()).collect(),
            Value::Array(a) => a
                .iter()
                .map(|it| crate::ecard::text_of(Some(it)))
                .filter(|s| !s.is_empty())
                .collect(),
            _ => continue,
        };
        if group_keys.is_empty() {
            continue;
        }
        keys.extend(group_keys);
        if let Some(imgs) = data.get(img_field).and_then(Value::as_array) {
            images.extend(imgs.iter().map(|it| crate::ecard::text_of(Some(it))));
        }
    }
    (!keys.is_empty()).then_some((uuid, keys, images))
}

/// 取一把安全键盘并写入进程级缓存（返回给前端的只有 pad_id + 键位 + 图片）。
pub async fn fetch_secure_keyboard(
    client: &SynjonesClient,
    kind: KeyboardKind,
) -> Result<SecurePad, CampusSynjonesError> {
    let v = client
        .get(EP_KEYBOARD, &[("type", kind.type_param())], Envelope::Berserker)
        .await?;
    let (uuid, keys, images) = parse_keyboard(&v["data"]).ok_or_else(|| {
        CampusSynjonesError::Parse("安全键盘响应缺少 uuid 或键位数据".to_string())
    })?;
    let pad_id = store_pad(uuid, keys.clone());
    Ok(SecurePad { pad_id, keys, images })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// **实测形态**（D3/D4）：键位是字符串、图片数组等长；Number 只有一组。
    #[test]
    fn parse_keyboard_reads_live_shapes() {
        let number = json!({
            "numberKeyboard": "X!-/n%~_Bv",
            "numberKeyboardImage": ["i0", "i1", "i2", "i3", "i4", "i5", "i6", "i7", "i8", "i9"],
            "password": "x",
            "uuid": "0123456789abcdef0123456789abcdef"
        });
        let (uuid, keys, images) = parse_keyboard(&number).expect("Number 键盘应可解析");
        assert_eq!(uuid, "0123456789abcdef0123456789abcdef");
        assert_eq!(keys.len(), 10, "实测恒 10 键");
        assert_eq!(keys[0], "X");
        assert_eq!(keys[9], "v");
        assert_eq!(images.len(), 10, "图片与键位一一对应");

        // Standard：四组按固定顺序拼接（数字→大写→小写→符号）
        let standard = json!({
            "numberKeyboard": "._:U=uZlMV",
            "upperLetterKeyboard": "xF]8<0hARe6",
            "lowerLetterKeyboard": "'$m9Q\\{@;p/7Ldr",
            "symbolKeyboard": "O&g(aYc%qo!>3wb`s^2Df",
            "uuid": "6fe409cf-7fa4-44a1-888d-67a10f2eda49"
        });
        let (_, keys, images) = parse_keyboard(&standard).expect("Standard 键盘应可解析");
        assert_eq!(keys.len(), 10 + 11 + 15 + 21);
        assert_eq!(keys[0], ".", "第一组必须是数字组（前端按下标渲染，顺序不可变）");
        assert_eq!(keys[10], "x");
        assert!(images.is_empty(), "未下发图片 → 空表（渲染层自行降级）");
    }

    /// 容错：缺 uuid / 键位空 / 键位非字符串数组 → None（宁可取不到键盘，不给残缺键盘）。
    #[test]
    fn parse_keyboard_rejects_broken_payloads() {
        assert!(parse_keyboard(&json!({})).is_none());
        assert!(parse_keyboard(&json!({"uuid": ""})).is_none());
        assert!(parse_keyboard(&json!({"uuid": "u", "numberKeyboard": ""})).is_none());
        assert!(parse_keyboard(&json!({"uuid": "u", "numberKeyboard": 123})).is_none());
        // 数组形态（类型漂移先例）也吃得下
        let arr = parse_keyboard(&json!({"uuid": "u", "numberKeyboard": ["1", "2"]}));
        assert_eq!(arr.unwrap().1, vec!["1", "2"]);
    }

    /// 缓存生命周期（**合并为单测**：共享 static，拆开并行跑会互相干扰容量断言）：
    /// take 取走即删 → 过期作废 → 超量淘汰最旧。
    #[test]
    fn pad_cache_lifecycle() {
        // 1) 取走即删（消耗语义）；pad_id 与真实 uuid 不相同
        let pad_id = store_pad("uuid-real-1".to_string(), vec!["1".into(), "2".into()]);
        assert!(pad_id.starts_with("pad"));
        assert_ne!(pad_id, "uuid-real-1", "真实 uuid 绝不当 pad_id 外泄");
        let secret = take_pad(&pad_id).expect("首次 take 应取到");
        assert_eq!(secret.uuid, "uuid-real-1");
        assert_eq!(secret.keys, vec!["1".to_string(), "2".to_string()]);
        assert!(take_pad(&pad_id).is_none(), "取走即删（重复 take 拿不到）");
        assert!(take_pad("不存在的id").is_none());

        // 2) 过期：TTL 之后的条目取不到（回拨 created，不真等 5 分钟）
        let expired_id = store_pad("uuid-exp".to_string(), vec!["1".into()]);
        if let Ok(mut list) = pads().lock() {
            if let Some(e) = list.iter_mut().find(|e| e.pad_id == expired_id) {
                e.created = Instant::now()
                    .checked_sub(PAD_TTL + Duration::from_secs(1))
                    .expect("回拨时长应合法");
            }
        }
        assert!(take_pad(&expired_id).is_none(), "过期的键盘映射必须作废");

        // 3) 容量上限：第 9 把进来时最旧的一把被淘汰
        let ids: Vec<String> = (0..MAX_PADS)
            .map(|i| store_pad(format!("u{i}"), vec!["k".into()]))
            .collect();
        let newest = store_pad("u-new".to_string(), vec!["k".into()]);
        assert!(take_pad(&newest).is_some(), "新键盘必须可用");
        assert!(take_pad(&ids[0]).is_none(), "超出容量最旧被淘汰");
        assert!(take_pad(&ids[1]).is_some(), "次旧的仍在");
    }

    /// 红线：`SecurePad` / `PadSecret` 的 Debug 必须打码。
    #[test]
    fn debug_masks_secret_material() {
        let pad = SecurePad { pad_id: "pad-secret".into(), keys: vec!["9".into()], images: vec![] };
        let dbg = format!("{pad:?}");
        assert!(!dbg.contains("pad-secret"));
        assert!(!dbg.contains('9'));
        assert!(dbg.contains("1 键"));

        let secret = PadSecret { uuid: "uuid-secret".into(), keys: vec!["8".into()] };
        let dbg = format!("{secret:?}");
        assert!(!dbg.contains("uuid-secret"));
        assert!(!dbg.contains('8'));
    }

    /// KeyboardKind：参数解析与服务端 type 值（大小写敏感）。
    #[test]
    fn keyboard_kind_parses_strictly() {
        assert_eq!(KeyboardKind::parse("number"), Some(KeyboardKind::Number));
        assert_eq!(KeyboardKind::parse(" standard "), Some(KeyboardKind::Standard));
        assert_eq!(KeyboardKind::parse("Number"), None, "命令参数只认小写");
        assert_eq!(KeyboardKind::parse(""), None);
        assert_eq!(KeyboardKind::Number.type_param(), "Number");
        assert_eq!(KeyboardKind::Standard.type_param(), "Standard");
    }
}
