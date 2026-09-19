---
title: 接线层（campus-hub src-tauri）
type: module
source_files:
  - tauri-app/src-tauri/src/lib.rs
  - tauri-app/src-tauri/src/commands/auth.rs
  - tauri-app/src-tauri/src/commands/profile.rs
  - tauri-app/src-tauri/src/commands/portal.rs
  - tauri-app/src-tauri/src/commands/timetable.rs
  - tauri-app/src-tauri/src/commands/electricity.rs
  - tauri-app/src-tauri/src/commands/electricity_history.rs
  - tauri-app/src-tauri/src/commands/mod.rs
  - tauri-app/src-tauri/src/infra/state.rs
  - tauri-app/src-tauri/src/infra/timetable.rs
  - tauri-app/src-tauri/src/infra/electricity_history.rs
  - tauri-app/src-tauri/src/infra/mod.rs
  - tauri-app/src-tauri/src/account/crypto.rs
  - tauri-app/src-tauri/src/account/store.rs
  - tauri-app/src-tauri/tauri.conf.json
  - tauri-app/src-tauri/capabilities/default.json
tags:
  - tauri
  - ipc
  - dpapi
  - session
  - login
  - avatar
  - portal
  - opener
  - apps
  - schedule
  - electricity
  - history
  - m4
---

# 接线层（campus-hub src-tauri）

`tauri-app/src-tauri`（crate 名 `campus-hub`）是协议核心与前端之间的 IPC 接线层：命令面、AppState、DPAPI 持久化。协议逻辑零实现——「协议核心全部在 campus-auth crate，本 crate 只做 IPC 接线与本地持久化」（`src/lib.rs:2`）。登录/账号命令定义在 `src/commands/auth.rs`，头像/资料命令定义在 `src/commands/profile.rs`，门户数据命令定义在 `src/commands/portal.rs`，全部注册于 `lib.rs:19-45`。

## 命令面（下表覆盖 auth / profile / portal / timetable；慧新E校与电费的 31 条见 `modules/campus-synjones.md` 与下方 M4 小节；**全量 66 条**的命令面总账在根 `_architecture.md`「命令面与模块地图」）

