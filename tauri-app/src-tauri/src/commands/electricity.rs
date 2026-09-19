//! 电费命令面（M3 批 3）：片区目录 / 级联查询 / 常用房间（本地存）/ 内嵌充值页。
//!
//! # 取数口径
//!
//! 目录与级联全部来自 `campus_synjones::charge`（协议事实见该模块头注，含两条 live 实测关键结论：
//! 「末级是输入级，服务端不下发下拉」「`showData` 键名恒为 `信息`，值是各片区格式不同的自由文本」）。
//!
//! # token 单活 → 沿用批 2 的全局唯一客户端
//!
//! 电费接口除目录外都需 token，且慧新E校 token 是**单活**的（并发 SSO 会互相顶掉）。
//! 故本模块**复用** `commands::synjones` 的进程级 `static SYNJONES` 与 `synjones_session`
//! （同锁、同客户端、同 `reenter`），**绝不另起第二套客户端**。匿名目录接口只需一个 reqwest
//! client（未登录时现建一个 `CasClient`），不触碰 token 缓存。
//!
//! # 常用房间：本地存（计划 §2.3）
//!
//! 平台侧 `sceneBind/add` 会改学校数据，不采用；房间只存本机
//! `%APPDATA%/campushub/electricity_rooms.json`。落盘沿用 `commands/profile.rs` 的同款极小
//! helper（`create_dir_all` + `to_string_pretty` + `fs::write`）——`infra/` 里没有通用 JSON
//! helper（`infra/timetable.rs` 是课表专用、`infra/state.rs` 是会话专用），故按既有范式在本模块
//! 内落地，不新造抽象。
//!
//! # 敏感纪律
//!
//! token **只在 Rust 内存**直接拼进 webview 初始化脚本与落点 URL（不经前端 JS API、不进日志、
//! 不进错误文案）；`showData` 之外的 `map.data` 含户号（PII），crate 层已不透出。

use super::auth::{session_client, CommandResult};
use super::synjones::{err_text, synjones_session};
use crate::infra::state::{self, AppState};
use campus_auth::cas::CasClient;
use campus_synjones::charge::{self, ElectricityQuery, FeeItem, RoomStep};
use campus_synjones::{CampusSynjonesError, BERSERKER_BASE, SYN_ACCESS_SOURCE};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

/// 无会话时的约定文案（与 portal.rs / profile.rs / synjones.rs 同口径）。
const ERR_NO_SESSION: &str = "请先登录";
/// 常用房间数量上限（防无界增长；超出时给可操作文案）。
const MAX_SAVED_ROOMS: usize = 20;
/// 充值窗口标题（用户要求：窗口标题用客户端品牌）。
const RECHARGE_TITLE: &str = "电费充值 · 锡院助手";
/// 充值窗口尺寸（用户要求：约 1000×760）。
const RECHARGE_SIZE: (f64, f64) = (1000.0, 760.0);
/// 充值页面路径前缀（官方 SPA；批 1 实测该路径是带 token 的合法落点）。
const RECHARGE_PATH_PREFIX: &str = "/charge-pc/pays/";

// ---------------- 常用房间（本地存） ----------------

/// 一个常用房间（计划 §2.3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedRoom {
    /// 本机 id（前端留空时由后端生成 epoch 毫秒字符串）。
    pub id: String,
    /// 所属片区（`FeeItem::id`）。
    pub feeitem_id: String,
    /// 片区名（冗余存下来，列表展示不必再拉目录）。
    #[serde(default)]
    pub feeitem_name: String,
    /// 完整选择路径（校区 → 楼栋 → 房间）；点击即按它重放级联。
    pub path: Vec<RoomStep>,
    /// 用户起的名字（留空时后端用路径名拼一个）。
    #[serde(default)]
    pub label: String,
}

fn rooms_path(dir: &Path) -> PathBuf {
    dir.join("electricity_rooms.json")
}

fn now_ms() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default()
}

/// 读取本地常用房间。文件缺失/损坏/反序列化失败 → 空列表（不报错、不删坏文件，
/// 与 `infra/timetable.rs` 的宽容读取同款——电费页首屏不能因存储异常白屏）。
pub fn load_rooms(dir: &Path) -> Vec<SavedRoom> {
    let Ok(raw) = fs::read_to_string(rooms_path(dir)) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<SavedRoom>>(&raw).unwrap_or_else(|e| {
        log::warn!("electricity_rooms.json 损坏，回退空列表: {e}");
        Vec::new()
    })
}

fn write_rooms(dir: &Path, rooms: &[SavedRoom]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let json = serde_json::to_string_pretty(rooms).map_err(|e| e.to_string())?;
    fs::write(rooms_path(dir), json).map_err(|e| format!("写 electricity_rooms.json 失败: {e}"))
}

