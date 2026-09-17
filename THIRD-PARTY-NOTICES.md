# 第三方组件版权声明（THIRD-PARTY NOTICES）

本文件汇总 Wxxy-CampusHub 所引用的第三方项目/组件及其许可证条款。这些组件由各自的许可条款授权，本项目自有代码的许可不覆盖它们。

> 当前为移植期初始版本；M0 脚手架锁定依赖树后，将按 Wxxy-CampusLogin 的模式补全 Cargo.lock / package-lock.json 级别的完整依赖清单。

## 一、第三方项目（源码级引用）

### shiguangschedule（拾光课程表）

- 来源：<https://github.com/XingHeYuZhuan/shiguangschedule>
- 版权：Copyright (C) 2025 XingHeYuZhuan
- 许可证：Apache License 2.0（副本：`crates/campus-schedule/LICENSE-shiguangschedule.txt`）
- 引用方式：算法与数据模型移植（Kotlin → Rust），范围与修改说明详见
  [`crates/campus-schedule/NOTICE.md`](crates/campus-schedule/NOTICE.md)
- 涉及文件：`crates/campus-schedule/src/{model,weeks,grid,timeslots}.rs`（文件头已标注）

## 二、第三方依赖（包管理器级）

- `campus-schedule` crate：serde / serde_json / chrono / thiserror（MIT 或 Apache-2.0 双许可；chrono 另含 Unicode-3.0 组件）
- M0 后统一汇总
