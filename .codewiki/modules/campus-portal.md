---
title: 门户业务协议核心（campus-portal）
type: module
source_files:
  - crates/campus-portal/src/lib.rs
  - crates/campus-portal/src/client.rs
  - crates/campus-portal/src/parse.rs
tags:
  - portal
  - http
  - jwt
  - parse
  - timetable
---

# 门户业务协议核心（campus-portal）

`crates/campus-portal` 是协议单点第四员（与 [[modules/campus-auth|CAS 协议核心]] 平级，**无 Tauri 依赖**，安卓可复用）：以门户前端同款请求头组调用门户业务接口（学期 / 钱包卡 / 周课表），把响应解析成供 IPC 层透传的 DTO。协议细节（接口全表、请求头来源、校本作息实测）见 `docs/superpowers/plans/2026-09-18-m2-portal-pages.md` §1.1–§1.3。模块文档明示敏感纪律：网关 JWT 与邮箱 `loginUrl`（内含 authkey）只在内存使用，不落盘、不写日志、不进文档、不返回前端（`lib.rs:1-8`）。

## 会话复用方式（不重建登录）

- `PortalClient { cas: CasClient, auth: Arc<Mutex<Option<AuthHead>>> }`（`client.rs:38-45`）：内部持已登录 `CasClient`，clone 共享同一 Cookie jar——**不复制会话/登录管理**，这是计划 §2.2 的冻结约定。
- 挂载点：`CasSession.portal`（`tauri-app/src-tauri/src/infra/state.rs:16-21`），`restore_session` 与会话重建时 `PortalClient::new(client.clone())` 构造（`state.rs:109-121`）——缓存生命周期 = 会话生命周期，登录/登出/会话替换时整体随会话丢弃，无需单独失效逻辑；防御性 `clear_auth()`（`client.rs:71-76`）。
- JWT/资料内存缓存 `AuthHead { token_id, user_id, org_id }`（`client.rs:30-35`）：缓存命中直接用，未命中调一次 `portal_user_profile()`（tryLoginUserInfo）入缓存（`client.rs:78-97`）。**`AuthHead` 故意不派生 Debug**（JWT 字段防误记日志）；std Mutex guard 在 await 前 drop、不跨 await 持锁，锁中毒按缓存未命中处理。取不到 JWT（会话失效）→ `PortalError::NotLogin`（"请先登录"）。
- `org_id` 缺省回填 `"-1"`（学生实测值，`client.rs:90`）。

## 请求头方案（统一 helper）

私有 `get(endpoint)`（`client.rs:99-133`）统一注入计划 §1.1 全部头：`Authorization`（JWT，**无 Bearer 前缀**）、`loginUserId`/`loginUserName`（同为学号）、`loginUserOrgId`、`appid: ly-upp`、`csrfTimestamp`（毫秒时间戳）+ `csrfToken`（**直接复用 `campus_auth::cas::csrf_token`，不重写**）、`X-Requested-With`、`Accept`；每请求 10s 超时，URL 附 `_t` 时间戳防缓存。门户 base 复用 `campus_auth::cas::PORTAL_PROBE`（`cas.rs:23`，本批提升为 pub），不设第二事实来源。为此 `CasClient` 新增 `http_client()` getter（`cas.rs:134`）——campus-auth 仅有的两处改动即 `http_client` 与 `PORTAL_PROBE` pub。计划约定 POST 需另加 `Content-Type`；本批三接口均为 GET，POST helper 随批次 2 资讯/待办接口引入。

## 公开 API

`lib.rs` 导出（全部 camelCase serde）：

- **`PortalError`**：`Http(reqwest)` / `NotLogin("请先登录")` / `Parse(String)`——HTTP 失败以状态码文本归入 Parse 通道（`client.rs:121-124`），错误风格与 campus-auth 一致（中文 message、不 panic）。
- **`SemesterInfo`**：grade/semester/currentWeek/weekCount/startDate/endDate/currentWeekDay（服务端全字符串字段，透传不转数字）。
- **`WalletSummary`**：cardBalance / bookBorrowed / mailUnread（均可 Option——钱包卡单字段缺失按部分成功容忍）。
- **`CourseBrief`**：name / room / teachingClass / slot（1-based 大节号）/ startTime（`"HH:MM"`，查不到为 None）。
- **`WeekSchedule`**：解析后的周课表（矩阵 + 周次 + 星期）。
- 客户端三查询：`query_semester_info` / `query_wallet_summary` / `query_week_schedule`（`client.rs:134-146`），端点常量 `EP_SEMESTER` / `EP_WALLET_CARD` / `EP_WEEK_SCHEDULE`（cardId 为门户布局对该卡的固定分配，`client.rs:15-21`）。

## 解析纯函数（parse.rs）

解析全部为 `parse_*(body: &str) -> Result<T, PortalError>` 纯函数（网络与解析彻底分离，单测不碰网络）：

- `parse_semester_info`（`parse.rs:49`）：门户信封 `meta.success` 判定 + 逐字段校验。
- `parse_wallet_summary`（`parse.rs:78`）：钱包卡 `data.data` 是**内嵌 JSON 字符串**，二次 parse 后取数组首项；`loginUrl`（邮箱免密链接，内含 authkey，**等同凭据**）以「结构体不定义该字段」方式直接丢弃——绝不入内存结构之外的任何地方。
- `parse_week_schedule`（`parse.rs:123`）→ `WeekSchedule`。
- 「下一节课」链路：`elapsed_slot_count`（当前时刻已开始的大节数，`parse.rs:187`）→ `next_course`（矩阵内从第 N 大节起找，跳过进行中大节，`parse.rs:228`）→ `next_course_from_now`（含跨天与周末守卫，`parse.rs:257`）；单格拆分 `course_from_cell`（`"课程名,教室,教学班,姓名"` 四段，**第 4 段任课教师名按契约丢弃**，`parse.rs:199`）。
- 校本大节表 `block_time_slots()`（`parse.rs:160-182`）：5 大节 × 100 分钟——大节2 10:10、大节3 13:45 为日程服务实测，其余档位推算/占位待 M2.5 校准；**刻意不改 `campus-schedule::default_time_slots()`**（上游默认值被金标测试钉住）。大节语义与 14:50→15:35 真机修正的完整教训见 [[learnings/portal-block-periods-and-school-timetable|门户大节语义与校本作息]]。

## 测试（13 个，全离线脱敏 fixture）

`parse.rs:272-506`：学期解析全字段/缺字段报错；钱包卡首项解析 + 宽容类型 + 部分成功 + 错误路径；周课表矩阵与星期号宽容解析；`elapsed_block_count_by_time`；`next_course_splits_cell_and_drops_fourth_segment`（四段拆分与教师名丢弃）；**`next_course_maps_column_pair_to_block_start`**（回归钉「列对 → 大节起始时刻」口径，防 14:50 类误报复发）；进行中跳节与跨天；部分段容忍与未知大节；周末守卫。fixture 全部用占位值，真实姓名/学号/邮箱/authkey/JWT 不进测试样本。

接线方式见 [[modules/campus-hub-tauri|接线层]] 的 `get_portal_overview`（子字段失败互不阻塞）。