| 命令 | 参数（camelCase） | data 形态 | 位置 |
|---|---|---|---|
| `get_captcha` | — | `{ uid, pngBase64 }`（pngBase64 为裸 base64，前端自行拼 data URI） | `auth.rs` |
| `login` | `account: { username, password }` | 成功 `{ username, displayName }`；三次识别穷尽 `{ uid, pngBase64 }` + message=`CAPTCHA_MANUAL` | `auth.rs` |
| `login_manual` | `account: { username, password, captchaUid, captchaCode }` | 同 login（手动单次提交不重试） | `auth.rs` |
| `login_saved` | `account: { username }` | 同 login（DPAPI 解密已存密码后复用自动流程） | `auth.rs` |
| `check_session` | — | `{ loggedIn: bool }` | `auth.rs` |
| `logout` | — | 无（`CommandResult::empty()`） | `auth.rs` |
| `list_accounts` | — | `{ accounts: [{ username, lastLogin, displayName? }] }`（密码绝不出现在返回里） | `auth.rs:479-494` |
| `remove_account` | `username: String` | 无（复用 `store::remove_account`，错误原文中文透传，`auth.rs:498-504`） | `auth.rs:498-504` |
| `get_avatar` | — | `{ imageBase64: string\|null, source: "local"\|"official"\|null }` | `profile.rs:162-167` |
| `set_avatar` | `imageBase64: String` | 同 AvatarData（空串/超 2MB → err） | `profile.rs:169-180` |
| `clear_avatar` | — | 同 AvatarData（只清本地，官方保留） | `profile.rs:182-191` |
| `sync_official_avatar` | — | 同 AvatarData（无会话 → err「请先登录」） | `profile.rs:193-209` |
| `upload_official_avatar` | `imageDataUrl: String` | 同 AvatarData（无会话 → err「请先登录」；data URL 非法/超 200KB → err；上传成功后重拉官方头像落盘，2026-09-18 新增） | `profile.rs:219-262` |
| `get_portal_overview` | — | `PortalOverview{ semester, wallet, nextCourse, fetchedAt }`，三个子项均可 null（无会话 → err「请先登录」；2026-09-18 M2 批次 1 新增） | `portal.rs:23-79` |
| `get_info_columns` | — | `InfoColumn[]`（后端固定 7 栏：订阅接口 + 实测全量兜底） | `portal.rs:86-97` |
| `get_info_list` | `columnId, page, pageSize` | `InfoPage`（total/pageCount 不可靠原样透传，前端满页判断分页） | `portal.rs:101-115` |
| `get_info_detail` | `url` | `InfoDetail{ title, html?, needsBrowser, url }` 三分类（正常 HTML / 鉴权门 `needsBrowser=true` 非错误 / 真错误；正文已由协议层白名单清洗，命令层不二次处理） | `portal.rs:119-131` |
| `get_todo_tabs` | — | `TodoTab[]`（接口 6 tab 全量透传，前端按契约展示三个） | `portal.rs:135-146` |
| `get_todo_list` | `tabId, page, pageSize` | `TodoPage`（tabId 白名单校验在协议层 `query_todo_list`） | `portal.rs:150-164` |
| `open_in_browser` | `url` | 无（白名单强制 `*.cwxu.edu.cn`，非法域名 err「仅支持校园官网链接」；2026-09-18 M2 批次 2 新增） | `portal.rs:183-192` |
| `get_app_catalog` | — | `AppCatalog{ groups, pinned }`（图标已由后端代拉为 data URL，失败条目 iconUrl 为 null；2026-09-18 M2 批次 3 新增） | `portal.rs:199-210` |
| `get_schedule_classify` | — | `ScheduleClassify[]`（5 类，会话内后端已缓存） | `portal.rs:214-225` |
| `get_schedule_month` | `startMs, endMs, codes` | `ScheduleEvent[]`（区间倒挂 err「日程区间无效」，前端 bug 防御；**M2 遗留项起 codes 含 `Default-Meeting` 时并入会议卡日程**——失败空贡献不影响课表，失败时 stderr 有 `[meeting-diag]` 打点） | `portal.rs:229-261` |
| `get_schedule_day_counts` | `startMs, endMs` | `ScheduleDayCount[]`（月视图角标；bs-schedule 计数接口无分类参数，计数为当日全量日程数；2026-09-18 M2 遗留项新增，命令数 24 → 25） | `portal.rs:264-281` |
| `open_app` | `url, isCas` | 无（**协议白名单** `is_http_url` 仅 http/https，非法 err「仅支持 http/https 链接」；`isCas` 契约保留字段、当前不影响打开策略——可达性提示由前端按 `AppItem.access` 分级给出） | `portal.rs:284-301` |
| `get_timetable` | — | `TimetableView{ timetable, slots, currentWeek, today }`（**纯本地读取，无网络、无需登录态**：读 `timetable.json`，缺失/损坏 → 空课表 `courses: []` 不报错；`slots` = `effective_slots_at(config, today)`（`config.slots` 自定义优先、回落内置 `campus_portal::block_time_slots()`，批 2 2026-09-19 加 date 参数预留 P3 区间命中、本批不消费；收尾轮单点化）**下发给前端做时间标签唯一事实源**、`currentWeek` = `weeks::current_week`（无开学日/今天越出学期为 null）、`today` = "YYYY-MM-DD"；组装纯函数 `build_timetable_view` 可单测。批次 1 原返回裸 `Timetable`，2026-09-18 批次 4 前修订（契约 §2.3），命令数 25 → 26） | `timetable.rs:36-71` |
| `import_timetable` | — | `ImportResult{ added, changed, removed, total, changes }`（链路见下节；M2.5 批次 2 新增，命令数 26 → 27） | `timetable.rs:75` |
| `add_course_manual` | `input: ManualCourseInput{ name, teacher, position, day, startSection, endSection, weeks, colorIndex, remark? }` | `Course`（source=Manual、id=`manual-<纳秒>`；入参校验：课程名/星期/节次/周次，M2.5 批次 2） | `timetable.rs:203` |
| `update_course` | `course: Course` | `Course`（按 id 整条替换，id 不存在 err「课程不存在」；任意来源可编辑，M2.5 批次 2） | `timetable.rs:238` |
| `delete_course` | `id: String` | 无（**级联清理**该课程挂载的 override；不存在 err「课程不存在」，M2.5 批次 2） | `timetable.rs:256` |
| `export_ics` | — | `String`（**写入用户下载目录 `课表.ics`（已存在覆盖）并返回写入的完整路径**——2026-09-18 真机验收发现 WebView2 不处理下载、前端 Blob 交付不可用，改由后端落盘；见下节，M2.5 批次 2） | `timetable.rs:429` |
| `parse_notice` | `text: String` | `NoticeCandidate[]`（L1/L2 解析**不入库**，语义见 [[modules/campus-schedule\|课表核心]] notice 节；本地课表 + `current_week` 现算传入；M2.5 批次 3，命令数 31 → 34） | `timetable.rs` |
| `apply_override` | `candidate: NoticeCandidate` | `CourseOverride`（校验 courseId 落在本地课程，缺失/已删 err「通知未匹配到本地课程」；字段级拷贝写 overrides，`autoApplied` = confidence==High；**同一 noticeId+courseId 重复采纳幂等覆盖**；M2.5 批次 3） | `timetable.rs` |
| `revoke_notice` | `noticeId: String` | `u32`（按 `source_notice_id` 整批删除 override，返回条数，0 条幂等成功；M2.5 批次 3） | `timetable.rs` |
| `save_time_slots` | `slots: Option<Vec<TimeSlot>>` | `TimetableView`（保存自定义作息/`null` 恢复内置；校验：≥1 条、≤20、number 正整数严格递增、HH:MM 且 end>start；返回刷新视图免二次拉取。M2.5 收尾轮 2026-09-18 新增，命令数 34 → 35；取舍见 [[decisions/timetable-editable-slots\|作息时间表可编辑]]） | `timetable.rs` |
| `save_semester_config` | `SemesterConfigInput` | `TimetableView`（开学日/总周数/周首日/显示周末整块保存；`currentWeekHint` 由后端 `semester_start_from_week` 反推开学日；`apply_display_constraints` 双向联动收口后端单点。批 1 2026-09-19 新增，契约 §7.1/§7.2） | `timetable.rs` |
| `save_skipped_dates` | `dates: Vec<NaiveDate>` | `TimetableView`（跳过日期**整体替换**，落库前升序+去重归一；日期合法性由 NaiveDate 反序列化保证。批 2 2026-09-19 新增，契约 §8.2） | `timetable.rs` |

