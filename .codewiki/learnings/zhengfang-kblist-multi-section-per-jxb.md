---
title: "正方 kbList 同一教学班按多时段拆多条——diff 匹配键必须含时段"
type: learning
source_files:
  - crates/campus-schedule/src/diff.rs
  - crates/campus-schedule/src/zhengfang.rs
tags:
  - zhengfang
  - diff
  - timetable
---

正方教务 `kbcx/xskbcx_cxXsKb` 的 kbList 中，**同一教学班（同 `jxb_id`）会按上课时段拆成多条记录**（2026-09-19 真机实测：马克思主义原理 3 条同 `540B1687E8…`、信息安全 2 条同 `542D71F5B2…`，各自不同 day/jcs/weeks）。

**后果**：任何以「课程名 + jxb_id」为键对多条记录做 `find`/`HashMap::insert` 的逻辑都会一对多覆盖。本项目 `diff_courses` 二次同步时本地 3 条马原全部被第一条教务记录的时段（周一 5-6）覆盖 → 同格重叠 → 前端分列渲染把课程块压成 1/3 宽（用户报告「点击同步更新后图案被压缩」）。

**修复口径**：匹配键必须是五元组 `(name, class_id, day, start_section, end_section)`，且多对多配对要用**消费式一对一**（incoming 按 key 分桶 VecDeque，旧库逐条队首消费），不能 `find` 也不能重建 HashSet 不回插。回归测试 `diff_pairs_same_class_multi_section_one_to_one` 固化。同类风险：任何新写的按 jxb_id 聚合逻辑（ICS、周次展开）都要先确认是否按多条展开处理。
