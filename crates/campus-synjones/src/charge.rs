//! 电费业务：片区目录 + 多级级联查询（慧新E校 `/charge/*`，2026-09-19 live 实测固化）。
//!
//! # 三端点与鉴权（计划 §1.5）
//!
//! | 用途 | 方法 | 路径 | 鉴权 | 信封 |
//! |---|---|---|---|---|
//! | 片区目录 | GET | [`EP_FEEITEM`] | **匿名可读**（本服务唯一；实测四种头/参组合皆 200） | Charge |
//! | 单片区详情 | GET | [`EP_SINGLE_FEEITEM`] | 需 token（实测 401 `{"code":401,"message":"缺失令牌,鉴权失败"}`） | Charge |
//! | 级联取数 | POST form | [`EP_GET_THIRD_DATA`] | 需 token | Charge |
//!
//! # 目录过滤（实测修订派单口径）
//!
//! 匿名目录实测 **8 条**，其中 `status==1` 有 **6 条**（派单 brief 写的「启用项只有 3 条」不准确）：
//!
//! | id | name | status | impl_interface | 电费页 |
//! |---|---|---|---|---|
//! | 181 | 补卡 | 1 | 空 | 否（直接输金额） |
//! | 401 | 慧新易校一卡通充值 | 1 | 空 | 否 |
//! | 407 | 扫商户码支付 | 1 | 空 | 否 |
//! | 408 | 安科瑞电控 | **2**（停用） | `iECAcrelServiceImpl` | 是（另一种电控实现，当前停用） |
//! | 428 | 梅园1号2号3号 | **2**（停用） | `iECTsmNewServiceImpl` | 是（同名停用项） |
//! | 448 | 桃园1号-李园8号 | 1 | `iECTsmNewServiceImpl` | **是** |
//! | 449 | 李园9号-李园11号 | 1 | `iECTsmNewServiceImpl` | **是** |
//! | 450 | 梅园1号-梅园3号 | 1 | `iECTsmNewServiceImpl` | **是** |
//!
//! 故电费页口径 = `status == 1 && impl_interface 非空`（「需级联选房间的第三方缴费项」），
//! 实测得 **3 条**（448/449/450）。用该规则而非写死 id：将来学校启用 408（安科瑞电控）会自动纳入。
//!
//! # 级联规则（官方 `charge-pc` bundle 逆向 + 2026-09-19 live 实测）
//!
//! 请求体 form：`{feeitemid, type, level}` + 各级已选 `total[k-1].code = <所选 value>`（同级内平铺）。
//!
//! | 步骤 | `type` | `level` | 响应 |
//! |---|---|---|---|
//! | 首轮 | `select` | `0` | `map.total[]`（层级定义，`level` 从 1 起）+ `map.data`（第 1 级选项） |
//! | 选中第 k 级 | `select` | `k` | `map.data`（第 k+1 级选项） |
//! | 选中**末级（房间）** | **`IEC`** | `total.length` | `map.showData`（最终展示信息） |
//!
//! 实测层数 `total` = **3 级**：`campus`(校区) → `building`(楼栋) → `room`(房间)（448/449/450 一致）。
//!
//! ## ⚠️ 末级是「输入级」，不是下拉（实测，与派单 brief 的「逐级下拉」不同）
//!
//! 官方 `feeitem.flag[4]` 是 UI 形态选择器：`0`=全下拉 / `1`=只输入 / `2`=先输入再选择 /
//! **`3`=先选择再输入**。三个启用电费片区均为 `flag="1100300000"`（`flag[4]=='3'`）⇒
//! **前 2 级下拉、末级「房间」是文本框**：服务端在 `level=2` 时返回的 `map.data` 为**空**，
//! 房间号由用户输入（实测 `room=101` 直接命中，`1-101`/`101室` 等则回 `tipinfo`）。
//! 客户端据此把「无下拉选项的该级」渲染成输入框（[`FeeItem::last_level_is_input`] 供 UI 判定）。
//!
//! ## ⚠️ `showData`：键名固定为 `信息`，值是**各片区格式互不相同的自由文本**（实测）
//!
//! | 片区 | `showData` | 实测值 |
//! |---|---|---|
//! | 450 | `{"信息": …}` | `房间号：101,剩余金额：-545.70，单价：0.5400` |
//! | 448 | `{"信息": …}` | `当前余额517.05元,当前剩余电量957.50度` |
//! | 449 | `{"信息": …}` | `房间当前剩余电费625.35` |
//!
//! 即：**余额/单价不是结构化字段，而是塞在一句校方自由文本里，且三个片区格式各不相同**；
//! `map.money` / `map.iectranamt` 在这三条实测里**都不存在**（官方页面同样只把这句原文渲染成
//! 一段 `键: 值`，见 bundle 的 `showData` 循环）。故本模块**不做任何文本解构**（解析会随文案漂移
//! 而静默失效）：原样透出 `信息` 键值，由前端通用字典渲染——**禁止**硬编码「剩余金额/单价」这类键名。
//!
//! ## 其它实测事实
//!
//! - `map.data`（末级对象）含 `account`（户号）、`aid`、`roomName` 等，**含 PII，本模块一律不透出**，
//!   也不写日志（live 探针写盘前经 `redact_json` 脱敏）。
//! - 房间号非法/不存在 → `map.tipinfo`（如 `缴费系统返回数据错误child==NULL！`）且 `showData` 为空。
//! - 房间号**给空串** → HTTP 200 但 `code=500` 且 `message` 为空串（聚合接口不校验入参，直接 500）。