约定：业务失败一律 `Ok(CommandResult::err(中文消息))`，`Err(String)` 仅限 IPC 框架层错误（`auth.rs` 注释冻结此口径）。头像五命令统一返回 `AvatarData`（键恒在、值可 null，`profile.rs:33-38`）。

## 门户总览命令（`commands/portal.rs`，M2 批次 1）

`get_portal_overview`（`portal.rs:44-79`）聚合 [[modules/campus-portal|门户业务协议核心]] 的三个接口（学期 / 钱包卡 / 本周课表），协议细节全部下沉 campus-portal，本命令只做接线：

- **子字段失败互不阻塞**：三个查询各自 `.ok()` 置 null，任一失败不影响其余（前端回落空态/"—"，不整页报错）；`nextCourse` 由 `next_course_from_now` 从周课表推算，无课/失败为 null（前端隐藏横幅）。
- **无会话守卫**：锁内 clone `session.portal`（Arc 包装廉价，guard 在 await 前 drop）后取数，`None` → err「请先登录」（`ERR_NO_SESSION`，与 profile.rs 同口径）。
- 敏感纪律：JWT 与邮箱 `loginUrl` 在 campus-portal 内部消化，本命令只透出钱包数字与课程简报，不含任何凭据字段（`portal.rs:1-6` 模块文档）。

## 资讯/待办命令与浏览器打开（M2 批次 2，2026-09-18）

六个新命令与 `get_portal_overview` 的聚合模式不同，全部**单接口透传**：统一经 `portal_of` helper（`portal.rs:79-82`，锁内 clone portal 后取数）+ 协议错误 `e.to_string()` 映射为中文 message，无会话 → err「请先登录」（`ERR_NO_SESSION`，与 profile.rs 同口径）。

- **`open_in_browser`**：系统浏览器打开 URL，走官方 `tauri-plugin-opener` 的 Rust API（`lib.rs:16-18` 注册插件；只在 Rust 侧调用，**不开放前端直接 invoke 插件命令，无需额外 capability**）。白名单校验收敛在 `pub(crate) open_url_in_browser` helper 内部第一行（`portal.rs:173-180`，复用 [[modules/campus-portal|门户业务协议核心]] 的 `is_allowed_info_url`——**与正文抓取同一事实来源**），非法域名 err「仅支持校园官网链接」；批次 3 `open_app` 复用其打开方式但校验换成协议白名单（见下节，两种校验语义不同）。
- **`get_info_detail`**：透传协议层三分类（正常 HTML / `needsBrowser=true` 引导浏览器 / 真错误），`needsBrowser` 是正常返回非错误；正文已由 campus-portal 白名单清洗，命令层不做二次处理。背景见 [[learnings/cwxu-official-site-content-extraction|官网正文抓取与鉴权门降级]]。
- **CSP 配套**：`tauri.conf.json:26` 的 `img-src` 在 `'self' data:` 基础上增加 `https://*.cwxu.edu.cn http://*.cwxu.edu.cn`——内嵌正文官网图片显示的必要配套，域名仍限校园官网。

## 应用/日程命令与 open_app（M2 批次 3 + 遗留项会议并入）

四个新命令（`portal.rs:194-301`）延续批次 2 的单接口透传模式（`portal_of` helper + 无会话 err「请先登录」）；遗留项批次补 `get_schedule_day_counts`（命令数 24 → 25）并扩展 `get_schedule_month`：

