---
title: 课表核心（campus-schedule）
type: module
source_files:
  - crates/campus-schedule/src/model.rs
  - crates/campus-schedule/src/weeks.rs
  - crates/campus-schedule/src/grid.rs
  - crates/campus-schedule/src/timeslots.rs
  - crates/campus-schedule/src/zhengfang.rs
  - crates/campus-schedule/src/diff.rs
  - crates/campus-schedule/src/holiday.rs
  - crates/campus-schedule/src/notice.rs
  - crates/campus-schedule/src/occurrence.rs
  - crates/campus-schedule/src/lib.rs
  - crates/campus-schedule/Cargo.toml
  - crates/campus-schedule/NOTICE.md
  - crates/campus-schedule/LICENSE-shiguangschedule.txt
tags:
  - schedule
  - timetable
  - zhengfang
  - algorithm
  - apache-2.0
---

# 课表核心（campus-schedule）

`crates/campus-schedule`：课表领域核心，算法与数据模型移植自 shiguangschedule（Kotlin → Rust，Apache-2.0；`model.rs:1-5` 文件头注明来源与修改），新增课程来源隔离、正方教务解析器与法定节假日解析合并。依赖仅 serde / serde_json / chrono / thiserror（`Cargo.toml`），与 [[modules/campus-auth|CAS 协议核心]] 同为「协议单点」、无 Tauri 依赖。

## 数据模型（model.rs）

- **Course**（`model.rs:31-68`）：字段对齐 shiguangschedule 的 Room 实体。两个关键设计：
  - **weeks 显式列表**（`model.rs:58-59`）：`weeks: Vec<u32>`（1-based），替代单双周标志位——沿用上游设计，任意周型（单周/双周/跳周）一律可表达。
  - **source 来源隔离**（`model.rs:19-26,57`）：`CourseSource::{Import, Manual}`——自动更新与调课解析只作用于 `Import` 课程，手动添加的课程永不触碰，为「自动更新只作用于导入课程」提供数据基础。`class_id`（正方 `jxb_id`）是自动更新 diff 的匹配键之一（`model.rs:61-63`）。
  - **disabled 停开标记**（`model.rs:64-68`，M2.5 批次 1 新增，`#[serde(default)]`）：自动更新发现课程在教务最新课表中消失时置 `true`（**不删记录**，保留其挂载的调课 override 可回滚）；`Manual` 课程**永不置位**（冻结契约 §2.4）。旧 JSON 无该字段缺省 false（serde 单测 `model.rs` `course_disabled_defaults_false_and_roundtrips`）。
  - 自定义时间：`is_custom_time = true` 时忽略节次、用 `custom_start_time/custom_end_time`（`model.rs:46-50`）。
- **CourseTableConfig**（`model.rs:66-96`）：`semester_start_date` 是周次计算锚点；默认 20 总周、一周从周一起（`model.rs:83-88`）；`slots: Option<Vec<TimeSlot>>`（`model.rs:89-95`，M2.5 收尾轮新增，`#[serde(default)]` 旧文件缺省 None）——自定义作息，None/空 = 内置校本大节表，有值 = 唯一事实源（取值单点与校验见 [[decisions/timetable-editable-slots|作息时间表可编辑]]）；`skipped_dates: Vec<NaiveDate>`（批 2 2026-09-19 新增，serde default 空列表）——全校性停课日（契约 §8.1），网格该列「休」、ICS 剔除、今日页（批 9）`skipped` 态。
- **TimeSlot**（`model.rs:91-101`）：节次 → "HH:MM" 时间段。
- **CourseOverride 调课叠加**（`model.rs:114-139`，自建模型、上游无）：叠加在导入课程之上、原数据保留可回滚；`OverrideKind::{Rescheduled, Cancelled, Extra}`（调课/停课/补课）；`source_notice_id` 是撤销与去重键（撤销某条通知 = 删除其匹配的全部 override）；`auto_applied` 区分高置信自动应用与低置信待确认。
- **Timetable 本地课表**（`model.rs:142-162`，M2.5 批次 1 新增）：`{ config: CourseTableConfig, courses, overrides, updated_at }`，即 `%APPDATA%/campushub/timetable.json` 的顶层结构（持久化与 IPC 透出见 [[modules/campus-hub-tauri|接线层]]）；序列化 camelCase（`updatedAt`），`courses`/`overrides`/`updated_at` 带 serde 缺省（旧文件/手工删节可读，单测 `timetable_serde_defaults_and_camel_case`）。