use crate::client::{parse_envelope, Envelope, SynjonesClient};
use crate::{CampusSynjonesError, BERSERKER_BASE, SYN_ACCESS_SOURCE};
use campus_auth::cas::CasClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// 片区目录（**匿名可读**，全服务唯一免 token 端点）。
pub const EP_FEEITEM: &str = "/charge/feeitem";
/// 单片区详情（需 token；本轮产品未消费，仅 live 取证：实测顶层 `{code,feeitem,msg,view}`，`view=="choose"`）。
pub const EP_SINGLE_FEEITEM: &str = "/charge/feeitem/singleFeeitem";
/// 级联取数（POST form，需 token）。
pub const EP_GET_THIRD_DATA: &str = "/charge/feeitem/getThirdData";

/// 单请求超时（与 `client.rs` 同值；匿名路径不走 `SynjonesClient`，故另需一份）。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 级联层数上限（防御前端传入的病态路径：每层一次请求）。
const MAX_CASCADE_LEVELS: usize = 8;

/// 官方 `flag[4] == '3'` = UI 形态「先选择再输入」：前 N-1 级下拉、末级输入框。
const FLAG_INDEX_UI_MODE: usize = 4;
const UI_MODE_SELECT_THEN_INPUT: char = '3';

/// 一个缴费片区（电费页只保留 `status==1 && impl_interface` 非空者，见模块头注）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeItem {
    /// 服务端 `feeitemid`（数字，对外统一字符串，便于放进 URL 路径）。
    pub id: String,
    pub name: String,
    /// 计费单位（实测 `"元"`）。
    pub billing_unit: String,
    /// 快捷金额档（服务端 `layout` `"10,50,100"` → `[10,50,100]`；无则空数组）。
    pub layout: Vec<u32>,
    /// 单次充值下限（元；服务端 `retain_money` 为字符串，实测 `"1"`）。
    pub retain_money: Option<f64>,
    /// 单次充值上限（元；服务端 `maxmoney`，实测 `"200"`）。
    #[serde(rename = "maxmoney")]
    pub maxmoney: Option<f64>,
    /// 备注（服务端 `remark`，实测电费项为空）。
    pub remark: String,
    /// 末级是否为**输入级**（`flag[4]=='3'`「先选择再输入」）：true 时前 N-1 级下拉、末级房间号由用户输入。
    pub last_level_is_input: bool,
}

/// 级联的一步（前端回传 / 本地存常用房间共用；计划 §2.3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomStep {
    /// 级序号（服务端 `total[].level`，从 1 起）。
    pub level: u32,
    /// 该级的**参数名**（服务端 `total[].code`，作为 form 键）。
    pub code: String,
    /// 该级的**已选值**（选项 `value`，或用户输入的房间号），作为 form 值。
    pub value: String,
    /// 该级已选**名称**（展示用，不参与请求）。
    #[serde(default)]
    pub name: String,
}

/// 一级的候选选项（`map.data` 条目 + 该级的 `code`/`level`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    /// 展示名（服务端选项 `name`）。
    pub label: String,
    /// 提交值（服务端选项 `value`）→ [`RoomStep::value`]。
    pub value: String,
    /// 该级参数名 → [`RoomStep::code`]。
    pub code: String,
    /// 该级序号 → [`RoomStep::level`]。
    pub level: u32,
}

/// 服务端层级定义（`map.total[]`）：UI 用它的 `name` 给每一级做标签（「校区」「楼栋」「房间」）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelInfo {
    pub level: u32,
    /// 该级的 form 参数名。
    pub code: String,
    /// 该级中文名（实测 校区/楼栋/房间）。
    pub name: String,
}

impl From<&LevelDef> for LevelInfo {
    fn from(d: &LevelDef) -> Self {
        Self {
            level: d.level,
            code: d.code.clone(),
            name: d.name.clone(),
        }
    }
}

/// 末级展示信息的一项：**键名由服务端下发**（实测恒为 `信息`），前端通用渲染 `label: value`。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Field {
    pub label: String,
    pub value: String,
}