- **`get_app_catalog`**：透传协议层 `AppCatalog{groups, pinned}`，图标 data URL 已在协议层代拉拼好，命令层零处理；`AppItem.access` 可达性分类由协议层按附录 A 实测表推导，命令层透传。
- **`get_schedule_classify` / `get_schedule_month`**：日程分类与区间明细透传；`get_schedule_month` 对 `endMs <= startMs` 直接 err「日程区间无效」（前端 bug 防御，不透传服务端）。**会议并入（M2 遗留项）**：课表明细 Ok 时 `extend(query_meetings_for_range(...))`——codes 含 `Default-Meeting` 才并入；会议链路任一环节失败为空贡献（**降级承诺：不影响课表日程与日历**），失败环节在协议层留 `[meeting-diag]` stderr 打点（只打失败，成功静默），命令层零打点。
- **`get_schedule_day_counts`**（遗留项新增）：月视图角标取数，区间倒挂防御同上；透传 bs-schedule `getCountBetweenTime`（**无分类参数，计数为当日全量**）。
- **`open_app(url, isCas)`**：校验用**协议白名单** `campus_portal::is_http_url`（仅 http/https），而非 `open_url_in_browser` 的域名白名单——该 URL 来自校方应用目录（受信来源）、后端不抓取它（无 SSRF 面）、只在系统浏览器打开；实测 30 条目录数据中 16 条为非校园域，域名白名单会把学校自己的合法应用全部拦掉。打开动作复用官方 `tauri-plugin-opener` Rust API。**`isCas` 是契约保留字段，当前不影响打开策略**（`let _ = is_cas`）——可达性提示由前端按 `AppItem.access` 分级给出（webvpn 提示后仍打开 / unavailable 只提示不打开）。⚠️ WebVPN B 类包装未实现（实测网关对未登录请求一律回落、明文包装无法验证，会话打通 + 包装 + A 类 CAS 直达签发归 M4）。分工原则与数据分布见 [[learnings/portal-app-catalog-and-icons|应用目录、图标代拉与 appLink 校验分工]]。

## 课表命令与导入/ICS/调课通知链路（`commands/timetable.rs`，M2.5 批次 1+2+3）

课表命令不依赖 `portal_of` 模式：`get_timetable`/`export_ics`/手动课程三命令/调课通知三命令/`save_time_slots` 是**纯本地操作**（不取 State，直接 `state::data_dir()`）；只有 `import_timetable` 需要会话——锁内 clone `(client, tgt, portal)` 三件套后 drop guard 再 await。