## 周次计算（weeks.rs，Kotlin 原文 AppSettingsRepository.kt:138-224）

- `week_index_at_date(target, semester_start, first_day_of_week)`（`weeks.rs:28-37`）：两端各自对齐到本周首日（`previous_or_same_day_of_week`，不足则回退，`weeks.rs:14-24`），天数差 / 7 + 1。自定义一周起始日由 `first_day_of_week` 支持。
- `current_week(today, cfg)`（`weeks.rs:40-51`）：越界（不在 1..=total_weeks，如假期）或未配置开学日 → None。
- `semester_start_from_week(today, week, first_day_of_week)`（`weeks.rs:55-62`）：反推开学日期，首周引导用；与正向互算 roundtrip 有测试（`weeks.rs:125-131`）。
- golden：2026-09-07（周一）开学时 2026-09-17 为第 2 周，与门户课表实测一致（`weeks.rs:69-76`）。

## 网格布局算法（grid.rs，Kotlin 原文 WeeklyScheduleViewModel.kt）

两种模式（`ScheduleMode`，`grid.rs:16-21`）：`Section`（节次网格）与 `Time24h`（24 小时网格）。

- `time_to_grid_scale` / `grid_scale_to_time`（`grid.rs:45-140`）：时刻 ↔ 浮点网格坐标（1.0 = 第 1 格顶部）双向换算；节间空隙落到下一节起点（`grid.rs:85-91`）；反向换算按整数节次取 TimeSlot 内插值（拖拽改课用）。
- `merge_courses(courses, time_slots, current_week, mode) -> Vec<MergedCourseBlock>`（`grid.rs:153-314`），Kotlin `mergeCourses` 逻辑 1:1 移植（含容差与越界修正）：
  1. 归一化：课程 → (start, end) 浮点区间（自定义时间与节次两条路径，`grid.rs:174-197`），越界钳制与最小高度 0.3 修正（`grid.rs:200-223`）；
  2. 按星期分组、起点升序 + 跨度降序排序（`grid.rs:229-245`）；
  3. **重叠分簇**：与簇内任一课程重叠（±EPS=0.01 容差）则并入（`grid.rs:247-259`）；
  4. **簇内贪心分列**（区间图着色）：复用首个结束时间 ≤ 起点的列，否则开新列（`grid.rs:261-284`）；
  5. 输出 `MergedCourseBlock`：`column_index/column_count` 供前端按列偏移渲染，非本周课程 `is_visual_demoted` 视觉淡化（`grid.rs:297-310`）。
- 渲染层（React）直接消费该几何结果做 absolute 定位（`grid.rs:9`）。

## 默认节次表（timeslots.rs）

`default_time_slots()`：13 节作息，08:00 起，午休 12:15-14:00，晚餐 17:20-18:30（`timeslots.rs:9-24`；Kotlin 原文 CourseTableRepository.kt:382-396）。后续按本校作息覆盖、用户可在设置中编辑（`timeslots.rs:3-4`）。

## 正方教务解析器（zhengfang.rs，本项目原创）

协议实测自 `https://jwgl.cwxu.edu.cn`（2026-09-17，`zhengfang.rs:2-4`）：`POST /jwglxt/kbcx/xskbcx_cxXsKb.html?gnmkdm=N2151`，body `xnm=<学年起始年>&xqm=<学期代码>`。

- `Semester`：第 1 学期=3、第 2 学期=12、暑期=16（`zhengfang.rs:52-66`）；`query_body` 构造表单体（`zhengfang.rs:69-71`）。
- `parse_kb_response(json, course_table_id)`（`zhengfang.rs:88-141`）：`kbList` 条目只声明用到的字段、全部宽容缺省（`zhengfang.rs:21-48`）；`jcs` "3-4" 解析节次、`xqj` 解析星期（非法条目整条跳过，坏数据测试 `zhengfang.rs:166-179`）；`oldzc` 十进制周次位掩码（bit0=第 1 周）经 `expand_week_mask` 展开（`model.rs:136-138`，单双周测试 `model.rs:147-150`）；输出 `source=Import` 课程。
- `stable_color`（`zhengfang.rs:144-146`）：课程名字节 `*31` 散列出颜色索引，同一门课每次导入颜色一致。

## 导入 diff（diff.rs，M2.5 批次 2，本项目原创）

