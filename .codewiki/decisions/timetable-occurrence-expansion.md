---
title: 课表生效实例展开（expand_occurrences 与 ICS 按生效结果导出）
type: decision
source_files:
  - crates/campus-schedule/src/occurrence.rs
  - crates/campus-schedule/src/model.rs
  - crates/campus-schedule/src/lib.rs
  - tauri-app/src-tauri/src/commands/timetable.rs
  - tauri-app/frontend/src/panels/TimetablePanel.tsx
  - tauri-app/frontend/src/shared/types.ts
tags:
  - timetable
  - ics
  - override
  - frontend
---

# 课表生效实例展开（expand_occurrences 与 ICS 按生效结果导出）

2026-09-19（全量补齐批 2 / P2，蓝图决策 5、契约 §8）。此前 `build_ics` 只遍历
`courses` 原始字段，调课/停课/补课 override 与跳过日、自定义时间课全部不生效于
导出日历（对账复核并入项 1/4/附注 5）；本批建立 Rust 侧「生效实例展开」纯函数
作为 ICS 与今日页（批 9）共用的地基。

## 结构与语义

- `OccurrenceKind::{Solid, MovedOut, Cancelled}` + `CourseOccurrence { kind, day,
  start_section, end_section, position, source_override_id }`；`expand_occurrences(
  course, overrides, week) -> Vec<CourseOccurrence>`。
- 展开分支逐条对照前端 `buildWeekBlocks`（受控双写：TS 无法调 Rust 纯函数，前端
  实现成熟且过 UI 验收，重写风险更大——Rust 侧单测钉住语义 + 两处注释互锚，
  改一处必须同步另一处）：停课两档（契约 §2.5.1）、停课优先于调课、调课 = 原时段
  MovedOut + 新时段 Solid（结束节次缺省 = 起始）、仅换教室 = 原位 Solid 新教室、
  多条 override 逆序取最后（与 `upsert_override` 覆盖写入呼应）。
- **extra 独立于停课/调课分支、不看出 `course.weeks`**（复核 P1-a/P1-b 修订）：
  前端 extra 循环在课程实体块分支之外独立执行，且不检查 `course.weeks`——初版
  Rust 把 cancel/resched 分支写成提前 `return` 吞掉了 extra（停课+补课 → 网格有
  补课块而展开结果没有），且 `build_ics` 只迭代 `course.weeks` 漏掉补课周。修复后
  expand 的 extra 循环恒执行；`build_ics` 迭代周次 = `course.weeks ∪ 各
  override.weeks`（去重排序）。
- **custom 课 override 语义（复核 P3-c 登记，与 occurrence.rs 模块注释互锚）**：
  custom 课不进网格（前端对节次 None continue）；被 resched → 带节次的 Solid 走
  大节表取时刻；未调整 → 节次 None 的 Solid 走 `custom_*_time`。批 9 统一口径。

## ICS 映射规则（决策 5）

- **cancelled → 不生成 VEVENT**（否决 `STATUS:CANCELLED`）：导出目的是提醒上课，
  取消事件在 Outlook/Google 仍显示条目徒增噪音；网格留虚线占位、日历只要事实。
- **调课**：原时段不生成；新时段生成，UID `{id}-w{week}d{newDay}s{newStart}@campushub`
  ——与原 UID **格式相同、值不同**，保证与原时段 UID 不同防日历端去重错乱；原位
  实例与旧版 UID 逐字节一致（golden 保）。
- **UID 后缀 `-o{override 短 id}`**（复核 P3-a 修订）：调课新位与补课实例追加
  override id 前 8 位——初版仅靠「day/start 代入 new 值」区分，补课缺省参数与
  原位同周同日同起始节次时会逐字节撞 UID（日历端去重错乱）；仅换教室（原位）
  是「同一事件换教室」，UID 保持不变。
- **DESCRIPTION 追加「调课/补课」**：按 `source_override_id` 回查 override 类型。
- **跳过日**：VEVENT 日期命中 `config.skipped_dates` → 剔除。
- **custom 课**：DTSTART/DTEND 直取 `custom_start_time/custom_end_time`（替掉旧版
  对无节次课程的整体 continue）；UID 用 `scustom` 段。
- VTIMEZONE 维持 floating local time 不动，只改 VEVENT 生成。

## 关键取舍

- **`source_override_id` 超出决策 5 结构规格**（契约 §8.4 已冻结）：Solid 实例无法
  仅凭几何字段区分「原位 / 调课新位 / 补课」，ICS 的 DESCRIPTION 标注必须溯源
  override；批 9 今日页亦可复用。备选「按课程是否有 override 推断」会把补课标签
  错标到同周原时段实例上，否决。
- **`effective_slots_at(config, date)` 本批只迁移签名**（契约 §8.3，决策 2）：
  date 参数暂不消费，批 3 只改函数内部实现 slot_rules 区间命中，全部调用点零返工。
- **节次口径**：occurrence 存教务小节（与 `Course.start_section` 同尺度），消费方
  按 `(s+1)/2` 折大节——与前端 blockOf 同一公式，避免出现第三套坐标。

## 前端

- 跳过日列：渲染层过滤（与周末裁剪同层，`buildWeekBlocks` 不动）——列头日期置灰 +
  「休」徽标，列内课程与空位按钮都不渲染；日期口径复用 `weekDates`/`dayKeyOf`。
- 设置弹层「跳过日期」区块：date input + chips 列表增删，独立「保存跳过日期」按钮
  调 `save_skipped_dates`（整体替换语义，与学期设置分开提交）；YAGNI 不做日历面板。
- `CourseTableConfig.skippedDates: string[]`（serde default 无 skip → 恒存在，
  非 optional，与 slots 同形态）。