- **`import_timetable` 链路**（`timetable.rs:75-150`）：门户学期信息（会话内已缓存）推导 `xnm`/`xqm`（冻结契约 §1.2 口径：`xnm`=`start_date` 前 4 位、`semester` `"1"→3/"2"→12`，**不用 `grade`**）→ `fetch_timetable_json`（901→TGT 静默重进在 campus-auth 内部；失败 Display 中文直接透出，`JwglNotLogin` =「教务会话已失效，请重新登录」）→ `parse_kb_response(json, DEFAULT_TABLE_ID)` → [[modules/campus-schedule|课表核心]] `diff_courses` 合并旧库 → 落库 → `ImportResult{added, changed, removed, total, changes}`（total = 合并后课程总数，含停开保留记录）。学期信息同时初始化/更新 `semester_start_date`（`"YYYYMMDD"`→`NaiveDate`）与 `semester_total_weeks`，**单字段解析失败保留旧值**（不因坏数据丢课表）。
- **手动课程三命令**共用 `mutate_timetable`（load → 改 → save 骨架，`timetable.rs:165`）；无进程内互斥（前端交互串行，契约 §2.2 原子性由调用方保证）。`add_course_manual` 入参校验（课程名非空/星期 1-7/节次 start≤end/周次非空且 ≥1）后构造 `source=Manual` 课程，id=`manual-<纳秒时间戳>`（与导入 id `<table_id>-<jxb_id>` 前缀不同永不冲突，取舍见 [[decisions/timetable-diff-manual-and-ics|课表 diff、手动课程与 ICS 导出决策]]）。
- **`export_ics`**（`build_ics`，`timetable.rs:299`）：展开式 VEVENT（不依赖 RRULE）——每门未停开课程经 `campus_schedule::expand_occurrences`（批 2 2026-09-19 重写，决策 5 / 契约 §8.5，见 [[decisions/timetable-occurrence-expansion|课表生效实例展开]]）展开其每个教学周的**生效实例**，迭代周次 = `course.weeks ∪ 各 override.weeks`（复核 P1-b：补课周可不属于 course.weeks），只消费 `Solid`：停课不生成 VEVENT、调课原时段消失而新时段生成、补课新增、DESCRIPTION 追加「调课/补课」（复核 P3-b：经预建的 id→类型映射溯源，防逐事件 find 张冠李戴）；UID `{id}-w{week}d{day}s{start}@campushub`（复核 P3-a）：原位实例（含仅换教室）与旧版逐字节一致，**调课新位与补课实例追加 `-o{override 短 id}` 后缀**防同位撞 UID；VEVENT 日期命中 `config.skipped_dates` → 跳过；时刻取值 = **该日** `effective_slots_at(config, date)`（大节号 = `(起始小节+1)/2`，查不到跳过该实例），custom 课（节次 None）未被调整时 DTSTART/DTEND 直取 `custom_start_time/custom_end_time`（UID 用 `scustom` 段），被 resched 时新位走大节表（P3-c 口径）；周首日对齐沿用契约 §7.3（第 1 周首日 = 开学日按 first_day_of_week 回退，col = `(day-firstDay+7)%7`）；TEXT 转义（`,` `;` `\` 换行）+ CRLF 行尾；floating local time（无 `Z`/`TZID`，RFC 5545 合法、Outlook/Google 按导入时区解释）；缺 `semester_start_date` err「请先完成一次导入」。生成后由 `write_ics_to`（`timetable.rs:429`）写入 `dirs::download_dir()` 下的「课表.ics」——**覆盖写**（无时间戳后缀）、失败透出系统错误、取不到下载目录 err 中文提示；命令返回写入的完整路径，前端只展示（WebView2 不处理下载，交付禁走 Blob，见 [[learnings/tauri-webview-ui-verification|真机 UI 验收路径]]）。
- **调课通知三命令**（M2.5 批次 3）：`parse_notice(text)` 本地课表 + `chrono::Local::now()` 现算 `current_week` 传入 `campus_schedule::parse_notice_text`（解析语义见 [[modules/campus-schedule|课表核心]]，取舍见 [[decisions/timetable-notice-l1l2|调课通知 L1/L2 分级口径与 noticeId 取舍]]），**不入库**；`apply_override(candidate)` 经 `candidate_to_override`（字段级拷贝 + `auto_applied`=High，纯函数与单测共用）写 overrides，`upsert_override` 以 noticeId+courseId 幂等覆盖、不同课程并存；`revoke_notice(noticeId)` 按 `source_notice_id` 整批删除返回条数。三者同样走 `mutate_timetable` 骨架、纯本地无会话。

## 电费历史与绑定宿舍命令（`commands/electricity_history.rs`，M4 批 2，2026-09-19）

与 `commands/electricity.rs`（片区/级联/常用房间/充值六条）**分文件**以免单模块膨胀；两者共用同一份常用房间存储与同一个进程级 synjones 客户端（token 单活，绝不自建第二套）。协议侧只读事实见 [[modules/campus-synjones|慧新E校协议核心]]，存储与合并语义见 `infra/electricity_history.rs` 模块头注与 [[decisions/electricity-daily-snapshot-and-merge|电费日快照与多端合并决策]]。

| 命令 | 参数（camelCase） | data 形态 | 说明 |
|---|---|---|---|
| `get_electricity_bills` | `page?`, `size?`, `feeitemId?` | `BillPage{ total, records }` | 缴费账单（**金额单位元**，不要再除 100）；`total` 是全量条数不是本页条数 |
| `get_electricity_monthly` | `year?` | `MonthTotal[12]` | **串行发 12 个请求**（空月 = 0），期间持全局锁 ⇒ 前端要独立三态、别叠着别的请求发 |
| `get_electricity_orders` | `status?`（0 待支付/1 已完成/None 全部） | `Order[]` | **含待支付**（`personal_data` 头组齐备即可，旧「恒 500」结论已推翻）；待支付单的 `orderId` 交给已有的 `recharge_status` / `recharge_cancel` 处理，本模块**不新增任何写路径** |
| `get_electricity_history` | `roomId?`（`SavedRoom.id`，None=全部房间）, `days?`（含今天；上限 3650） | `HistoryEntry[]`（**时间升序**） | 纯本地读，不需要会话；`roomId` 用常用房间 id 而非内部 `roomKey`（后者不下发 UI）；`days` 有上限是因为 `NaiveDate - Duration` 越界会 panic |
| `bind_electricity_room` | `id`, `bound` | `SavedRoom[]` | 绑定「我的宿舍」（**最多一个**，绑新的自动解绑旧的；`set_bound` 保证）；保存房间（`save_electricity_room`）**沿用存量绑定**，不因前端没带 `bound` 而静默丢绑定 |
| `run_electricity_snapshot` | — | `SnapshotOutcome{ entry, replaced }` | 对**绑定的**房间跑一次级联查询并落盘；未登录/未绑定返回**可读中文原因**（不 panic） |

**启动补采**（`lib.rs` 新增 `.setup()`）：`tauri::async_runtime::spawn` 一个后台任务，三道静默护栏——今日已采过 / 未绑定宿舍 / **当前无内存会话**（无会话时不进 SSO，避免后台补采签发 token 顶掉用户正在用的会话）⇒ 都不满足才采一次。不弹窗、不阻塞启动，失败只 `log::warn`；日志只含错误文案与余额数值，**无 token / 账号 / 户号**。

**采集策略取舍**：不注册 Windows 计划任务（token 单活会被后台采集顶掉）⇒ 应用没开的日子就是空档，图表留空（不插值、不补零）。

## 登录重试状态机

`run_login` 内核三命令共享（`auth.rs:220-297`），登录用**全新 CasClient**（干净 jar，避免旧会话 cookie 干扰，`auth.rs:228`），密码仅在内存中存续、立即 RSA 加密（`auth.rs:230`）。

- **自动模式**（`auth.rs:240-296`）：`kaptcha → captcha::solve → login` 循环，总提交 ≤`MAX_LOGIN_ATTEMPTS=3`（`auth.rs:27`）。重试决策纯函数 `should_retry`（`auth.rs:206-208`）：**仅 `WrongCaptcha` 重试**；`WrongUserOrPwd`（防 CAS 连续错误计数锁号）、`UserLocked`、`NeedTwoVerify`、`Unknown`、`Network` 一律立即终止（单测 `auth.rs:550-570`）。识别失败（solve 返回 None）未提交到 CAS、不消耗连续错误计数，刷新重试不计提交（`auth.rs:252-259`）。
- **穷尽转手动**（`auth.rs:275-294`）：返回 `success:false + message="CAPTCHA_MANUAL" + data={uid, pngBase64}`，前端据此切手动模式；穷尽前最后一次提交的 uid 可能已被 CAS 消耗，故先尽力刷新一张新验证码图、失败用最后一张兜底。
- **手动模式**（`auth.rs:233-239`）：单次提交不自动重试，验证码错误由用户刷新重输。
- **接近阈值保护**：`parse_error_count_hint` 从 TWOVERIFY 的 data 原文解析「已连续错误N次，阈值M」（`auth.rs:173-190`）；`n+1 >= m` 时错误消息明确提示已停止自动重试。

## LoginResultData：untagged 双形态

`#[serde(untagged)] enum LoginResultData { LoggedIn(LoginData), ManualNeeded(CaptchaData) }`（`auth.rs:101-105`）——序列化时内联，前端拿到裸对象：成功 = `{ username, displayName }`，穷尽 = `{ uid, pngBase64 }`。前端按字段收窄：`isLoginOk`（`"username" in data`）/ `isCaptchaPayload`（`"uid" in data`）类型守卫（`authStore.ts:36-43`）。