`diff_courses(existing, incoming) -> DiffResult`（`diff.rs:135`）实现冻结契约 §2.4 的自动对比更新，输出即「合并后的完整课程列表」（直接写库）+ 计数（added/changed/removed）+ 人类可读 `changes` 文案：

- **匹配键 = 课程名 + `class_id` + 时段三元组（day/start_section/end_section）**（`match_key`，`diff.rs:39`，五元组）；`class_id` 缺失退化为仅课程名匹配（`Option<&str>` 的 None==None）。⚠️ 匹配键只有名+id 在真机翻过车：正方 kbList 同一教学班（同 `jxb_id`）**按多时段拆多条**（马原 3 条同 `540B1687E8…`），二次同步一对多覆盖 → 本地多条被改成同一条时段 → 同格重叠分列渲染、课程块压缩。2026-09-19 修复：diff 改**消费式一对一配对**（incoming 按 key 分桶 `VecDeque`，旧库逐条队首消费 + `consumed` 标记；新增只收未消费项），回归测试 `diff_pairs_same_class_multi_section_one_to_one` / `diff_duplicate_local_keys_only_one_consumed`。
- **Manual 零触碰**（`diff.rs:141-145`）：旧库 Manual 课程原样 push——不参与匹配、不被更新、永不被置 `disabled`（专门单测 `diff_never_touches_manual_courses` 覆盖「同名 Import 来袭」与「Manual 消失」两个方向）。
- 消失 → 置 `disabled=true` 不删记录；**已停开的再次消失不重复计数/不重复报文案**（`diff.rs:167-176`，单测 `diff_second_run_no_duplicate_removed`）——removed 语义 =「本次新发现停开」。
- 字段级 diff（`field_changes`，`diff.rs:43-76`）：星期/节次/周次/教室/教师五个字段；周次**排序后比较**（旧库手工编辑乱序不误报）。文案形态照契约：`信息安全 教室 D4-207 → D4-305`，多字段以「；」连接；added=`新增 X`、removed=`X 停开`、复活=`X 恢复开课`。
- **复活语义**：disabled 旧课再次匹配到 → `disabled=false` 并计入 changed（单测 `diff_revives_disabled_course`）。
- 匹配成功时**整条采用 incoming 数据但 id 沿用旧库**（`diff.rs:160-165`）——调课 override 挂在 `course_id` 上，id 必须稳定（class_id 缺失退化匹配时 incoming 的 `<table_id>-<空 jxb_id>` id 可能不同，取舍见 [[decisions/timetable-diff-manual-and-ics|课表 diff、手动课程与 ICS 导出决策]]）。
- `format_weeks`（`diff.rs:103`）：周次列表连续区间合并成紧凑文案（`1,2,3,7,8` → `1-3,7-8`）。

## 法定节假日解析与合并（holiday.rs，2026-09-20 节假日轮）

- `parse_timor_year(json) -> Vec<NamedDate>`：解析 timor.tech `holiday/year/{year}` 响应（`{"holiday":{"MM-DD":{holiday,name,date,…}}}`），只取 `holiday=true`（放假）——`false` 是调休补班日照常上课；升序去重，坏条目跳过，非 JSON 报 Err。拉取请求在 tauri 层（timor 是公网 API、与校园会话无关，需浏览器 UA 否则被 Cloudflare 拦），本 crate 保持零网络依赖。
- `merge_holidays(skipped, named, fresh) -> (skipped, named)`：先从 `skipped_dates` 剔除旧自动假日（`holiday_names` 是「自动假日」的唯一记录），再并入新集——官方修订取消的假日自动退场，用户手动停课日不受影响；`holiday_names` 整体替换（名随官方修订）。合并语义被 `merge_sorts_and_dedups` 等单测钉住。

## 调课通知 L1/L2 解析（notice.rs，M2.5 批次 3，本项目原创）

`parse_notice_text(text, courses, current_week) -> Vec<NoticeCandidate>`（`notice.rs`）实现冻结契约 §2.5，纯函数、零新依赖（std 字符串处理，无 regex）；单候选产出与取舍见 [[decisions/timetable-notice-l1l2|调课通知 L1/L2 分级口径与 noticeId 取舍]]。
- **全校日期置换探测**（`detect_date_swap(text, semester_start) -> Option<Result<DateSwap, String>>`，2026-09-19 节假日轮）：真实公告「9月20日（星期日）补9月28日（星期一）课程」是课表层置换（不含课程名/节次）——逐课解析会把书名号《关于…放假安排的通知》误提为课程名。返回 None（非置换）/ Err（置换但缺学期锚点）/ Ok（date=上课日、weekday=被补日星期、confidence、excerpt）。tauri 层 `auto_parse_notices` 双轨：置换命中 → `NoticeSwapCandidate`（`apply_swap_day` 写 `config.swap_days`，替代早期逐课 Extra 方案）；未命中 → `parse_notice_text`。置换渲染周次按置换日所在教学周现算（模型不存周次）。