/// 同一房间的判定键：片区 + 完整路径（代码与值）。
fn room_key(r: &SavedRoom) -> (String, Vec<(String, String)>) {
    (
        r.feeitem_id.clone(),
        r.path
            .iter()
            .map(|s| (s.code.clone(), s.value.clone()))
            .collect(),
    )
}

/// 房间标题：用户没起名时用路径名拼（取末两级，尽量有信息量）。
fn default_label(r: &SavedRoom) -> String {
    let names: Vec<&str> = r
        .path
        .iter()
        .map(|s| s.name.trim())
        .filter(|n| !n.is_empty())
        .collect();
    let tail = &names[names.len().saturating_sub(2)..];
    let joined = tail.join(" ");
    if joined.is_empty() {
        "未命名房间".to_string()
    } else {
        joined
    }
}

/// 新增/更新一个常用房间（纯逻辑，单测覆盖）：校验 → 同片区同路径视为同一房间（沿用原 id）
/// → 追加或替换 → 上限校验。返回新列表（不改动入参）。
pub fn upsert_room(mut rooms: Vec<SavedRoom>, mut room: SavedRoom) -> Result<Vec<SavedRoom>, String> {
    if room.feeitem_id.trim().is_empty() {
        return Err("缺少片区 id".to_string());
    }
    if room.path.is_empty() {
        return Err("房间路径为空，请先在页面上选到房间".to_string());
    }
    if let Some(bad) = room.path.iter().position(|s| s.value.trim().is_empty()) {
        return Err(format!("第 {} 级未填值", bad + 1));
    }
    if room.label.trim().is_empty() {
        room.label = default_label(&room);
    }
    let key = room_key(&room);
    match rooms.iter().position(|r| room_key(r) == key) {
        Some(i) => {
            // 同一房间重复保存 = 改名/刷新元信息，不产生重复项
            room.id = rooms[i].id.clone();
            rooms[i] = room;
            Ok(rooms)
        }
        None => {
            if rooms.len() >= MAX_SAVED_ROOMS {
                return Err(format!("常用房间已达上限（{MAX_SAVED_ROOMS} 个）"));
            }
            if room.id.trim().is_empty() {
                room.id = now_ms();
            }
            rooms.push(room);
            Ok(rooms)
        }
    }
}

/// 删除一个常用房间（纯逻辑，单测覆盖）；id 不存在则原样返回。
pub fn remove_room(rooms: Vec<SavedRoom>, id: &str) -> Vec<SavedRoom> {
    rooms.into_iter().filter(|r| r.id != id).collect()
}

// ---------------- 目录 / 级联 ----------------

/// 电费侧失败文案：会话失效沿批 2 口径；网络层失败点明「需校园网」（内网明文 IP，校外不可达）；
/// 服务端空文案（实测房间号给空串时 `code=500` 且 `message` 为空）补一句可读话。
fn elec_err(e: &CampusSynjonesError) -> String {
    match e {
        CampusSynjonesError::Http(msg) => {
            format!("无法访问学校服务，请确认已连校园网（{msg}）")
        }
        CampusSynjonesError::Api { code, msg } if msg.trim().is_empty() => {
            format!("学校服务返回异常（code={code}）")
        }
        other => err_text(other),
    }
}

/// 片区目录（**免登录可调**：`/charge/feeitem` 是该服务唯一匿名端点，见 crate 头注）。
/// 已登录时复用会话 client（同 Cookie jar），未登录时现建一个——不触碰 token 缓存。
#[tauri::command]
pub async fn list_feeitems(
    state: State<'_, AppState>,
) -> Result<CommandResult<Vec<FeeItem>>, String> {
    let cas = match session_client(&state).await {
        Some(c) => c,
        None => match CasClient::new() {
            Ok(c) => c,
            Err(e) => return Ok(CommandResult::err(&format!("初始化网络客户端失败：{e}"))),
        },
    };
    Ok(match charge::list_feeitems(&cas).await {
        Ok(items) => CommandResult::ok(items),
        Err(e) => CommandResult::err(&elec_err(&e)),
    })
}

/// 级联查询：`path` 为空取第 1 级选项；非空则重放该路径。
/// 末级（房间）是**输入级**（服务端不下发选项）——`options` 为空且 `isFinal == false` 时，
/// 前端按 `FeeItem.lastLevelIsInput` 把该级渲染成输入框（房间号），带上房间号再请求一次即得 `view`。
#[tauri::command]
pub async fn query_electricity(
    state: State<'_, AppState>,
    feeitem_id: String,
    path: Vec<RoomStep>,
) -> Result<CommandResult<ElectricityQuery>, String> {
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    Ok(
        match charge::query_cascade(&sess.client, &feeitem_id, &path).await {
            Ok(q) => CommandResult::ok(q),
            Err(e) => CommandResult::err(&elec_err(&e)),
        },
    )
}

// ---------------- 常用房间命令 ----------------

