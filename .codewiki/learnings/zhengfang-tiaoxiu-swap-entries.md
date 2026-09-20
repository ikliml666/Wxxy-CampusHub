---
title: 教务调休条目形态与公告置换冗余
type: learning
source_files:
  - crates/campus-schedule/src/diff.rs
  - tauri-app/src-tauri/src/commands/timetable.rs
tags:
  - schedule
  - zhengfang
  - holiday
  - evidence
---

# 教务调休条目形态与公告置换冗余（2026-09-20 取证）

**现象**：用户报教务引入调休功能后，与公告解析冲突，「同一门课挤在同一格」。

**取证**（复用 restore_session 恢复本机会话拉 kbList，临时 example 用完即删；2026-09-20 恰为补课日）：

1. **正方教务以「新增同教学班条目」表达调休补课**：kbList 中「信息隐藏与取证技术」同 `jxb_id=542DE6D999` 出现两条——`xqj=1, zcd=1-3周,5-12周`（原周一课）+ `xqj=7, zcd=2周`（调休新条目，zcd 锁定补课周）。即**教务已把补课排进周日列**，diff 同步后本地自动获得，无需任何置换逻辑。
2. **公告置换（`config.swap_days`）与教务调休是同一事实的两份表达**：并存必然冗余——置换渲染层是「整列替换」（swap 命中列只显示被补日课程），教务调休条目则是真实课程条目；二者并存时同格重叠/替换互相打架。
3. **kbList 的 `date`/`dateDigit` 字段是查询日时间戳**（全部条目同值「2026年9月20日」），**不是**逐条上课日期，勿误读为调休标记。
4. timor.tech 节假日 API（时光课程表同源）`holiday/year/{year}` 需**浏览器 UA**，否则被 Cloudflare 拦（裸 curl 返回 Just a moment 页）；`holiday=false` 是补班日（如 2026-09-20「中秋节前补班」、10-10），**不上跳过集**。

**修复口径（教务为准三层架构）**：① 每天自动导入教务课表（`auto_sync_tick`，启动+每小时，`last_auto_import` 闸）；② 导入后清理已被教务表达的置换（`weekday_covered`）与完全重合的旧逐课 extra override（`redundant_extra_override_ids`），`apply_swap_day` 采纳前同判定拦截；③ timor 法定假日并入 `skipped_dates` 补上教务调休功能的缺口（如中秋 9-25~27 教务未处理、假日照常排课），渲染/ICS/今日页的「休」态全部复用既有 skipped 消费方。

**教训**：同一事实有两份数据来源时，先取证权威源的表达形态（是改数据还是加标记），再让辅助源「权威源已表达即退场」——判据放导入/采纳的单点（crate 纯函数 + 单测），不在每个消费方各拦一遍。
