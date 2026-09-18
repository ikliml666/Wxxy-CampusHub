---
title: 作息时间表可编辑（save_time_slots 与 effective_slots 单点取值）
type: decision
source_files:
  - crates/campus-schedule/src/model.rs
  - tauri-app/src-tauri/src/commands/timetable.rs
  - tauri-app/src-tauri/src/lib.rs
  - tauri-app/frontend/src/panels/TimetablePanel.tsx
  - tauri-app/frontend/src/shared/types.ts
tags:
  - timetable
  - ipc-contract
  - frontend
---

# 作息时间表可编辑（save_time_slots 与 effective_slots 单点取值）

2026-09-18（M2.5 收尾轮）。M2.5 批次 4 明确「不做」作息编辑（内置校本大节表即可）；
收尾轮按计划 §2.1/§2.3 追加契约落地：`CourseTableConfig.slots` 字段 + `save_time_slots`
命令 + 前端「作息」弹层。

## 动机

内置校本 5 大节表 `campus_portal::block_time_slots()` 的 3 个档位是反推占位
（大节 1/4/5 有 `ponytail:` 标记，待校准），且学校作息可能学期间调整。没有编辑
入口时，每次校准都要改代码重编译；把作息变成用户可编辑数据后，校准动作降级为
「应用内改一次保存」。

## 数据模型与取值口径（契约 §2.1 / §2.3 收尾轮修订）

- `CourseTableConfig.slots: Option<Vec<TimeSlot>>`（serde default，旧文件无字段 →
  `None`）：`None`/空 = 内置校本大节表；有值 = 用户编辑过的作息，**视为唯一事实源**。
- 取值收敛到单点 `effective_slots(&CourseTableConfig) -> Vec<TimeSlot>`
  （`commands/timetable.rs`）：`config.slots` 有值且非空 → 自定义；否则回落
  `campus_portal::block_time_slots()`。**网格行（`TimetableView.slots`）、ICS 展开、
  大节号→时间查找三处必须共用本函数**，不得一处分发一处硬编码——原
  `build_timetable_view` / `build_ics` 各自调 `block_time_slots()` 的写法已收口。
- `save_time_slots(slots: Option<Vec<TimeSlot>>) -> TimetableView`：`None` = 清空
  自定义恢复内置；`Some(v)` = 校验通过后保存；返回刷新后的 `TimetableView`
  （前端免二次拉取）。命令数 34 → 35。

## 校验规则（非法一律 `CommandResult::err` 中文原因）

至少 1 条、≤ 20 条（防误填）；`number` 正整数且**严格递增不重复**；时间匹配
`HH:MM`（`parse_hm`：两位小时 ≤23、两位分钟 ≤59）且 `end_time > start_time`
（等长数字串字典序即时间序）。

## 取舍

- **大节号不手填**：前端 `SlotsEditor` 的行号即大节号（保存时 `number = 行序`），
  增删行天然满足「严格递增不重复」，后端校验仅作防御——用户语义里「第 N 大节」
  本就对应行序，手填只会制造校验错误面。
- **`Some(空数组)` 归一为 `None`**：与「None/空 = 内置」口径一致，保存前 filter，
  防御性归一（校验层已把空数组挡为「至少一条」，正常路径到不了）。
- **上限 20 条**：正常学校大节远低于此，仅防误粘贴大量行。
- **前端不引第三方时间选择器**：原生 `<input type="time">`（YAGNI）。
- **`TimeSlot` 补 derive `PartialEq`**：`CourseTableConfig` 的 `PartialEq` 链式要求 +
  单测 `assert_eq!` 比较 `Vec<TimeSlot>` 需要。
- 今日页「下一节课」仍直接消费 `block_time_slots()`（未经 config）——今日页没有
  课表 config 可读，作息校准以课表页的自定义为准后，今日页与课表页可能短暂
  分叉，属已知取舍（后续可让今日页改走 `get_timetable` 的 slots，本轮不动）。

## 前端

- 顶栏「作息」按钮（与导入/导出 ICS 同排）打开弹层：列出当前生效每一条
  （大节号 + `<input type="time">` 起止），支持增删行；「保存」→ `Some(v)`、
  「恢复本校默认」（仅自定义态可用）→ `null`；保存/恢复成功用返回的
  `TimetableView` 直接 `setView`，网格行随 `slots.len()` 变化（行数本就不写死）。
- `CourseTableConfig.slots: TimeSlot[] | null`（镜像 Rust，Option 无
  skip_serializing_if → 恒存在）。
- 停课 override 的渲染口径同步冻结为契约 §2.5.1 两档（`new_day` 有值 = 只停该周
  星期 d 那一次；`None` = 该周整周全停），`buildWeekBlocks` 既有行为本就一致，
  注释已改为明确引用契约。
