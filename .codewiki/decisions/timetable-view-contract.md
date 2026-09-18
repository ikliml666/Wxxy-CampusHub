---
title: 课表视图契约（TimetableView 下发 slots 与 currentWeek）
type: decision
source_files:
  - tauri-app/src-tauri/src/commands/timetable.rs
  - tauri-app/frontend/src/panels/TimetablePanel.tsx
  - tauri-app/frontend/src/shared/types.ts
tags:
  - timetable
  - ipc-contract
  - frontend
---

# 课表视图契约（TimetableView 下发 slots 与 currentWeek）

2026-09-18（M2.5 批次 4 前），冻结契约 §2.3 的 `get_timetable` 出参由批次 1 的裸
`Timetable` 修订为 `TimetableView{ timetable, slots, currentWeek, today }`。

## 动机

1. **时间标签唯一事实源**：校本大节作息（5 大节）此前是 `campus-portal::parse` 的
   私有 `block_time_slots()`，只有今日页间接消费。若课表页自行硬编码「大节1=08:00」，
   未来作息校准（大节 1/4/5 尚未实测，见 parse.rs 的 ponytail 标记）要改两处且必然漂移。
   把 `slots` 随 `get_timetable` 下发，前端时间标签只来自后端（教训同
   [[learnings/portal-block-periods-and-school-timetable|门户大节语义与校本作息]]）。
2. **周次口径统一**：视图周默认值、列头日期（开学日 + (周-1)×7）都依赖「当前是第几周」。
   前端自算会复制 `weeks::current_week` 的对齐逻辑（firstDayOfWeek 折算），且与
   `parse_notice` 用的口径可能分叉。后端算好 `currentWeek`（无开学日/今天越出学期 →
   null）+ `today`（"YYYY-MM-DD"，列头高亮直接字符串比对）一次下发。

## 落地要点

- 后端组装是纯函数 `build_timetable_view(tt, today)`（`commands/timetable.rs`），
  可脱离 Tauri 单测（148 passed 基线上 +2）。
- 前端镜像类型在 `shared/types.ts`：⚠️ `NoticeCandidate` 的 Option 字段 Rust 侧
  `skip_serializing_if` 缺省省略 → TS 必须用**可选属性**；`Course`/`CourseOverride`
  的 Option 字段无 skip → 恒存在 `| null`。两类形态不可混写。
- 导入课程的 `colorIndex` 来自 `zhengfang::stable_color`（课名 31 折叠哈希，0..=65535），
  不是色板下标——前端取色一律 `% COURSE_PALETTE.length`，与手动课程的 0..=7 下标统一。
- 导入课程 `remark` 装的是「课程性质·考核方式」（`kcxz·khfsmc`），不是备注——
  详情浮层按 `source` 区分标签（导入=「性质 · 考核」，手动=「备注」）。

## 渲染语义分工（前端 `buildWeekBlocks`）

override 叠加不改原记录，视图层负责合成：停课（cancelled）→ 原时段虚线占位「已停」；
调课（rescheduled，新时间≠原时间）→ 原时段「已调出」虚块 + 新时段实体块；仅换教室 →
原位渲染新教室；补课（extra）→ 新增实体块。同一课多条 override 逆序取最后一条
（后采纳覆盖先采纳，与后端 `upsert_override` 幂等语义呼应）。同日重叠分列是后端
`grid::merge_courses` 贪心占道语义的轻量前端重写（不在 IPC 面，勿重复实现）。