/// 末级视图（`map.showData` + 金额 + 提示）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElectricityView {
    /// `showData` 通用字典（键名/文案由服务端定，勿解构、勿硬编码）。
    pub fields: Vec<Field>,
    /// 金额（元；服务端 `money` / `iectranamt` 二者取有值者）。⚠️ 实测 448/449/450 **都不下发**这两个字段（恒 None）。
    pub money: Option<f64>,
    /// 提示文案（`tipinfo`；存在时 `fields` 为空，如房间号不存在）。
    pub tip: Option<String>,
}

/// 一次级联查询的结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElectricityQuery {
    /// 服务端层级定义（UI 用它渲染每级标签与深度）。
    pub levels: Vec<LevelInfo>,
    /// 下一级选项（空 = 该级无下拉；结合 [`FeeItem::last_level_is_input`] 判断是否输入级）。
    pub options: Vec<Choice>,
    /// 是否已到末级（房间）。
    pub is_final: bool,
    /// 末级视图（`is_final == true` 时有值；`fields`/`tip` 可能都为空）。
    pub view: Option<ElectricityView>,
}

// ---------------- 纯解析（单测覆盖，无网络） ----------------

/// 文本字段：字符串原样（trim），数字转字符串，缺失/其它 → 空串。
fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// 元金额字段：数字或数字字符串均可（服务端形态不定：实测 `money` 为数字、`retain_money` 为字符串）。
fn yuan_of(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// `status` 是否启用（实测为数字 `1`；容忍字符串形态，服务端类型有漂移先例）。
fn is_active(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        Some(Value::String(s)) => s.trim() == "1",
        _ => false,
    }
}

/// 单条片区（过滤前）。
fn parse_feeitem(it: &Value) -> FeeItem {
    let flag = text_of(it.get("flag"));
    FeeItem {
        id: text_of(it.get("feeitemid")),
        name: text_of(it.get("name")),
        billing_unit: text_of(it.get("billing_unit")),
        layout: text_of(it.get("layout"))
            .split(',')
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .collect(),
        retain_money: yuan_of(it.get("retain_money")),
        maxmoney: yuan_of(it.get("maxmoney")),
        remark: text_of(it.get("remark")),
        last_level_is_input: flag.chars().nth(FLAG_INDEX_UI_MODE) == Some(UI_MODE_SELECT_THEN_INPUT),
    }
}

/// 解析片区目录并**按电费页口径过滤**（`status==1 && impl_interface` 非空，见模块头注）。
pub fn parse_feeitems(v: &Value) -> Vec<FeeItem> {
    v["feeitemList"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|it| {
                    is_active(it.get("status")) && !text_of(it.get("impl_interface")).is_empty()
                })
                .map(parse_feeitem)
                .collect()
        })
        .unwrap_or_default()
}

/// 服务端层级定义（`map.total[]` 条目）。
#[derive(Debug, Clone, PartialEq)]
struct LevelDef {
    level: u32,
    code: String,
    name: String,
}

/// 解析 `map.total[]`（按 `level` 升序——官方前端亦 `sortBy(level)`）。
fn level_defs(map: &Value) -> Vec<LevelDef> {
    let mut defs: Vec<LevelDef> = map["total"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|t| LevelDef {
                    level: t["level"].as_u64().unwrap_or(0) as u32,
                    code: text_of(t.get("code")),
                    name: text_of(t.get("name")),
                })
                .collect()
        })
        .unwrap_or_default();
    defs.sort_by_key(|d| d.level);
    defs
}

/// 第 `step`（0 起）级之后的下一级 = `total[step]`（服务端 `level` 从 1 起）。
fn next_level_def(defs: &[LevelDef], step: usize) -> Result<&LevelDef, CampusSynjonesError> {
    defs.get(step).ok_or_else(|| {
        CampusSynjonesError::Parse("级联响应缺少 map.total 层级定义（无法得知下一级参数名）".to_string())
    })
}

/// `map.data` 条目 → [`Choice`]（`code`/`level` 取自该级定义，`name`/`value` 取自选项本身）。
fn choices_of(opts: &[Value], def: &LevelDef) -> Vec<Choice> {
    opts.iter()
        .map(|o| Choice {
            label: text_of(o.get("name")),
            value: text_of(o.get("value")),
            code: def.code.clone(),
            level: def.level,
        })
        .collect()
}

/// 末级响应 → [`ElectricityQuery`]（`showData` 通用字典 + 金额 + 提示；**不透出 `map.data` 的户号**）。
fn final_query(map: &Value, defs: &[LevelDef]) -> ElectricityQuery {
    let fields = map["showData"]
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| Field {
                    label: k.clone(),
                    value: text_of(Some(v)),
                })
                .collect()
        })
        .unwrap_or_default();
    let money = yuan_of(map.get("money")).or_else(|| yuan_of(map.get("iectranamt")));
    let tip = {
        let s = text_of(map.get("tipinfo"));
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };
    ElectricityQuery {
        levels: defs.iter().map(LevelInfo::from).collect(),
        options: Vec::new(),
        is_final: true,
        view: Some(ElectricityView { fields, money, tip }),
    }
}

