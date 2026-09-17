# NOTICE — campus-schedule

本 crate（`campus-schedule`）的部分算法与数据模型移植自第三方开源项目，依其许可证（Apache License 2.0）在此声明。

## shiguangschedule（拾光课程表）

- **上游项目**：<https://github.com/XingHeYuZhuan/shiguangschedule>
- **版权**：Copyright (C) 2025 XingHeYuZhuan
- **许可证**：Apache License 2.0（副本见本目录 `LICENSE-shiguangschedule.txt`）
- **移植范围**（Kotlin → Rust，均为算法与数据模型，未复制任何平台代码）：
  - 周次计算三函数：`getWeekIndexAtDate` / `calculateSemesterStartDate` / `getPreviousOrSameDayOfWeek`
    （上游 `data/repository/AppSettingsRepository.kt:138-224` → 本 crate `src/weeks.rs`）
  - 网格坐标换算与课程分列算法：`timeToGridScale` / `gridScaleToTime` / `mergeCourses`
    （上游 `ui/schedule/WeeklyScheduleViewModel.kt:324-409, 632-754` → 本 crate `src/grid.rs`）
  - 数据模型字段设计：`Course` / `CourseWeek`（周次显式列表） / `CourseTableConfig` / `TimeSlot`
    （上游 `data/db/main/*.kt` → 本 crate `src/model.rs`）
  - 默认 13 节作息常量
    （上游 `data/repository/CourseTableRepository.kt:382-396` → 本 crate `src/timeslots.rs`）
- **修改说明**：以上内容均已按 Rust 习惯重写（chrono/serde 生态），并做了如下扩展——
  课程 `source`（导入/手动来源隔离）、`CourseOverride`（单次调课叠加）、
  `class_id`（正方教学班匹配键）、正方教务响应解析器（`src/zhengfang.rs`，本项目原创）。
  修改过的文件保留上游版权声明并注明改动。

依据 Apache License 2.0 第 4 条：本 crate 不构成上游项目的官方背书，亦不使用
"shiguangschedule"/"拾光课程表" 名号进行推广。

## 本 crate 自有原创部分

- 正方教务课表响应解析器（`src/zhengfang.rs`）
- 课程来源隔离（`CourseSource`）与调课叠加（`CourseOverride`）模型
- 周次/分列算法的 Rust 工程化封装与全部单元测试

以上部分随 Wxxy-CampusHub 项目的许可条款分发。