## 登录成功收尾（`finish_login` `auth.rs:300-351`）

`sso_follow(PORTAL_SERVICE)` 建门户会话 → **尽力取门户真实姓名**（`portal_user_profile`，失败只 warn 并回退上次已存 displayName、再回退学号——save_account 是整条 upsert，直接传 None 会把旧 displayName 抹掉，`auth.rs:312-330`）→ `save_account`(DPAPI，失败不阻断会话、降级日志 `auth.rs:332-334`) → `jar.snapshot()` + **`ok.tgt` 逐项 DPAPI 加密写 session.json**（`auth.rs:337-341`；TGT 必须随会话持久化——CAS 不种登录 cookie，它是教务会话静默续期唯一凭据，`login_saved` 免密重登走同一内核自动获得同样落盘）→ AppState 锁内同步赋值（guard 语句末 drop、此后无 await，`auth.rs:343-350`，`CasSession.tgt = Some(ok.tgt)`）。`displayName` 无来源时用 username（`auth.rs:352`）。姓名不进日志（敏感纪律）。

`check_session` 返回 false（无会话/已过期/探测失败）时清 AppState 会话与 session.json（`auth.rs:450-455`）。`logout`：`GET {CAS_BASE}/logout` 尽力而为（5s 超时，复用 `sso_follow` 发 GET；REST 登录无 CASTGC，服务端可能本就无全局会话），随后本地清理必然执行（`auth.rs:461-475`）。`session_client` 是取会话 client 的唯一入口（锁内 clone，`auth.rs:357-364`），profile 模块的 `sync_official_avatar` 同样取用。

## DPAPI 裸 FFI（`account/crypto.rs`）

Windows `CryptProtectData` / `CryptUnprotectData`（CurrentUser 作用域，跨用户不可解密）以 `extern "system"` + `#[link(name = "crypt32")]` 裸 FFI 实现（`crypto.rs:16-37`），**零新依赖**（不引入 windows crate），拷贝自参考项目 Wxxy-CampusLogin 同名模块（`crypto.rs:3-4`）。要点：

- `call_dpapi` 通用 helper 统一 DataBlob 构造、结果检查与 `LocalFree` 释放；失败路径判空后同样释放输出缓冲（`crypto.rs:46-78`）。
- 对外只暴露两个字符串接口：`dpapi_protect(明文) -> DPAPI 密文 base64`、`dpapi_unprotect(密文 base64) -> 明文`（`crypto.rs:121-139`）。
- 非 Windows 编译期桩返回 Err「加密存储仅桌面端支持」（`crypto.rs:142-150`；安卓阶段 2 换 Android Keystore）。