// ---------------- 余额提取（M4；**严格形态，宁缺勿错**） ----------------

/// 余额关键词候选（**金额类**，按长度降序尝试）。
///
/// ⚠️ 刻意**不含**「剩余电量 / 电量」这类**非金额**词：末级文本里 `当前剩余电量957.50度`
/// 是 kWh 而非元，把它当成「余额」会让 UI 把电量当钱显示（错报数字）。
/// 实测 448 的文本同时出现「当前余额517.05元」与「当前剩余电量957.50度」，取前者即正确值，
/// 故排除电量词不影响任何一片区的真实取数。
const BALANCE_KEYWORDS: [&str; 3] = ["剩余金额", "剩余电费", "余额"];

/// 关键词与数字之间允许出现的分隔符（含「零宽紧邻」形态——实测 448/449 就是关键词直接贴数字；
/// `为`/`是` 是中文里不改变语义的连接词）。分隔符之外的内容一律不跳过（不是模糊匹配）。
const BALANCE_SEPARATORS: [char; 6] = [':', '：', '=', '＝', '为', '是'];

/// 严格提取一段文本里的余额数值：**关键词 + 分隔符 + 数字**，其余一概不认。
///
/// 三条纪律（与模块头注「绝不做文本解构」的取舍一致）：
/// 1. **只认相邻形态**：数字必须紧跟关键词（中间只允许 [`BALANCE_SEPARATORS`] 与空白），
///    绝不在文本里乱找「第一个数字」（那会在校方改文案时错报别的数）；
/// 2. **绝不位置解构**：不按逗号/顿号切段取第 N 段（三片区文案格式互不相同）；
/// 3. **宁缺勿错**：提不到返回 `None`，调用方按「无数据」展示，不显示臆造值。
///
/// 实测三片区样例（均应命中，见单测）：`剩余金额：-545.70，单价：0.5400`、
/// `当前余额517.05元,当前剩余电量957.50度`、`房间当前剩余电费625.35`。
pub fn balance_from_text(text: &str) -> Option<f64> {
    // 取**最早出现**的关键词（同位置时长词优先），后续数字才算它的值
    let mut best: Option<(usize, &str)> = None;
    for kw in BALANCE_KEYWORDS {
        for (pos, _) in text.match_indices(kw) {
            let better = match best {
                None => true,
                Some((p, k)) => pos < p || (pos == p && kw.len() > k.len()),
            };
            if better {
                best = Some((pos, kw));
            }
        }
    }
    let (pos, kw) = best?;
    let rest = &text[pos + kw.len()..];
    let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || BALANCE_SEPARATORS.contains(&c));
    parse_leading_number(rest)
}

/// 从文本开头解析一个严格形态的数字（允许前导 `-`、一个小数点）；解析不成立 → `None`。
fn parse_leading_number(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let neg = b.first() == Some(&b'-');
    let start = usize::from(neg);
    let mut i = start;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        // 关键词后没有数字（如 `余额不足，请充值`）→ 安静退化为「无数据」
        return None;
    }
    if b.get(i) == Some(&b'.') {
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > i + 1 {
            i = j;
        }
    }
    // 千分位歧义（`1,234.56`）：只取到 `1` 会错报 ⇒ 直接放弃
    let mut it = s[i..].chars();
    if let (Some(sep), Some(d)) = (it.next(), it.next()) {
        if matches!(sep, ',' | '，') && d.is_ascii_digit() {
            return None;
        }
    }
    s[..i].parse::<f64>().ok()
}

/// 从末级视图字段（`map.showData` 字典）提取余额数值 + **原始文本**（原文用于并存留证据）。
///
/// 逐字段**独立**扫描（不跨字段拼接——拼接可能造出「A 字段尾是关键词、B 字段首是数字」的假命中）；
/// `raw` 只是各字段值的拼接（实测恒单键「信息」，拼接仅为通用性）。
pub fn balance_from_fields(fields: &[Field]) -> (Option<f64>, String) {
    let balance = fields.iter().find_map(|f| balance_from_text(&f.value));
    let raw = fields
        .iter()
        .map(|f| f.value.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    (balance, raw)
}

// ---------------- 取数 ----------------

/// 片区目录（**匿名，无需 token**）。
///
/// 不走 [`SynjonesClient`]（其 `get` 强制 `ensure_token`）——匿名端点不该要求会话。
/// 头组仍照客户端惯例双份携带 `synAccessSource`（实测匿名四种组合皆 200，此处防学校侧后续收紧）。
pub async fn list_feeitems(cas: &CasClient) -> Result<Vec<FeeItem>, CampusSynjonesError> {
    let resp = cas
        .http_client()
        .get(format!("{BERSERKER_BASE}{EP_FEEITEM}"))
        .query(&[("synAccessSource", SYN_ACCESS_SOURCE)])
        .header("synAccessSource", SYN_ACCESS_SOURCE)
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| CampusSynjonesError::Http(e.to_string()))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    Ok(parse_feeitems(&parse_envelope(
        Envelope::Charge,
        status,
        &body,
    )?))
}