/// 本地常用房间列表（无入参，读盘失败回空列表）。
#[tauri::command]
pub async fn get_electricity_rooms() -> Result<CommandResult<Vec<SavedRoom>>, String> {
    let dir = state::data_dir()?;
    Ok(CommandResult::ok(load_rooms(&dir)))
}

/// 保存常用房间（upsert），返回更新后的完整列表。
#[tauri::command]
pub async fn save_electricity_room(
    room: SavedRoom,
) -> Result<CommandResult<Vec<SavedRoom>>, String> {
    let dir = state::data_dir()?;
    match upsert_room(load_rooms(&dir), room) {
        Ok(rooms) => match write_rooms(&dir, &rooms) {
            Ok(()) => Ok(CommandResult::ok(rooms)),
            Err(e) => Ok(CommandResult::err(&e)),
        },
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

/// 删除常用房间，返回更新后的完整列表。
#[tauri::command]
pub async fn delete_electricity_room(id: String) -> Result<CommandResult<Vec<SavedRoom>>, String> {
    let dir = state::data_dir()?;
    let rooms = remove_room(load_rooms(&dir), &id);
    match write_rooms(&dir, &rooms) {
        Ok(()) => Ok(CommandResult::ok(rooms)),
        Err(e) => Ok(CommandResult::err(&e)),
    }
}

// ---------------- 内嵌充值页（计划 §2.6） ----------------

/// 窗口 label：只留 ASCII 字母数字（Tauri label 限制）+ 长度截断。
fn window_label(feeitem_id: &str, room: Option<&SavedRoom>) -> String {
    let safe = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(24)
            .collect()
    };
    match room {
        Some(r) => {
            let room_id = safe(&r.id);
            if room_id.is_empty() {
                // 前端可能只带路径、不带 id（房间参数仅用于窗口标识）→ 退回「每片区一窗」
                format!("recharge-{}", safe(feeitem_id))
            } else {
                format!("recharge-{}-{room_id}", safe(feeitem_id))
            }
        }
        None => format!("recharge-{}", safe(feeitem_id)),
    }
}

/// 内嵌充值页的 4030 处置脚本：**在官方页面脚本之前**接管 fetch/XHR，把 `synAccessSource` 的
/// `pc` 值改写为 [`SYN_ACCESS_SOURCE`]（`app`）。
///
/// # 为什么需要它（2026-09-19 实测确证）
///
/// 学校慧新E校服务端按请求参数 `synAccessSource` 做**来源授权**：`pc` 来源被拒、`app` 来源放行，
/// 拒绝形态是 HTTP 401 + `body.code == 4030`。官方 `charge-pc` 页自己部分请求**硬编码**
/// `synAccessSource=pc`（不读我们注入的 `agentType`），于是内嵌窗弹出「提示 服务大厅未授权(1)」，
/// 充值流程被挡。宿主侧无法约束官方页的请求体，只能在页面侧把这些请求的来源参数改写掉。
///
/// # 覆盖范围（三种携带位置，且**只**改这一个参数）
///
/// 1. URL query —— `fetch(url)` / `XHR.open(method, url)`
/// 2. 请求头 —— `fetch` 的 `init.headers`（`Headers` / 键值对数组 / 普通对象）、`Request` 实例的
///    `headers`、`XHR.setRequestHeader`（键名大小写不敏感）
/// 3. 请求体 —— `XHR.send(body)` / `fetch` 的 `init.body`，覆盖 urlencoded 字符串、
///    `URLSearchParams`、`FormData`
///
/// 思路直接借鉴用户自写的油猴脚本 `xll-apk-analysis/fix-4030.user.js`（同目的：hook fetch/XHR
/// 改写来源参数），此处补齐请求头与请求体两条路径，并**内联为常量**（不引用外部文件、不新增依赖）。
///
/// # 纪律
///
/// - 幂等：`window.__campushub4030Hook` 只装一次；改写只匹配 `synAccessSource=pc`，重复执行结果不变。
/// - 绝不碰其它内容（token 头等），也**不新增**该参数——官方没带的请求保持原样。
/// - 全程 `try/catch`：hook 的任何异常都不许影响后续 token/configs 注入与官方页面本身。
///
/// **这是为绕开学校服务端 PC 来源授权缺陷（4030）而做的参数改写，属已知的临时措施；
/// 学校若修复 PC 授权即可整段移除（连同 [`init_script`] 里对本常量的拼接）。**
///
/// `__SYN_ACCESS_SOURCE__` 占位符由 [`init_script`] 替换为 [`SYN_ACCESS_SOURCE`]。
const RECHARGE_4030_HOOK: &str = r#"(function () {
  'use strict';
  try {
    if (window.__campushub4030Hook) return;
    window.__campushub4030Hook = true;
    var SOURCE = '__SYN_ACCESS_SOURCE__';
    var isSourceKey = function (name) {
      return typeof name === 'string' && name.toLowerCase() === 'synaccesssource';
    };
    var isPc = function (value) { return typeof value === 'string' && /^pc$/i.test(value); };

    /* 位置一：URL query（fetch(url) / XHR.open(method, url)）。只改 synAccessSource 的值。 */
    var fixUrl = function (url) {
      try {
        if (typeof url !== 'string' || url.indexOf('synAccessSource') === -1) return url;
        return url.replace(/(^|[?&])synAccessSource=pc(?![0-9A-Za-z_])/gi, '$1synAccessSource=' + SOURCE);
      } catch (e) { return url; }
    };

    /* 位置二：请求头。键名大小写不敏感，值只有 pc 才改。 */
    var fixHeaders = function (headers) {
      try {
        if (!headers) return headers;
        if (typeof Headers !== 'undefined' && headers instanceof Headers) {
          if (!isPc(headers.get('synAccessSource'))) return headers;
          try {
            headers.set('synAccessSource', SOURCE);
            return headers;
          } catch (e) {
            var cloned = new Headers(headers);
            cloned.set('synAccessSource', SOURCE);
            return cloned;
          }
        }
        if (Array.isArray(headers)) {
          return headers.map(function (pair) {
            if (pair && isSourceKey(pair[0]) && isPc(pair[1])) return [pair[0], SOURCE];
            return pair;
          });
        }
        if (typeof headers === 'object') {
          var out = {};
          var hit = false;
          Object.keys(headers).forEach(function (key) {
            if (isSourceKey(key) && isPc(headers[key])) { out[key] = SOURCE; hit = true; }
            else { out[key] = headers[key]; }
          });
          return hit ? out : headers;
        }
      } catch (e) {}
      return headers;
    };

    /* 位置三：请求体。覆盖 urlencoded 字符串 / URLSearchParams / FormData。 */
    var fixBody = function (body) {
      try {
        if (typeof body === 'string') {
          if (body.indexOf('synAccessSource') === -1) return body;
          return body.replace(/(^|&)synAccessSource=pc(?![0-9A-Za-z_])/gi, '$1synAccessSource=' + SOURCE);
        }
        if (typeof URLSearchParams !== 'undefined' && body instanceof URLSearchParams) {
          if (isPc(body.get('synAccessSource'))) body.set('synAccessSource', SOURCE);
          return body;
        }
        if (typeof FormData !== 'undefined' && body instanceof FormData) {
          if (isPc(body.get('synAccessSource'))) body.set('synAccessSource', SOURCE);
          return body;
        }
      } catch (e) {}
      return body;
    };

    /* Request 实例：URL 须改写时重建（body 走 duplex 半双工透传）；失败退回原对象，绝不阻断请求。 */
    var rebuildRequest = function (req, url) {
      try {
        var opt = { method: req.method, headers: new Headers(req.headers) };
        ['mode', 'credentials', 'cache', 'redirect', 'referrer', 'referrerPolicy',
          'integrity', 'keepalive', 'signal'].forEach(function (key) {
          try { if (req[key] != null) opt[key] = req[key]; } catch (e) {}
        });
        if (req.body) { opt.body = req.body; opt.duplex = 'half'; }
        return new Request(url, opt);
      } catch (e) { return null; }
    };

    /* 官方页 axios 走 XHR —— 主路径。 */
    var xhrOpen = XMLHttpRequest.prototype.open;
    var xhrSend = XMLHttpRequest.prototype.send;
    var xhrSetHeader = XMLHttpRequest.prototype.setRequestHeader;
    XMLHttpRequest.prototype.open = function () {
      try { arguments[1] = fixUrl(arguments[1]); } catch (e) {}
      return xhrOpen.apply(this, arguments);
    };
    XMLHttpRequest.prototype.send = function (body) {
      try { body = fixBody(body); } catch (e) {}
      return xhrSend.call(this, body);
    };
    XMLHttpRequest.prototype.setRequestHeader = function (name, value) {
      try { if (isSourceKey(name) && isPc(value)) value = SOURCE; } catch (e) {}
      return xhrSetHeader.call(this, name, value);
    };

    if (typeof window.fetch === 'function') {
      var nativeFetch = window.fetch;
      window.fetch = function (input, init) {
        try {
          if (typeof input === 'string') {
            input = fixUrl(input);
          } else if (typeof Request !== 'undefined' && input instanceof Request) {
            try { fixHeaders(input.headers); } catch (e) {}
            var fixed = fixUrl(input.url);
            if (fixed !== input.url) {
              var rebuilt = rebuildRequest(input, fixed);
              if (rebuilt) input = rebuilt;
            }
          } else if (input) {
            input = fixUrl(String(input));
          }
          if (init) {
            var headers = fixHeaders(init.headers);
            var body = fixBody(init.body);
            if (headers !== init.headers || body !== init.body) {
              init = Object.assign({}, init, { headers: headers, body: body });
            }
          }
        } catch (e) {}
        return nativeFetch.call(this, input, init);
      };
    }
  } catch (e) { /* hook 失败不影响后续 token/configs 注入与官方页面 */ }
})();
"#;