## 存储位置与格式（`%APPDATA%/campushub/`，`infra/state.rs:34-38`）

| 文件 | 结构 | 写入方 |
|---|---|---|
| `session.json` | `{ username, cookies: [{ name, valueB64(DPAPI) }], tgtB64?(DPAPI) }`（`state.rs:63-70`；`tgtB64` 为 M2.5 批次 1 新增，缺省/None 时省略——CAS TGT 是教务会话静默续期唯一凭据，DPAPI 密文落盘、绝不落明文） | `persist_session(dir, username, cookies, tgt)`（`state.rs:82-101`）；读 `load_session` → `StoredSession{ username, cookies, tgt }`（单个 cookie/TGT 解密失败跳过为 None，`state.rs:107-124`）；删 `clear_session` |
| `accounts.json` | `{ accounts: [{ username, passwordB64(DPAPI), lastLogin(epoch 毫秒串), displayName? }] }`（`store.rs:13-29`） | `save_account`（同 username upsert 覆盖，`store.rs:43-62`）、`remove_account`（不存在报错，`store.rs:81-89`） |
| `profile.json` | `{ localBase64?, officialBase64?, officialFetchedAt? }`（camelCase，字段缺省即不存在，`profile.rs:41-52`）——**明文 base64，不走 DPAPI** | `store_local_avatar` / `clear_local_avatar` / `store_official_avatar`（`profile.rs:127-153`）；读 `read_profile`（文件缺失/损坏按空档处理，`profile.rs:62-67`） |
| `timetable.json` | `campus_schedule::Timetable`（camelCase：`config/courses/overrides/updatedAt`）——**非凭据明文**，与 profile.json 同级；`infra/timetable.rs`（M2.5 批次 1 新建）：`load_timetable`（缺失/损坏 → 空课表不报错、不删坏文件，`timetable.rs:41-54`）、`save_timetable`（整体读写，原子性由调用方保证——冻结契约 §2.2 单文件无数据库，`timetable.rs:56-60`）、`empty_timetable`（`DEFAULT_TABLE_ID="default"`，`timetable.rs:24-36`） | `import_timetable`（M2.5 批次 2）与手动课程三命令写入 |
| `electricity_rooms.json` | `SavedRoom[]`（camelCase：`id/feeitemId/feeitemName/path/label/bound`）——非凭据明文；`id` = 本机 epoch 毫秒串（**撞号顺延**，见 [[learnings/history-dedupe-not-by-adjacent-sort\|历史去重不能靠排序后看相邻]]），`bound` = 「我的宿舍」（`serde(default)` 兼容旧文件，最多一个）；读写在 `commands/electricity.rs`（`load_rooms` / `write_rooms`，缺失/损坏 → 空列表不删坏文件） | `save_electricity_room` / `delete_electricity_room`（M3 批 3）、`bind_electricity_room`（M4 批 2） |
| `electricity_history.json` | `HistoryEntry[]`（camelCase：`id/device?/roomKey/roomName/feeitemId/feeitemName/collectedAt/date/balance/raw/source`）——非凭据明文，**不含户号**（`map.data` 的 PII 在 crate 层就不透出）；`infra/electricity_history.rs`（M4 批 2 新建）：`load_history`（缺失/损坏 → 空列表、不删坏文件）、`save_history`（先 `normalize` 再整体写）、`normalize`（按 id 去重 → 时间升序 → 上限 `MAX_HISTORY=2000` 丢最旧）、`merge_history`（多端合并，可交换 + 幂等） | `run_electricity_snapshot` / 启动补采（M4 批 2） |

启动回填 `restore_session()`（`state.rs:137-153`）：`run()` 在 `manage` 之前调用（避免 setup 内碰 tokio Mutex，`lib.rs:11-13`），读 session.json → 解密 → `jar.restore` 回填，**TGT 一并回填 `CasSession.tgt`**（旧格式文件无 tgtB64 → None，教务 901 时上层直接引导重新登录）；文件缺失/损坏/cookies 空 → None。落盘内容不含 cookie/TGT 明文有单测断言（`state.rs:168-196`，含旧格式兼容 `state.rs:198-224`）。`CasSession` 自 2026-09-18 起挂 `portal: PortalClient`（M2 批次 1）与 `tgt: Option<String>`（M2.5 批次 1，`state.rs:16-29`，仅内存明文、与 cookie 同级敏感）——`finish_login`（`auth.rs:343-350`）与 `restore_session`（`state.rs:144-152`）两处构造均 `PortalClient::new(client.clone())` 共享同一 jar，缓存生命周期 = 会话生命周期（详见 [[modules/campus-portal|门户业务协议核心]]）。

## 头像存取、官方同步与上传学校（`commands/profile.rs`）

