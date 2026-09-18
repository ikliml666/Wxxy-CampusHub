---
title: 课表核心（campus-schedule）
type: module
source_files:
  - crates/campus-schedule/src/model.rs
  - crates/campus-schedule/src/weeks.rs
  - crates/campus-schedule/src/grid.rs
  - crates/campus-schedule/src/timeslots.rs
  - crates/campus-schedule/src/zhengfang.rs
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

`crates/campus-schedule`：课表领域核心，算法与数据模型移植自 shiguangschedule（Kotlin → Rust，Apache-2.0；`model.rs:1-5` 文件头注明来源与修改），新增课程来源隔离与正方教务解析器。依赖仅 serde / serde_json / chrono / thiserror（`Cargo.toml`），与 [[modules/campus-auth|CAS 协议核心]] 同为「协议单点」、无 Tauri 依赖。

## 数据模型（model.rs）

- **Course**（`model.rs:31-68`）：字段对齐 shiguangschedule 的 Room 实体。两个关键设计：
  - **weeks 显式列表**（`model.rs:58-59`）：`weeks: Vec<u32>`（1-based），替代单双周标志位——沿用上游设计，任意周型（单周/双周/跳周）一律可表达。
  - **source 来源隔离**（`model.rs:19-26,57`）：`CourseSource::{Import, Manual}`——自动更新与调课解析只作用于 `Import` 课程，手动添加的课程永不触碰，为「自动更新只作用于导入课程」提供数据基础。`class_id`（正方 `jxb_id`）是自动更新 diff 的匹配键之一（`model.rs:61-63`）。
  - **disabled 停开标记**（`model.rs:64-68`，M2.5 批次 1 新增，`#[serde(default)]`）：自动更新发现课程在教务最新课表中消失时置 `true`（**不删记录**，保留其挂载的调课 override 可回滚）；`Manual` 课程**永不置位**（冻结契约 §2.4）。旧 JSON 无该字段缺省 false（serde 单测 `model.rs` `course_disabled_defaults_false_and_roundtrips`）。
  - 自定义时间：`is_custom_time = true` 时忽略节次、用 `custom_start_time/custom_end_time`（`model.rs:46-50`）。
- **CourseTableConfig**（`model.rs:66-88`）：`semester_start_date` 是周次计算锚点；默认 20 总周、一周从周一起（`model.rs:83-88`）。
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

## Apache-2.0 合规三件套

1. `THIRD-PARTY-NOTICES.md`（仓库根）第一节：shiguangschedule 来源、版权（Copyright (C) 2025 XingHeYuZhuan）、许可证副本位置与涉及文件清单。
2. `crates/campus-schedule/LICENSE-shiguangschedule.txt`：Apache-2.0 许可证全文副本。
3. `crates/campus-schedule/NOTICE.md`：移植范围与修改说明；另各移植源文件头（model/weeks/grid/timeslots.rs:1-7）均标注 Adapted 来源。crate 自身 `license = "Apache-2.0"`（`Cargo.toml`）。