/// 官方充值页所需的注入内容（2026-09-19 逆向官方 bundle + 实测确定）：
///
/// 1. `localStorage.configs` —— 官方 bundle 在**模块顶层** `JSON.parse(localStorage.getItem("configs"))`
///    后取 `.base` 作 axios baseURL。该键本应由一次**同步** XHR 拉 `/config/base.config.json`
///    （随后 `e.pc.base = window.location.origin`）写入；**那次请求一旦失败，`JSON.parse(null)` 会让整页抛错白屏**，
///    故必须预注入。形状 = 该文件 `pc` 对象：实测 `{"title":"xayf","version":"1.0.0"}` + `base`。
/// 2. `sessionStorage.access_token` —— 官方 store 初始化即 `get("access_token", true)`，其 getter 实现为
///    `JSON.parse(原文)`（失败回落原文）；官方 setter 对**字符串**是原样存（不 stringify），
///    故此处也**原样存裸 token**，与官方写入口径一致，任何按原文读的第三方读取者也不会拿到带引号的值。
///    官方原路径是落点 URL 的 `synjones-auth` 参数（`App.getInfo()`），我们两处都给（见 [`recharge_url`]）。
/// 3. `sessionStorage.agentType` —— 官方请求拦截器**按原文**读它（`sessionStorage.getItem("agentType")`），
///    作为 FormData 分支的 `synAccessSource`；官方代码只在拦截器里把它硬编码成 `pc` 用于 GET/其它 POST。
///    该值由宿主应用写入（本 bundle 无写入点），故填 [`SYN_ACCESS_SOURCE`]（`app`）。
/// 4. `token_type` —— 官方有缺省 `bearer`，仍显式写入，防未来版本改缺省。
///
/// 官方还有 `ecardConfigPC`/`frontConfigPC`/`currentThemePC` 三个 localStorage 键（主题色），
/// **不预注入**：缺失时官方会自己同步 XHR `/berserker-app/frontInfo?type=pc&synAccessSource=pc`
/// （2026-09-19 实测匿名 200）补齐；预注入反而可能与其主题结构不符或被覆盖。
///
/// 5. **4030 处置 hook**（见 [`RECHARGE_4030_HOOK`]）——必须排在脚本最前：`initialization_script`
///    保证先于官方页面脚本执行，hook 装上后才轮得到官方页发请求；属**已知临时措施**
///    （学校修复 PC 来源授权即可移除）。
///
/// 脚本在**每次导航**都会执行，故整体幂等（样式只在缺失时插一次、hook 只装一次、改写只匹配 `pc`）。
fn init_script(token: &str) -> String {
    let token_js = serde_json::to_string(token).unwrap_or_else(|_| "\"\"".to_string());
    // hook 先于 token/configs 注入落位；占位符换成 crate 常量，避免在 JS 里重复硬编码来源值。
    let hook = RECHARGE_4030_HOOK.replace("__SYN_ACCESS_SOURCE__", SYN_ACCESS_SOURCE);
    format!(
        r#"{hook}(function () {{
  try {{
    sessionStorage.setItem('access_token', {token_js});
    sessionStorage.setItem('token_type', 'bearer');
    sessionStorage.setItem('agentType', '{source}');
    localStorage.setItem('configs', JSON.stringify({{ title: 'xayf', version: '1.0.0', base: '{base}' }}));
    // 统一显示风格（尽力而为）：官方把主题色写成 .theme-class 上的 --color-* 变量，
    // 这里用更高优先级 + !important 把主色族覆盖为「锡院紫」；官方改版换变量名即失效。
    if (!document.getElementById('campushub-recharge-style')) {{
      var s = document.createElement('style');
      s.id = 'campushub-recharge-style';
      s.textContent = [
        'html.theme-class, :root {{',
        '  --color-primary: #5b2e90 !important;',
        '  --color-primary-hover: #7040a8 !important;',
        '  --color-primary-active: #47246f !important;',
        '  --color-primary-disabled: #ad97c8 !important;',
        '  --color-primary-hover-border: #ad97c8 !important;',
        '  --color-primary-hsla: 272, 52%, 37% !important;',
        '  --color-gradualPrimary: linear-gradient(0, #7040a8 0%, #5b2e90 100%) !important;',
        '}}',
        '.el-button, .el-input__inner, .el-card, .el-dialog, .el-message-box {{ border-radius: 8px !important; }}'
      ].join('\n');
      (document.head || document.documentElement).appendChild(s);
    }}
  }} catch (e) {{ /* 注入失败不阻塞官方页面：官方仍会自行拉 config 与主题 */ }}
}})();"#,
        base = BERSERKER_BASE,
        source = SYN_ACCESS_SOURCE
    )
}