头像命令全部只做接线与本地存取，协议拉取/上传复用 `CasClient` 的门户资料接口（见 [[modules/campus-auth|CAS 协议核心]]「门户资料接口」）：

- **生效优先级：本地 > 官方 > 无**，纯函数 `current_avatar` 统一裁决（`profile.rs:78-97`）；`clear_avatar` 只清本地、官方保留（回落展示，`profile.rs:182-191`）。
- **落盘即明文**：头像不是凭据，base64 明文写 `profile.json`，不经 DPAPI（`profile.rs:5-6` 注释；取舍见 [[decisions/guest-mode-account-shell|游客优先与账号外壳决策]]）。
- **本机体积守卫**：`set_avatar` 空串/超 `AVATAR_MAX_B64=2MB` 拒绝，冻结文案「本机头像过大（上限 2MB）」（`profile.rs:23,98-105`；2026-09-18 由 512KB 放宽到 2MB，前端裁切器按同阈值预检）。
- **`sync_official_avatar`**：无会话 → 约定错误文案「请先登录」（`ERR_NO_SESSION`，前端据此引导登录，`profile.rs:28,195-199`）；有会话 → 锁纪律 `session_client` clone 出 client 后发请求，成功落盘 `officialBase64 + officialFetchedAt`，网络/解析失败不落盘、不清已有头像（`profile.rs:193-209`）。
- **`upload_official_avatar(imageDataUrl)`**（2026-09-18 新增，`profile.rs:219-262`）：把裁切后的头像上传回学校系统。链路：无会话直接 err「请先登录」→ `validate_official_data_url` 校验（须 `data:image/` 开头、剥前缀后裸 base64 ≤ `OFFICIAL_AVATAR_MAX_B64=200KB`，`profile.rs:26,108-122`——服务端**原样存储不压缩**，守卫只能本端做）→ `CasClient::portal_change_portrait` 上传（内部现取 JWT/ids 并现算 csrf，见 [[learnings/portal-avatar-upload-protocol|门户头像上传协议]]）→ 成功后重新 `portal_login_info` 拉官方头像落盘 → 返回最新 `AvatarData`（以服务端回读为准）。日志只打码用户名（`profile.rs:264-273`，与 `auth.rs::mask_username` 同款），**绝不打印 data URL**（体积可达数百 KB）。
- 单测覆盖优先级轮转（官方 → 本地覆盖 → 清本地回落官方 → 全空双 null）、2MB/200KB 校验与冻结文案、data URL 非法形态、无会话文案契约（`profile.rs:275-389`）。

## 最小权限

`capabilities/default.json` 仅声明 `permissions: ["core:default"]`、仅 `main` 窗口——不给多余能力。opener 插件（`tauri-plugin-opener = "2"`）只在 Rust 侧经 `OpenerExt` 调用（`portal.rs:176-178`），不开放前端直接 invoke 插件命令，故 capabilities 无需追加条目。CSP（`tauri.conf.json:26`）：`connect-src 'self' ipc://localhost`；`img-src 'self' data:` 之上，M2 批次 2 为内嵌正文官网图片增加 `https://*.cwxu.edu.cn http://*.cwxu.edu.cn`（域名仍限校园官网）。

## 离线单测（`commands/auth.rs:506-626` + `commands/profile.rs:275-389`）

错误码→中文消息映射、重试状态机、计数提示解析与接近阈值文案、`login_saved` 解密失败路径（坏密文/账号不存在，不发起网络请求）、用户名打码（`mask_username`：前 2 位 + 末位，`auth.rs:157-165`）；profile 侧头像优先级轮转、2MB/200KB 体积校验（含 data URL 非法形态）、无会话文案契约；state 侧 session 往返（含 TGT DPAPI 密文落盘断言、TGT 缺省与旧格式兼容，`state.rs:166-224`）；timetable 存储往返 + 缺失/损坏回空（`timetable.rs:70-149`）；**M4 批 2 新增**：`electricity_history.json` 往返/损坏回空/同日同房间覆盖/上限丢最旧/`merge_history` 可交换且幂等/`roomKey` 稳定性与过滤窗口（`infra/electricity_history.rs`），`build_entry` 用三片区 live 原文钉余额提取与 `id` 推导、末级 `tipinfo` 拒绝采集、绑定唯一性与「保存房间不丢绑定」、`SavedRoom.id` 撞号顺延（`commands/electricity_history.rs` + `commands/electricity.rs`）。

**当前全量**：`cargo test --workspace` = **322 passed / 0 failed / 13 ignored**（2026-09-19 M4 批 2 实跑；分目标 campus-auth 34、campus-hub 110、campus-portal 54、campus-schedule 55、campus-synjones 66 + 集成 3；13 个 ignored 全是需校园网/真机凭据的 live 测试，**开发期不跑**——会顶掉用户正在用的会话）。更早的分批计数（M2.5 批次 3 的 146/4）已随历次批次累加，以本次为准。