/// 发一次 `getThirdData`，返回响应整包（`map` 在 `v["map"]`）。
async fn third_data(
    client: &SynjonesClient,
    feeitem_id: &str,
    ty: &str,
    level: usize,
    picked: &[RoomStep],
) -> Result<Value, CampusSynjonesError> {
    let mut form: Vec<(String, String)> = vec![
        ("feeitemid".to_string(), feeitem_id.to_string()),
        ("type".to_string(), ty.to_string()),
        ("level".to_string(), level.to_string()),
    ];
    form.extend(picked.iter().map(|s| (s.code.clone(), s.value.clone())));
    let refs: Vec<(&str, String)> = form.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    client
        .post_form(EP_GET_THIRD_DATA, &refs, Envelope::Charge)
        .await
}

/// 级联查询。
///
/// `path` = 已选各级（按顺序，最后一级为房间号或某级选项值）；`path.len()` 即本次要问到的深度：
///
/// - `path` 为空 → 返回第 1 级选项（片区页首屏）。
/// - `path` 非空 → 重放这些选择；未到末级返回下一级选项（**空 `options` = 该级无下拉，末级输入级**），
///   到末级（`level == total.length`）自动切 `type=IEC` 并返回末级视图。
///
/// 实现按官方口径**从第 0 级重放**：必须靠每轮响应的 `map.total` 才能判定「何时该切 IEC」
/// （`total` 只在响应里，不在目录接口里）。故一次查询的请求数 = `path.len() + 1`（级数 ≤3，内网开销可忽略）。
/// 另有一条更短的官方路径——已绑定房间（`sceneinfo`）可用**单次** `type=IEC` 直取末级（实测可行性已由
/// live 探针的末级请求证实），但 `sceneinfo` 在平台侧绑定表里，本客户端不持有，故不采用。
pub async fn query_cascade(
    client: &SynjonesClient,
    feeitem_id: &str,
    path: &[RoomStep],
) -> Result<ElectricityQuery, CampusSynjonesError> {
    let feeitem_id = feeitem_id.trim();
    if feeitem_id.is_empty() {
        return Err(CampusSynjonesError::Parse("缺少片区 id".to_string()));
    }
    if path.len() > MAX_CASCADE_LEVELS {
        return Err(CampusSynjonesError::Parse(format!(
            "级联路径过长（{} 级，上限 {MAX_CASCADE_LEVELS}）",
            path.len()
        )));
    }
    if let Some(bad) = path.iter().position(|s| s.value.trim().is_empty()) {
        // 房间号给空串会被服务端当成 code=500 空文案（见模块头注），提前拦住给可操作文案
        return Err(CampusSynjonesError::Parse(format!(
            "第 {} 级未填值",
            bad + 1
        )));
    }

    let mut defs: Vec<LevelDef> = Vec::new();
    let mut step = 0usize;
    loop {
        // 末级判定按官方口径：`level == total.length` → `type` 切 `IEC`
        let ty = if !defs.is_empty() && step == defs.len() {
            "IEC"
        } else {
            "select"
        };
        let v = third_data(client, feeitem_id, ty, step, &path[..step]).await?;
        let map = &v["map"];
        if defs.is_empty() {
            defs = level_defs(map);
            if defs.is_empty() {
                return Err(CampusSynjonesError::Parse(
                    "级联响应缺少 map.total 层级定义（无法判定层数）".to_string(),
                ));
            }
        }
        if ty == "IEC" {
            return Ok(final_query(map, &defs));
        }

        let opts = map["data"].as_array().cloned().unwrap_or_default();
        if step == path.len() {
            // 到本次请求的深度：有选项 → 下一级下拉；无选项 → 该级无下拉（末级输入级，房间号由用户输入）
            let options = if opts.is_empty() {
                Vec::new()
            } else {
                choices_of(&opts, next_level_def(&defs, step)?)
            };
            return Ok(ElectricityQuery {
                levels: defs.iter().map(LevelInfo::from).collect(),
                options,
                is_final: false,
                view: None,
            });
        }
        // 未到深度：本级无 `data` 属正常（上一级已是输入级），继续重放
        step += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// **实测样本**（2026-09-19 匿名 `GET /charge/feeitem` 全量 8 条，只保留判据字段）。
    ///
    /// 钉死两条事实：① `status==1` 实为 **6** 条（非派单 brief 的 3 条）；
    /// ② 电费页口径（`status==1 && impl_interface` 非空）**恰好 3 条** = 448/449/450，停用的同名 428 不出现。
    fn feeitem_sample() -> Value {
        json!({
            "msg": "success",
            "code": 200,
            "feeitemList": [
                {"feeitemid": 181, "name": "补卡", "status": 1, "billing_unit": "元",
                 "layout": null, "retain_money": null, "maxmoney": null, "remark": "make-up-card",
                 "flag": "1100000000", "impl_interface": null},
                {"feeitemid": 401, "name": "慧新易校一卡通充值", "status": 1, "billing_unit": "元",
                 "layout": "1,10,50,100", "retain_money": null, "maxmoney": null, "remark": null,
                 "flag": "1100000000", "impl_interface": null},
                {"feeitemid": 407, "name": "扫商户码支付", "status": 1, "billing_unit": "元",
                 "layout": null, "retain_money": null, "maxmoney": null, "remark": null,
                 "flag": "1100000000", "impl_interface": null},
                {"feeitemid": 408, "name": "安科瑞电控", "status": 2, "billing_unit": "元",
                 "layout": "10,50,100", "retain_money": null, "maxmoney": null, "remark": null,
                 "flag": "1100100000", "impl_interface": "iECAcrelServiceImpl"},
                {"feeitemid": 428, "name": "梅园1号2号3号", "status": 2, "billing_unit": "元",
                 "layout": "10,50,100", "retain_money": "1", "maxmoney": "200", "remark": null,
                 "flag": "1100000000", "impl_interface": "iECTsmNewServiceImpl"},
                {"feeitemid": 448, "name": "桃园1号-李园8号", "status": 1, "billing_unit": "元",
                 "layout": "10,50,100", "retain_money": "1", "maxmoney": "200", "remark": null,
                 "flag": "1100300000", "impl_interface": "iECTsmNewServiceImpl"},
                {"feeitemid": 449, "name": "李园9号-李园11号", "status": 1, "billing_unit": "元",
                 "layout": "10,50,100", "retain_money": "1", "maxmoney": "200", "remark": null,
                 "flag": "1100300000", "impl_interface": "iECTsmNewServiceImpl"},
                {"feeitemid": 450, "name": "梅园1号-梅园3号", "status": 1, "billing_unit": "元",
                 "layout": "10,50,100", "retain_money": "1", "maxmoney": "200", "remark": null,
                 "flag": "1100300000", "impl_interface": "iECTsmNewServiceImpl"}
            ]
        })
    }

    /// 电费页口径：实测 8 条样本 → 恰好 3 条（448/449/450）；停用的 428（同名）与 408 不出现。
    #[test]
    fn parse_feeitems_keeps_only_enabled_third_party_items() {
        let items = parse_feeitems(&feeitem_sample());
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["448", "449", "450"], "应只剩三个启用电费片区");
        let f450 = items.iter().find(|i| i.id == "450").expect("450 应在内");
        assert_eq!(f450.name, "梅园1号-梅园3号");
        assert_eq!(f450.billing_unit, "元");
        assert_eq!(f450.layout, vec![10, 50, 100], "layout 逗号串 → 数字数组");
        assert_eq!(f450.retain_money, Some(1.0), "retain_money 字符串 → 元");
        assert_eq!(f450.maxmoney, Some(200.0), "maxmoney 字符串 → 元");
        assert_eq!(f450.remark, "");
        assert!(f450.last_level_is_input, "flag[4]=='3' → 末级是输入级（实测房间号靠用户输入）");
    }

    /// `status==1` 的原始条数（6）与「无 impl_interface」的非电费项——固化现实。
    #[test]
    fn sample_has_six_enabled_items_three_of_them_electricity() {
        let list = feeitem_sample()["feeitemList"].as_array().cloned().unwrap();
        let enabled = list.iter().filter(|it| is_active(it.get("status"))).count();
        assert_eq!(enabled, 6, "实测 status==1 有 6 条（补卡/充值/扫码 + 电费三片区）");
        assert_eq!(list.len(), 8, "实测目录全量 8 条");
        // 全下拉形态（flag[4]=='0'）不应被当成输入级
        let non_input = parse_feeitems(&json!({"feeitemList": [
            {"feeitemid": 9, "name": "X", "status": 1, "impl_interface": "impl", "flag": "1100000000"}
        ]}));
        assert!(!non_input[0].last_level_is_input);
    }

    /// 目录容错：缺 `feeitemList` / 空数组 / status 为字符串 / layout 垃圾段 → 不 panic。
    #[test]
    fn parse_feeitems_tolerates_missing_and_odd_shapes() {
        assert!(parse_feeitems(&json!({"code": 200})).is_empty());
        assert!(parse_feeitems(&json!({"feeitemList": []})).is_empty());
        let odd = parse_feeitems(&json!({"feeitemList": [
            {"feeitemid": 1, "name": "X", "status": "1", "impl_interface": "impl", "layout": "abc,10"}
        ]}));
        assert_eq!(odd.len(), 1, "status 为字符串 \"1\" 也应认作启用");
        assert_eq!(odd[0].layout, vec![10], "layout 里的垃圾段跳过");
        assert_eq!(odd[0].retain_money, None);
    }

    /// 层级定义：按 `level` 升序（官方前端同款），并据此给出下一级 code/level。
    #[test]
    fn level_defs_sorted_and_next_def_by_step() {
        let map = json!({"total": [
            {"level": 3, "code": "room", "name": "房间"},
            {"level": 1, "code": "campus", "name": "校区"},
            {"level": 2, "code": "building", "name": "楼栋"}
        ]});
        let defs = level_defs(&map);
        assert_eq!(defs.iter().map(|d| d.level).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(defs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), vec!["校区", "楼栋", "房间"]);
        assert_eq!(next_level_def(&defs, 0).unwrap().code, "campus");
        assert_eq!(next_level_def(&defs, 2).unwrap().level, 3);
        assert!(next_level_def(&defs, 3).is_err(), "越界应报 Parse 而不是 panic");
    }

    /// 选项 → Choice：`code`/`level` 取自该级定义，展示名/值取自选项（实测值形如 `2309046091500776&1号楼`）。
    #[test]
    fn choices_take_code_and_level_from_def() {
        let def = LevelDef {
            level: 2,
            code: "building".to_string(),
            name: "楼栋".to_string(),
        };
        let opts = vec![
            json!({"name": "1号楼", "value": "2309046091500776&1号楼"}),
            json!({"name": "3号楼", "value": "2407124814300001&3号楼"}),
        ];
        let cs = choices_of(&opts, &def);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].label, "1号楼");
        assert_eq!(cs[0].value, "2309046091500776&1号楼", "选项 value 原样透出（含 & 复合值）");
        assert_eq!(cs[0].code, "building");
        assert_eq!(cs[0].level, 2);
    }

    /// **末级实测样本**（450，2026-09-19 live 取证）：`showData` 只有 `信息` 一个键，值是自由文本，
    /// `money`/`iectranamt` 都不下发 ⇒ `money` 必须为 None，且**不得解构**该文本。
    #[test]
    fn final_query_keeps_free_text_info_and_no_money() {
        let map = json!({
            "data": {"account": "20230001", "room": "101", "roomName": "梅园1号101"},
            "showData": {"信息": "房间号：101,剩余金额：-545.70，单价：0.5400"}
        });
        let q = final_query(&map, &[]);
        assert!(q.is_final);
        assert!(q.options.is_empty());
        let view = q.view.expect("末级应有 view");
        assert_eq!(view.fields.len(), 1, "实测 showData 只有一个键");
        assert_eq!(view.fields[0].label, "信息", "键名由服务端下发（实测值）");
        assert_eq!(view.fields[0].value, "房间号：101,剩余金额：-545.70，单价：0.5400");
        assert_eq!(view.money, None, "实测 448/449/450 都不下发 money/iectranamt");
        assert_eq!(view.tip, None);
        // PII 纪律：户号不进任何对外字段
        let text = serde_json::to_string(&view).unwrap();
        assert!(!text.contains("20230001"), "map.data 的户号不得透出：{text}");
    }

    /// 末级提示：房间号非法时 `tipinfo` 有值、`showData` 为空（实测 `缴费系统返回数据错误child==NULL！`）。
    #[test]
    fn final_query_carries_tipinfo_when_room_invalid() {
        let map = json!({"dataType": "IEC", "feeFlag": 0, "tipinfo": "缴费系统返回数据错误child==NULL！"});
        let view = final_query(&map, &[]).view.unwrap();
        assert!(view.fields.is_empty());
        assert_eq!(view.tip.as_deref(), Some("缴费系统返回数据错误child==NULL！"));
        // 其它片区形态（448/449）也走同一路径：单键自由文本
        let map448 = json!({"showData": {"信息": "当前余额517.05元,当前剩余电量957.50度"}});
        let v = final_query(&map448, &[]).view.unwrap();
        assert_eq!(v.fields[0].value, "当前余额517.05元,当前剩余电量957.50度");
    }

    /// 前端传入的病态路径：超长 / 空片区 id / 某级空值 → 报 Parse（不发请求，不 panic）。
    #[tokio::test]
    async fn query_cascade_rejects_bad_input() {
        let cas = CasClient::new().expect("创建 CasClient 失败");
        let c = SynjonesClient::new(cas, None, None).with_target_url("/charge-pc/pays/450");
        let path: Vec<RoomStep> = (0..(MAX_CASCADE_LEVELS + 1))
            .map(|i| RoomStep {
                level: i as u32,
                code: "k".to_string(),
                value: "v".to_string(),
                name: String::new(),
            })
            .collect();
        let e = query_cascade(&c, "450", &path).await.expect_err("超长路径应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");

        let e = query_cascade(&c, "  ", &[]).await.expect_err("空片区 id 应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");

        // 房间号空串：服务端会回 code=500 空文案，必须提前拦住
        let empty_room = vec![RoomStep {
            level: 3,
            code: "room".to_string(),
            value: "  ".to_string(),
            name: String::new(),
        }];
        let e = query_cascade(&c, "450", &empty_room).await.expect_err("空房间号应报错");
        assert!(matches!(e, CampusSynjonesError::Parse(_)), "实际 {e:?}");
        assert!(e.to_string().contains("第 1 级未填值"), "实际 {e}");
    }

    // ---------- 余额提取（M4，严格形态） ----------

    /// **实测三片区文案全部命中**（这三句是 live 原文；金额单位元）。
    #[test]
    fn balance_extraction_hits_all_three_live_texts() {
        // 450：全角冒号 + 负号 + 小数；后面还跟着「单价」的另一个数字，不能取错
        assert_eq!(
            balance_from_text("房间号：101,剩余金额：-545.70，单价：0.5400"),
            Some(-545.70)
        );
        // 448：关键词**紧邻**数字（无分隔符），且同一句里有「剩余电量」的干扰数字
        assert_eq!(
            balance_from_text("当前余额517.05元,当前剩余电量957.50度"),
            Some(517.05)
        );
        // 449：无分隔符 + 无单位
        assert_eq!(balance_from_text("房间当前剩余电费625.35"), Some(625.35));
    }

    /// 关键词取**最早出现**的那个；分隔符形态多样（`：`/`:`/`是`/空格/无）。
    #[test]
    fn balance_extraction_takes_earliest_keyword_and_tolerates_separators() {
        assert_eq!(
            balance_from_text("单价：0.5400，余额：12.3"),
            Some(12.3),
            "单价在前也不干扰（只认关键词后的数字）"
        );
        assert_eq!(balance_from_text("余额: 8.5 元"), Some(8.5), "半角冒号 + 空格");
        assert_eq!(balance_from_text("余额是 9.9 元"), Some(9.9), "「是」连接词");
        assert_eq!(balance_from_text("剩余金额 100"), Some(100.0), "只有空格");
        assert_eq!(
            balance_from_text("余额0.00元"),
            Some(0.0),
            "0 是合法余额（不能与「提不到」混为一谈）"
        );
        assert_eq!(balance_from_text("余额：5元，优惠券：3张"), Some(5.0), "只取关键词后第一个数字");
    }

    /// **提不到就返回 None**（宁缺勿错）：无关键词 / 关键词后无数 / 电量词 / 千分位歧义 / 空串。
    #[test]
    fn balance_extraction_refuses_when_unsure() {
        assert_eq!(balance_from_text("房间号：101,单价：0.5400"), None, "没有余额关键词");
        assert_eq!(
            balance_from_text("当前剩余电量957.50度"),
            None,
            "电量（度）不是金额——刻意不认，绝不当余额显示"
        );
        assert_eq!(balance_from_text("余额不足，请及时充值"), None, "关键词后不是数字");
        assert_eq!(balance_from_text("剩余金额：--"), None, "只有符号没有数字");
        assert_eq!(balance_from_text("余额：1,234.56元"), None, "千分位歧义 → 宁可无数据");
        assert_eq!(balance_from_text("缴费系统返回数据错误child==NULL！"), None);
        assert_eq!(balance_from_text(""), None);
        // 关键词在句尾（无后续内容）也不能 panic
        assert_eq!(balance_from_text("当前余额"), None);
    }

    /// 多字段（`showData` 字典）：逐字段独立扫描，**不跨字段拼接**（拼接会造出假命中）。
    #[test]
    fn balance_from_fields_scans_fields_independently() {
        let hit = vec![Field {
            label: "信息".to_string(),
            value: "当前余额517.05元,当前剩余电量957.50度".to_string(),
        }];
        let (balance, raw) = balance_from_fields(&hit);
        assert_eq!(balance, Some(517.05));
        assert_eq!(raw, "当前余额517.05元,当前剩余电量957.50度", "原文原样返回（留证据）");

        let miss = vec![Field {
            label: "信息".to_string(),
            value: "房间号：101,单价：0.5400".to_string(),
        }];
        let (balance, raw) = balance_from_fields(&miss);
        assert_eq!(balance, None);
        assert_eq!(raw, "房间号：101,单价：0.5400");

        // 字段 A 尾是关键词、字段 B 首是数字 → **不得**拼起来命中（独立性纪律）
        let split = vec![
            Field {
                label: "a".to_string(),
                value: "当前余额".to_string(),
            },
            Field {
                label: "b".to_string(),
                value: "999.99".to_string(),
            },
        ];
        assert_eq!(balance_from_fields(&split).0, None, "跨字段拼接会误报，必须拒绝");

        let empty: Vec<Field> = Vec::new();
        let (balance, raw) = balance_from_fields(&empty);
        assert_eq!(balance, None);
        assert_eq!(raw, "");
    }
}