/// 充值页 URL。带官方落点参数 `synjones-auth`（批 1 实测 `/charge-pc/pays/450?synjones-auth=…`
/// 是该页合法落点，其 `App.getInfo()` 会读它、写入 sessionStorage 并顺手拉 userInfo）；
/// 初始化脚本同时写 sessionStorage，两条路径互不冲突，任一条成立即已登录。
fn recharge_url(token: &str, feeitem_id: &str) -> Result<Url, String> {
    let raw = format!(
        "{BERSERKER_BASE}{RECHARGE_PATH_PREFIX}{}?synjones-auth={token}",
        feeitem_id.trim()
    );
    raw.parse::<Url>()
        .map_err(|e| format!("充值页地址非法：{e}"))
}

/// 在应用内 webview 打开官方充值页（已是登录态，无需跳浏览器）。
///
/// - 先取客户端缓存里的 token；没有才走一次 SSO（`reenter`）——**不重复 SSO**，避免把其他调用方
///   正在用的 token 顶掉（token 单活，见 crate 头注）。
/// - 同一房间重复打开只聚焦已有窗口（label 唯一），不叠窗口。
/// - 窗口以主窗口为父：主窗口关闭时一并关闭（Windows 属主窗口语义）；挂父失败退化为独立窗口。
/// - **不给官方页面任何 Tauri API 能力**，且**无需改 `capabilities/`**（2026-09-19 查证）：
///   `core:webview:allow-create-webview-window` 只管控**前端 JS 命令**
///   （`tauri::webview::plugin::create_webview_window`），本命令走 Rust `WebviewWindowBuilder::build()`，
///   不受 ACL 约束；官方页所在窗口不被任何 capability 覆盖 ⇒ 其 JS 无 IPC 权限
///   （Tauri 2 默认只对 `withGlobalTauri: true` 且 capability 显式授权的源注入 IPC；
///   本工程 `withGlobalTauri` 缺省 false）。
#[tauri::command]
pub async fn open_recharge_page(
    app: AppHandle,
    state: State<'_, AppState>,
    feeitem_id: String,
    room: Option<SavedRoom>,
) -> Result<CommandResult<()>, String> {
    let id = feeitem_id.trim();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Ok(CommandResult::err("片区 id 非法"));
    }
    let Some(guard) = synjones_session(&state).await else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let Some(sess) = guard.as_ref() else {
        return Ok(CommandResult::err(ERR_NO_SESSION));
    };
    let token = match sess.client.token() {
        Some(t) if !t.is_empty() => t,
        _ => match sess.client.reenter().await {
            Ok(t) => t,
            Err(e) => return Ok(CommandResult::err(&elec_err(&e))),
        },
    };

    let label = window_label(id, room.as_ref());
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_focus();
        return Ok(CommandResult::empty());
    }
    let url = match recharge_url(&token.access_token, id) {
        Ok(u) => u,
        Err(e) => return Ok(CommandResult::err(&e)),
    };
    let script = init_script(&token.access_token);
    let make = || {
        WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url.clone()))
            .title(RECHARGE_TITLE)
            .inner_size(RECHARGE_SIZE.0, RECHARGE_SIZE.1)
            .initialization_script_for_all_frames(script.clone())
    };
    let builder = match app.get_webview_window("main") {
        Some(main) => match make().parent(&main) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("充值窗口挂主窗口失败（{e}），退化为独立窗口");
                make()
            }
        },
        None => make(),
    };
    // 只记 label 与片区 id（URL 含 token，绝不进日志）
    match builder.build() {
        Ok(_) => {
            log::info!("已打开充值窗口 {label}（片区 {id}）");
            Ok(CommandResult::empty())
        }
        Err(e) => Ok(CommandResult::err(&format!("打开充值窗口失败：{e}"))),
    }
}