- **NoticeCandidate**（契约 §2.3 冻结字段，camelCase）：`{noticeId, courseId?, courseName, changeType, weeks, newDay?, newStartSection?, newEndSection?, newPosition?, confidence: "high"|"low", reason, excerpt}`；`NoticeConfidence::{High, Low}` serde 小写。
- **L1 提取**（数字锚定扫描 = `match_indices('周'/'节')` + `digits_before` 往左收 ASCII 数字，字节级安全因多字节字符各字节 ≥0x80）：
  - 课程名：对本地课程名 contains 匹配；多名命中裁剪被长名包含的短名；0 命中回退书名号《X》提取填充 `courseName`；
  - 周次：`3-4周` 区间优先 → 单周 `第3周`/`3周` → `本周`（需 `current_week` 锚点）；
  - 星期：`周X` / `星期X` / `星期天`，1=周一…7=周日；
  - 节次：`3-4节` 区间优先 → 单节 `第3节`（**强制「第」前缀**防「共16节课」误提）；
  - 教室：`D4-207` 形态 token（`字母数字-数字`）优先 →「教室：/教室:」后兜底；
  - 类型关键词：停课 > 补课 > 调课，无命中默认 Rescheduled（不影响置信）。
- **箭头消歧**（`after_adjust_arrow`）：「由 A 调整到 B」的新值在 12 个箭头词之后——周次/星期/节次/教室四提取器 **tail 优先、全文回退**；旧值在前是调课通知的结构性特征（首个单测样例即暴露）。
- **L2 置信**：reasons 空集 ⇔ High——课程唯一命中 && 周次/星期/节次齐全；降级原因逐项中文写入 `reason`（缺要素 / 同名多门 / 多名 / 0 命中 / 「本周」无锚点）。
- **noticeId**：`notice_id_for` = `manual:<16 位十六进制>`（`DefaultHasher` 正文哈希，非密码学，去重与撤销键；M5 改公告 id）。

## 生效实例展开（occurrence.rs，批 2 2026-09-19，本项目原创）

`expand_occurrences(course, overrides, week) -> Vec<CourseOccurrence>`（契约 §8.4，取舍见 [[decisions/timetable-occurrence-expansion|课表生效实例展开]]）把一门课程 × 一个周次展开为经 override 叠加后的全部实例：`OccurrenceKind::{Solid, MovedOut, Cancelled}` 对齐前端 `buildWeekBlocks` 的三态（solid 实块 / moved-out 已调出 / cancelled 已停）。**受控双写**：语义基准是前端 `buildWeekBlocks`（Rust 单测钉住语义，两处注释互锚）；ICS 导出与今日页（批 9）只消费 `Solid`。展开规则：停课两档（契约 §2.5.1，`new_day` None = 整周全停 / Some(d) = 仅该次）、停课优先于调课、调课跨天/换节 = 原时段 MovedOut + 新时段 Solid、仅换教室 = 原位 Solid 新教室、extra 补课追加 Solid 新实体、多条 override 逆序取最后；**extra 循环独立于停课/调课分支且不看出 `course.weeks`**（复核 P1-a/P1-b 修订：停课+补课并存、补课周 ∉ course.weeks 都要与前端覆盖面一致）；custom 课（节次 None）实例节次为 None，被 resched 时产出带节次的 Solid 走大节表（P3-c 登记，见 [[modules/campus-schedule|课表核心]] occurrence 节注释互锚）。

## Apache-2.0 合规三件套

1. `THIRD-PARTY-NOTICES.md`（仓库根）第一节：shiguangschedule 来源、版权（Copyright (C) 2025 XingHeYuZhuan）、许可证副本位置与涉及文件清单。
2. `crates/campus-schedule/LICENSE-shiguangschedule.txt`：Apache-2.0 许可证全文副本。
3. `crates/campus-schedule/NOTICE.md`：移植范围与修改说明；另各移植源文件头（model/weeks/grid/timeslots.rs:1-7）均标注 Adapted 来源。crate 自身 `license = "Apache-2.0"`（`Cargo.toml`）。
