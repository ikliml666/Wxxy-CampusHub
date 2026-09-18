---
title: 课表 diff、手动课程与 ICS 导出决策
type: decision
source_files:
  - crates/campus-schedule/src/diff.rs
  - tauri-app/src-tauri/src/commands/timetable.rs
  - tauri-app/src-tauri/src/infra/timetable.rs
tags:
  - timetable
  - diff
  - ics
  - m2.5
---

# 课表 diff、手动课程与 ICS 导出决策（M2.5 批次 2）

## 背景

冻结契约 §2.4/§2.3 规定了 diff 语义与命令面，但实现时遇到契约未覆盖的细节，需就地裁决并记录。

## 决策

1. **diff 匹配成功时整条采用教务新数据、但 `id` 沿用旧库**（`diff.rs` 合并分支）：调课 override（批次 3 落地）挂在 `course_id` 上，前端也以 id 为 React key——id 必须跨导入稳定。常规路径 `class_id` 相同 → `jxb_id` 相同 → id 天然一致；唯一分叉是 `class_id` 缺失退化匹配（旧 id 可能 ≠ `default-<空>`），此处强制保留旧 id。
2. **复活计入 changed**：`disabled=true` 的旧课重新匹配到 → `disabled=false`，计入 changed（文案 `X 恢复开课`）。理由：用户视角这是一次值得展示的变更；计入 removed（复活当负变更）语义混乱。
3. **已停开课程再次消失不重复计数**：removed =「本次新发现停开」。否则每次导入 removed 都重复报旧账，前端「有变更」提示永不消停。
4. **周次列表排序后比较**：`weeks` 语义为集合，旧库经手工编辑后顺序可能乱，顺序敏感会把等价数据误报为变化。
5. **手动课程 id = `manual-<纳秒时间戳>`**：与导入 id `<table_id>-<jxb_id>` 前缀不同（永不冲突）；创建后即固定，且 Manual 不参与 diff，id 天然稳定。不引入 uuid 依赖。
6. **`delete_course` 级联删除该课程的 override**：课程删除后 override 成孤儿，revoke/回滚都找不到宿主；一并清理最省事（批次 3 的 override 写入路径尚未落地，先建好不变量）。
7. **ICS 使用 floating local time**（`DTSTART:20260907T080000`，无 `Z`/`TZID`）：作息表时刻是「本地墙钟」语义；floating 是 RFC 5545 合法形态，Outlook/Google 日历按导入时区解释。避免手写 VTIMEZONE 块（冗长且换时区用户仍需正确性让位给简洁）。**不做行折叠**：SUMMARY/LOCATION/DESCRIPTION 均为短文本（课名/教室/教师，实测远低于 75 字节上限）。
8. **ICS 大节号 = `(start+1)/2`（整数除法，即 ceil(小节/2)）**，结束时刻取 `end_section` 对应大节的 `end_time`（`3-4节` → 大节2 → 10:10-11:50）；起始/结束大节任一超出校本 5 大节表 → 跳过该课程（无时刻可展开，不伪造）。校本大节表复用 `campus_portal::block_time_slots`（本批次提升为 `pub` 并 re-export）——与今日页「下一节课」同一事实来源，禁止复制常量。
9. **import 时配置初始化失败保留旧值**：`semester_start_date`/`semester_total_weeks` 由学期信息更新，解析失败不整体报错——坏一个字段不应丢整份课表。
10. **手动课程命令不做进程内互斥**：load-modify-save 的竞态仅在前端并发 invoke 时出现，交互模式（用户点击）天然串行；契约 §2.2 本就「原子性由调用方保证」。若未来出现后台定时导入，再补锁。

## 教训

- `Semester` 只有 1/2 两学期映射（契约口径），`"3"`（暑期）等未知序号直接报错而非猜测——参数推导宁缺毋滥。