/// 兜底：在系统浏览器打开同一官方充值页（内嵌窗被学校侧改动搞坏时的退路）。
/// 入参只允许数字片区 id，URL 由后端拼装，前端无法借它打开任意地址（与 `portal::open_in_browser`
/// 的白名单思路一致，只是这里的合法目标是内网 IP）。
#[tauri::command]
pub async fn open_recharge_in_browser(
    app: AppHandle,
    feeitem_id: String,
) -> Result<CommandResult<()>, String> {
    let id = feeitem_id.trim();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Ok(CommandResult::err("片区 id 非法"));
    }
    let url = format!("{BERSERKER_BASE}{RECHARGE_PATH_PREFIX}{id}");
    Ok(
        match app.opener().open_url(url.clone(), None::<&str>) {
            Ok(()) => CommandResult::empty(),
            Err(e) => CommandResult::err(&format!("打开浏览器失败：{e}")),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("campushub-elec-{tag}-{n}"))
    }

    fn room(label: &str, value: &str) -> SavedRoom {
        SavedRoom {
            id: String::new(),
            feeitem_id: "450".to_string(),
            feeitem_name: "梅园1号-梅园3号".to_string(),
            path: vec![
                RoomStep {
                    level: 1,
                    code: "campus".to_string(),
                    value: "1&无锡学院".to_string(),
                    name: "无锡学院".to_string(),
                },
                RoomStep {
                    level: 2,
                    code: "building".to_string(),
                    value: "2309&1号楼".to_string(),
                    name: "1号楼".to_string(),
                },
                RoomStep {
                    level: 3,
                    code: "room".to_string(),
                    value: value.to_string(),
                    name: value.to_string(),
                },
            ],
            label: label.to_string(),
        }
    }

    /// 落盘往返：save → load 相等；JSON 为 camelCase 明文（与前端 DTO 一致）。
    #[test]
    fn rooms_roundtrip_and_camel_case() {
        let dir = temp_dir("rt");
        let rooms = upsert_room(Vec::new(), room("我的宿舍", "101")).unwrap();
        assert_eq!(rooms.len(), 1);
        assert!(!rooms[0].id.is_empty(), "空 id 应由后端补上");
        write_rooms(&dir, &rooms).unwrap();

        let raw = fs::read_to_string(rooms_path(&dir)).unwrap();
        assert!(raw.contains("\"feeitemId\""), "落盘应 camelCase：{raw}");
        assert_eq!(load_rooms(&dir), rooms);
        fs::remove_dir_all(&dir).ok();
    }

    /// 同一片区同一路径重复保存 → 原地更新（沿用原 id），不产生重复项；换房间号 → 新项。
    #[test]
    fn upsert_dedups_same_path_and_keeps_id() {
        let rooms = upsert_room(Vec::new(), room("宿舍", "101")).unwrap();
        let first_id = rooms[0].id.clone();
        let renamed = upsert_room(rooms, room("宿舍改名", "101")).unwrap();
        assert_eq!(renamed.len(), 1, "同房间重复保存不应新增");
        assert_eq!(renamed[0].id, first_id, "重复保存应沿用原 id");
        assert_eq!(renamed[0].label, "宿舍改名", "新名字生效");

        let more = upsert_room(renamed, room("隔壁", "102")).unwrap();
        assert_eq!(more.len(), 2, "不同房间号应新增");
    }

    /// label 缺省由路径名拼（末两级）；路径名全空 → 「未命名房间」。
    #[test]
    fn missing_label_gets_default_from_path() {
        let rooms = upsert_room(Vec::new(), room("", "101")).unwrap();
        assert_eq!(rooms[0].label, "1号楼 101");
        let mut nameless = room("", "101");
        for s in &mut nameless.path {
            s.name = String::new();
        }
        assert_eq!(default_label(&nameless), "未命名房间");
    }

    /// 校验：缺片区 id / 空路径 / 某级空值 → 报错；超上限 → 报错。
    #[test]
    fn upsert_validates_input_and_cap() {
        let mut no_feeitem = room("x", "101");
        no_feeitem.feeitem_id = "  ".to_string();
        assert!(upsert_room(Vec::new(), no_feeitem).is_err());

        let mut empty_path = room("x", "101");
        empty_path.path.clear();
        assert!(upsert_room(Vec::new(), empty_path).is_err());

        let mut blank_room = room("x", "101");
        blank_room.path[2].value = " ".to_string();
        let e = upsert_room(Vec::new(), blank_room).unwrap_err();
        assert!(e.contains("第 3 级未填值"), "实际 {e}");

        let mut all: Vec<SavedRoom> = Vec::new();
        for i in 0..MAX_SAVED_ROOMS {
            all = upsert_room(all, room(&format!("r{i}"), &format!("{i}01"))).unwrap();
        }
        assert_eq!(all.len(), MAX_SAVED_ROOMS);
        let e = upsert_room(all, room("溢出", "999")).unwrap_err();
        assert!(e.contains("上限"), "实际 {e}");
    }

    /// 删除：命中即移除，未知 id 原样返回；文件缺失/损坏 → 空列表不 panic（坏文件保留现场）。
    #[test]
    fn remove_and_tolerant_load() {
        let rooms = upsert_room(Vec::new(), room("宿舍", "101")).unwrap();
        let id = rooms[0].id.clone();
        assert!(remove_room(rooms.clone(), &id).is_empty());
        assert_eq!(remove_room(rooms, "不存在").len(), 1);

        let dir = temp_dir("missing");
        assert!(load_rooms(&dir).is_empty());
        fs::create_dir_all(&dir).unwrap();
        fs::write(rooms_path(&dir), "{ not json").unwrap();
        assert!(load_rooms(&dir).is_empty());
        assert!(rooms_path(&dir).exists(), "坏文件保留现场");
        fs::remove_dir_all(&dir).ok();
    }

    /// 初始化脚本：四个必需键都在、品牌样式在、token 只作 JS 字符串字面量、整体幂等。
    #[test]
    fn init_script_carries_required_keys_and_brand_css() {
        let s = init_script("T0KEN");
        for key in [
            r#"sessionStorage.setItem('access_token', "T0KEN")"#,
            "sessionStorage.setItem('token_type', 'bearer')",
            "sessionStorage.setItem('agentType', 'app')",
            "localStorage.setItem('configs'",
            "campushub-recharge-style",
            "--color-primary: #5b2e90",
        ] {
            assert!(s.contains(key), "脚本缺 {key}：{s}");
        }
        assert!(s.contains(BERSERKER_BASE), "configs.base 应为内网根");
        assert!(!s.contains("bearer T0KEN"), "token 不带 bearer 前缀（官方自己拼）");
        assert!(s.contains("if (!document.getElementById('campushub-recharge-style'))"));
        // 含引号/反斜杠的 token 也必须被转义成合法 JS 字符串
        let risky = init_script("a\"b\\c");
        assert!(
            risky.contains(r#"sessionStorage.setItem('access_token', "a\"b\\c")"#),
            "{risky}"
        );
    }

    /// 4030 处置 hook：排在脚本最前（官方页面脚本之前）、覆盖三种携带位置、占位符已替换、
    /// 不引用外部文件；注入本体（token/configs）与品牌样式不受影响。
    #[test]
    fn init_script_puts_4030_hook_first_and_covers_three_carriers() {
        let s = init_script("T0KEN");
        let hook_at = s.find("__campushub4030Hook").expect("缺 hook 幂等标记");
        let inject_at = s
            .find("sessionStorage.setItem('access_token'")
            .expect("缺 token 注入");
        assert!(hook_at < inject_at, "hook 必须最先执行（初始化脚本首段）");

        // 位置一/二/三：URL query、请求头、请求体
        for mark in [
            "XMLHttpRequest.prototype.open =",
            "XMLHttpRequest.prototype.setRequestHeader =",
            "XMLHttpRequest.prototype.send =",
            "window.fetch =",
            "init.headers",
            "init.body",
            "URLSearchParams",
            "FormData",
        ] {
            assert!(s.contains(mark), "hook 缺覆盖点 {mark}");
        }
        // 只改 synAccessSource 这一个值：正则与替换片段都在，且顺序为 hook → 注入 → 样式
        assert!(s.contains("synAccessSource=' + SOURCE"), "{s}");
        assert!(s.contains(r"synAccessSource=pc(?![0-9A-Za-z_])"));
        assert!(!s.contains("__SYN_ACCESS_SOURCE__"), "占位符必须已替换");
        assert!(s.contains("var SOURCE = 'app';"), "来源值应取 crate 常量");
        assert!(!s.contains("fix-4030.user.js"), "不得引用外部脚本文件");
        // 只新增该参数是不允许的：脚本不含「无条件补写 app」的追加逻辑
        assert!(!s.contains("includes('?') ? '&' : '?'"), "不得为缺失参数的请求补写来源");
    }

    /// 充值 URL 与窗口 label：URL 带官方落点参数、id 去空白；label 只含 ASCII 字母数字并截断。
    #[test]
    fn recharge_url_and_window_label() {
        let url = recharge_url("T0KEN", "450 ").unwrap();
        assert_eq!(
            url.as_str(),
            "http://10.3.100.110/charge-pc/pays/450?synjones-auth=T0KEN"
        );
        let mut r = room("宿舍", "101");
        r.id = "1758-2/9:9".to_string();
        assert_eq!(window_label("450", Some(&r)), "recharge-450-1758299");
        assert_eq!(window_label("450", None), "recharge-450");
        // id 为空（只带路径）→ 退回每片区一窗，不产生 "recharge-450-" 这类尾部连字符
        let mut no_id = room("宿舍", "101");
        no_id.id = String::new();
        assert_eq!(window_label("450", Some(&no_id)), "recharge-450");
        let mut long = room("宿舍", "101");
        long.id = "a".repeat(80);
        assert_eq!(
            window_label("450", Some(&long)).len(),
            "recharge-450-".len() + 24,
            "超长 id 应截断"
        );
    }

    /// 失败文案：网络层失败点明「需校园网」，空文案的业务错误不出现悬空冒号。
    #[test]
    fn elec_err_messages_are_actionable() {
        let net = elec_err(&CampusSynjonesError::Http("HTTP 502".to_string()));
        assert!(net.contains("校园网"), "实际 {net}");
        let api = elec_err(&CampusSynjonesError::Api {
            code: 500,
            msg: String::new(),
        });
        assert_eq!(api, "学校服务返回异常（code=500）");
        assert_eq!(
            elec_err(&CampusSynjonesError::NotLogin),
            "登录已过期，请重新登录"
        );
    }
}
