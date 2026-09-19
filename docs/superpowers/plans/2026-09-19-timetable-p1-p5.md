# 课表「全量补齐 P1-P5」分批实施蓝图（glm5.3 规划轮，2026-09-19）

> 规划：glm5.3 分身（用户点名）· 执行：glm5-3-flash 逐批派发（串行，同项目写入不并行）· 基线 master（P0 已完成，commit 4e2647b）。
> 本蓝图是唯一规划产物，后续执行分身按批领取；引用 `docs/upstream-shiguangschedule-inventory.md` 简称「清单§N」，`docs/HANDOFF-timetable-parity.md` 简称「HANDOFF」，`docs/superpowers/plans/2026-09-18-m2.5-timetable.md` 简称「契约」。
> 用户决策（2026-09-19）：全量 P1-P5 补齐；glm5.3 规划、glm5-3-flash 执行；每批主智能体验证后提交合并。

## 〇、总览与全局纪律

**批次总序（11 批）**：

| 批 | 内容 | 体量 | 新增 IPC 命令 |
|---|---|---|---|
| 1 | P1：设置与周首日接线（含开学日设置、ICS 对齐） | 中 | 1 |
| 2 | P2：skippedDates + ICS 按生效结果展开（含 custom 时间、复核并入项 1/4/附注5） | 中 | 1 |
| 3 | P3：slot_rules 组合作息 | 中 | 1 |
| 4 | P4：网格拖拽改课 | 中 | 0 |
| 5 | P5-a：表单与编辑体验（custom 时间表单/别名/dirty 拦截/备注计数/单双周文案） | 小 | 0（扩字段） |
| 6 | P5-b：JSON 导入导出 + ICS VALARM | 小 | 2 |
| 7 | P5-c：顶栏与渲染小件（周次弹窗/标题态机/非本周降级/色板扩展） | 小 | 0（扩字段） |
| 8 | P5-d：调课搬迁 + 快速删除 | 中 | 2 |
| 9 | P5-e：今日页接本地课表（复核并入项 3） | 中 | 1 |
| 10 | P5-F：多课表（存储 v2） | 大 | 5 |
| 11 | P5-G：多方案编辑 + 全局课程管理 | 大 | 0 |

**顺序理由**（对 HANDOFF 建议序的两处调整）：
- 严格保持 P1→P2→P3→P4 主链：P1 冻结 firstDayOfWeek 语义是 P2 ICS 日期展开的前置；P2 建立 Rust 侧「生效实例展开」纯函数（今日页与 ICS 共用的地基）；P3 只改 `effective_slots_at` 内部（P2 预留 date 参数，避免返工）；P4 在稳定数据链路上做交互。
- P5 十八条小件中，把「今日页接线」（依赖批 2 展开函数 + 批 5 自定义时间）与「调课搬迁/快速删除」（动课程批量语义、依赖批 4 的 override 语义成熟）独立成批，其余十二条按同域分 3 批（5/6/7）。硬塞 2-3 批会让单批跨 4 个文件域且验收面过宽。
- 多课表（批 10）放所有功能批之后：`mutate_timetable`（`commands/timetable.rs:214-222`）是所有写命令的**单点骨架**，批 10 只换这一个函数内部 + 新命令，中间批次无论怎么加命令都不增加批 10 成本；晚做让高价值特性先交付且无回归迁移代价（论证见架构决策 1）。
- 多方案编辑（批 11）在批 10 后：按课程名聚合的方案编辑天然限于单表内，应在最终存储结构上开发。

**全局纪律（每批隐含遵守）**：
1. 每批开工前先在契约文档追加修订段（沿用 §6 P0 惯例，不重写冻结段落），新命令名/字段名冻结后不得改；实现方发现契约问题 → 停下报告主智能体裁决。
2. `campus_schedule::default_time_slots()` 13 节常量与 golden 测试永不改；校本口径只动 `campus_portal::block_time_slots()`（`parse.rs:169`）与 `effective_slots` 收敛点。
3. 所有模型新字段一律 `#[serde(default)]`，旧 `timetable.json` 无损打开且**必须有单测证明**；TS 镜像同步（Rust Option 无 `skip_serializing_if` → TS `| null` 恒存在；有 skip → TS 可选属性，两类形态不可混写）。
4. 验证固定三件：`cargo test --workspace` + `tsc --noEmit` + `npm run build`；涉 UI 批加真机截图对照。
5. 收尾四件套：CHANGELOG 条目、CodeWiki（架构变化才更新 + `cw index` + `cw meta update`）、分支提交、合并。
6. 分支命名：本会话统一在 `feat/sess-1e38d0e8-*` 分支按批提交（会话 worktree 隔离纪律优先于蓝图命名建议），每批独立 commit、完成即 ff 合并 master（不攒批）。

## 一、架构决策冻结（实施前一次定清，执行者不得偏离）

### 决策 1：多课表存储升级（批 10）

**结构（单文件多表，YAGNI 否决每表一文件）**——桌面单用户、课表数据 KB 级，目录管理/孤儿文件/迁移复杂度都是纯成本：

```rust
// infra/timetable.rs 新顶层（v2）
pub struct TimetableStore {
    pub tables: Vec<TableEntry>,
    pub current_table_id: String,
}
pub struct TableEntry {
    pub id: String,          // 与 timetable.config.course_table_id 一致
    pub name: String,        // v1 迁移时 = "默认课表"
    pub created_at: String,  // RFC3339；v1 迁移时 = timetable.updated_at（空串容忍）
    pub timetable: Timetable,// 复用现有结构原样
}
```

- **旧文件自动迁移（懒迁移）**：`load_store(dir)` 先解析为 `serde_json::Value` 探测顶层键——有 `tables` → v2 反序列化；只有 `config` → v1，包装为单条 `TableEntry { id: "default", ... }`、`current_table_id = "default"`。**读时归一到内存，不立即写回**；首次任何 save 自然覆盖为 v2（用户只读不写不破坏原文件）。`save_store` 直接序列化 v2。
- **DEFAULT_TABLE_ID 处置**：保留为迁移锚点常量（v1 → `"default"`）；`empty_timetable()` 改为 `empty_table(id: &str)`；`parse_kb_response(json, table_id)`（import 时）传**当前表 id** 而非恒 default。
- **config（slots/skippedDates/slot_rules/overrides）归属**：本就挂在各表 `Timetable.config` / `overrides` 内，天然随表走；`currentTableId` 是 store 顶层唯一全局态。
- **既有命令零语义变化**：`mutate_timetable` 改名为 `mutate_current_table<T>(dir, f: FnOnce(&mut Timetable) -> Result<T, String>)`——内部换 `load_store` → 定位 `current_table_id` 的表 → f → 放回 → `save_store`。所有写命令（add/update/delete/apply_override/revoke/save_time_slots 及批 1-9 新增的写命令）**函数体一行不改**，这是批 10 晚做零额外成本的依据。
- **契约冲击面**：`TimetableView` 增 `table: TableBrief { id, name }` 字段（加法兼容）；infra 既有测试的 fixture 不动（表内结构未变）；契约文档补记 v2 存储结构。

### 决策 2：P3 slot_rules 数据结构与 effective_slots 改造

```rust
// model.rs（CourseTableConfig 增字段，serde default）
#[serde(default)]
pub slot_rules: Vec<SlotRule>,

pub struct SlotRule {
    pub start_date: NaiveDate,   // 含
    pub end_date: NaiveDate,     // 含
    pub slots: Vec<TimeSlot>,    // 恒非空（保存时校验）；序列化 camelCase
}
```

- **命中语义**（对齐上游 firstOrNull，清单§3）：`start_date <= date <= end_date` 取**首个命中**；区间允许重叠（重叠取先声明者，上游同语义，单测钉死）。无命中 → 回落 `config.slots` → 仍 None/空 → 内置 `block_time_slots()`（三段回落链）。
- **effective_slots 改造**：签名升级为 `fn effective_slots_at(config: &CourseTableConfig, date: NaiveDate) -> Vec<TimeSlot>`。批 2 就把所有调用点迁到该签名（date 暂不消费，注释标注 P3 扩展点）；批 3 只改函数内部实现区间命中。「三处共用收敛点」不变式保持：网格（`TimetableView.slots`）、ICS 展开（**逐事件日期取值**）、时刻查找全部经它。
- **TimetableView.slots 取「今天」的生效作息**（非视图周）：前端网格行数按单份 slots 渲染，跨区间换季的那一周无法逐天变行；上游周视图同样只取当天生效作息（清单§4「三周窗口预取」）。已知取舍，注释写明。ICS 精确到每个 VEVENT 日期。
- **保存命令独立**：`save_slot_rules(rules: Option<Vec<SlotRule>>) -> TimetableView`，与 `save_time_slots` 单职责并列，整体替换语义；`None` = 清空全部规则。校验：每条规则内部复用 `validate_time_slots`（`timetable.rs:564`）+ `start_date <= end_date`。

### 决策 3：今日页数据源接线（批 9）

- **新命令而非扩展 get_timetable / 混入 get_portal_overview**：门户聚合命令混本地数据是分层污染；get_timetable 塞今日子集是概念混杂。`get_today_courses() -> TodayCoursesView` 独立命令，TodayPanel 在 authed 时与 `get_portal_overview` **并行取数、互不阻塞**。
- **本地优先，教务兜底**：`TodayCoursesView.has_local == true`（本地有课程且已设开学日）→ 信任本地结果（`next` 为 null 就如实显示今日无课/已结束）；`has_local == false`（未导入/未配置）→ 回落现有门户 `nextCourse`，未导入用户体验不变。理由：本地是教务数据的超集（导入后叠加 override/停开/手动/跳过日），「下一节课」的价值在准确反映这些调整。
- **「下一节课」计算**：Rust 纯函数 `today_courses(tt, now) -> TodayCoursesView`（放 `commands/timetable.rs`，可脱离 Tauri 单测）：`current_week` 为 None → `no_semester`（无开学日）或 `vacation`（有开学日但越界，口径 = `weeks::week_index_at_date` 越界方向）；today 命中 `skipped_dates` → `skipped` 态；否则对每门未 `disabled` 课程用批 2 的 `expand_occurrences` 取本周实体课（过滤 occurrence.day == 今天星期），节次课查 `effective_slots_at(config, today)` 得起止时刻，自定义时间课取 `custom_*_time`；按开始时刻排序；`next` = 第一门未结束的（上游「第一门未结束」语义，清单§4）；`ongoing` 由后端按 now 算好下发（时钟口径单点，前端不自算）。
- 停课的那次不进列表（与 ICS 同语义——日历与今日页只要事实，网格才留虚线痕迹）。

### 决策 4：P4 拖拽交互设计（批 4）

- **点击/拖拽区分**：pointerdown 记录起点；移动曼哈顿距离 > **4px** 进入拖拽（`setPointerCapture`）；pointerup 时未进入拖拽 → 原点击行为（详情浮层），**且需 suppress 标志位吃掉后续 click 事件**（capture 后 click 仍触发，这是本批最常见 bug）。Esc 取消拖拽复位。
- **落点计算：前端几何换算，不走 grid.rs 互转**（纠正 HANDOFF 预判）：`grid.rs` 的 `time_to_grid_scale`/`grid_scale_to_time` 是「时刻 ↔ 浮点网格」换算，服务于 24h 模式；本仓是大节行网格，落点只需 `目标大节 = clamp(floor((y - 网格顶) / ROW_H) + 1, 1, slots.len())`、`天列 = floor((x - 左) / 列宽)`，纯前端像素几何，逐帧 IPC 调 Rust 不现实。grid.rs 预留件保留不动（24h 模式将来启用时用）。
- **拖拽中预览**：原块 `opacity-40` + 目标位置虚线轮廓（按落点 + 原跨度画 ghost div）；落点天列 hover 高亮。拖主体不改时长；**拉伸手柄不做**（触摸防误触设计，桌面 YAGNI，HANDOFF 已定）；跨周边缘浮动搬运不做。
- **周次语义与落库分叉（冻结决策 3 的精确化）**：
  - **多周课（weeks.length > 1，无论来源）→ 生成单周 Rescheduled override**：`weeks = [展示周]`、`new_day/new_start_section/new_end_section`、`new_position = 原教室`、`source_notice_id = "drag:<courseId>:<week>"`（稳定 id → 同课同周再拖被 `upsert_override` 幂等覆盖，天然支持反复拖）、`auto_applied = false`。与上游「多周克隆拆分」（清单§16.2）在 override 模型下等价。
  - **单周导入课 → 同样走 override**（决策 3：不被下次导入覆盖）。
  - **单周手动课（weeks.length == 1 && source == "manual"）→ 直接改字段**（update_course 链路），因为它本来就只有这一周。
  - 落库**零新命令**：override 路径构造 `NoticeCandidate { changeType: "rescheduled", noticeId: "drag:...", confidence: "low", reason: "拖拽调整", excerpt: "", ... }` 复用 `apply_override`；直改路径复用 `update_course`。位置未变短路（不产生记录）。
  - 新小节换算：`newStartSection = 2 * 新起始大节 - 1`，跨度保持 `原 endSection - 原 startSection`。ghost 块（已停/已调出）不可拖。

### 决策 5：ICS override 展开映射（批 2）

- **cancelled → 不生成 VEVENT**（否决 STATUS:CANCELLED）：导出目的是日历提醒上课，带 STATUS:CANCELLED 的事件在 Outlook/Google 仍显示条目徒增噪音；上游 ICS 引擎也是按生效结果跳过（清单§2.8）。取消课从导出剔除，与网格「已停占位」分工：网格留痕、日历只要事实。
- **rescheduled**：原时段不生成；新时段生成（时间/节次/教室取 new_*，DESCRIPTION 追加「调课」），UID 用 `course.id-w{week}d{newDay}s{newStart}@campushub` 保证与原时段 UID 不同（防日历端去重错乱）。
- **extra**：新时段生成，DESCRIPTION 追加「补课」。
- **多条 override 叠加**：逆序取最后一条、停课优先于调课——**逐分支对照前端 `buildWeekBlocks`（`TimetablePanel.tsx:168-253`）抄语义**（见风险 R2）。
- **VTIMEZONE**：维持现状 floating local time（无 TZID/Z），只动 VEVENT 生成逻辑，不引入 VTIMEZONE 块。
- **共享纯函数**：`crates/campus-schedule/src/occurrence.rs` 新建：

```rust
pub enum OccurrenceKind { Solid, MovedOut, Cancelled }
pub struct CourseOccurrence {
    pub kind: OccurrenceKind,
    pub day: u8,
    pub start_section: Option<u8>,  // Solid/MovedOut 的节次；None 仅当课程本身 custom
    pub end_section: Option<u8>,
    pub position: String,
}
/// 课程在某周经 override 展开后的全部实例（含 ghost 供前端语义对照；
/// ICS/今日页只消费 Solid）。语义必须与前端 buildWeekBlocks 对齐，
/// 两处注释互锚：改一处必须同步另一处。
pub fn expand_occurrences(course: &Course, overrides: &[CourseOverride], week: u32) -> Vec<CourseOccurrence>
```

前端 TS 无法调 Rust 纯函数，`buildWeekBlocks` 保持自有实现（成熟且过 UI 验收，重写风险更大）——**接受受控双写**：Rust 侧单测钉住语义 + 两侧注释互锚。ICS 与今日页（批 9）共用 Rust 侧，避免三写。

## 二、批次详表

### 批 1（P1）：设置与周首日接线

**范围**：showWeekends / firstDayOfWeek 前端消费（列头旋转 + 5 列裁剪 + 约束联动）；开学日/总周数/当前周手动设置整块（复核并入项 2——`semester_start_from_week`（`weeks.rs:55-62`）死代码激活）；ICS 日期周首日对齐（复核并入项 4）。

**后端改动**：
- `crates/campus-schedule/src/weeks.rs:14`：`previous_or_same_day_of_week` 改 `pub`；`lib.rs` 补 re-export。
- `tauri-app/src-tauri/src/commands/timetable.rs`：
  - 新命令 `save_semester_config(input: SemesterConfigInput) -> TimetableView`（返回刷新视图，沿 save_time_slots 惯例）。`SemesterConfigInput { semester_start_date: Option<NaiveDate>, semester_total_weeks: u32, first_day_of_week: u8, show_weekends: bool }`（serde camelCase；`semester_start_date = None` = 清空开学日即假期态）。校验：周数 1..=30（上游滚轮范围，清单§11）、firstDay 1..=7、日期合法。
  - 约束联动收口后端单点 `apply_display_constraints(&mut CourseTableConfig)`：`first_day_of_week == 7 → show_weekends = true`；`!show_weekends → first_day_of_week = 1`（上游双向联动，清单§1）。save_semester_config 内调用；前端只做提示不禁用。
  - `build_ics`（:353-429）日期推导改对齐式：`week_first = previous_or_same_day_of_week(start_date, first_day_of_week)`；`col = (day - firstDay + 7) % 7`；`date = week_first + (week-1)*7 + col`。firstDay=1 且开学日为周一时与现式等价（golden 保）；补「开学日非周一」「firstDay=7」两个新测试。
  - lib.rs 注册命令。

**前端改动**（`TimetablePanel.tsx` + `types.ts`）：
- 显示列序列：`displayDays = showWeekends ? 7 : 5` 列，列头 = 从 firstDay 起 rotate 的 DAY_HEADERS（firstDay=7 → 周日起）。
- `weekDates`（:765-773）重写：`weekFirst = previousOrSame(semesterStartDate, firstDay)`（JS 纯函数，`getDay()` 折算，注释锚定 weeks.rs 语义）+ `(week-1)*7 + i`；`todayCol = weekDates.findIndex(== today)`。
- **day → 显示列映射**：`colIdx = (day - firstDay + 7) % 7`，`colIdx >= displayDays` 的课不渲染；`buildWeekBlocks` 内部 columns 保持 7 天计算不动，渲染层映射（最短 diff）。`gridTemplateColumns` 动态 `56px repeat(n, ...)`。
- 「课表设置」弹层（新组件，顶栏「设置」按钮，仅 ready 态）：开学日 `<input type="date">` + 清空按钮；总周数 number 1-30；「今天是第 N 周」（number，保存时调 save_semester_config 且 start_date 用后端反推——**反推放后端**：input 增可选 `current_week_hint: Option<u32>`，save 命令内 `semester_start_from_week(today, hint, firstDay)` 计算后回填 start_date，口径单点）；每周起始日 select（周一/周日）；显示周末 switch（联动提示）。
- `types.ts`：`SemesterConfigInput` 接口。

**验收**：firstDay=周日 → 列头周日起且周末强制显示；关周末 → firstDay 自动回周一、网格 5 列、周六日课静默不渲染；设置当前周为 5 → 顶栏周次/列头日期随之正确；ICS firstDay=1 输出与旧版逐字节一致、firstDay=7 日期对齐；旧 JSON 打开无损；深色模式不破。

**实施顺序**：后端先行（命令+联动+ICS 对齐+单测）→ 前端。
**提交切分**：① `feat(timetable): save_semester_config 与显示约束联动` ② `feat(timetable): ICS 周首日对齐` ③ `feat(timetable): 前端周末列/周首日消费与课表设置弹层`。

### 批 2（P2）：skippedDates + ICS 按生效结果展开

**范围**：跳过日期链路（手动维护，冻结决策 4，不接第三方 API）；ICS 消费 override（复核并入项 1）；ICS 接自定义时间课（复核附注 5）；`effective_slots_at` 签名预留（决策 2）。

**后端**：
- `crates/campus-schedule/src/model.rs`：`CourseTableConfig` 增 `#[serde(default)] pub skipped_dates: Vec<NaiveDate>`（序列化 `skippedDates: string[]`）。
- 新建 `crates/campus-schedule/src/occurrence.rs`（决策 5 的 `expand_occurrences` + 全语义单测：停课两档（newDay 有/无）、调课跨天（moved-out+solid 两条）、仅换教室（原位 solid 新教室）、补课、多 override 逆序取最后、停课优先调课）；`lib.rs` 导出。
- `commands/timetable.rs`：
  - `effective_slots(config)` → `effective_slots_at(config, date)`（P2 阶段忽略 date，注释 P3 扩展点）；`build_timetable_view` 传 today；`build_ics` 逐事件日期取值。
  - `build_ics` 改造：对每门未 disabled 课程调 `expand_occurrences(course, &tt.overrides, week)` 只取 `Solid`；每个 VEVENT 日期先查 `skipped_dates` 命中则跳过；时刻取值——节次课查该日 slots（大节 = `(s+1)/2` 不变），**custom 课直接 DTSTART/DTEND 取 `custom_start_time/custom_end_time`**（替掉 :373-375 的 continue）；UID/调课/补课规则见决策 5。
  - 新命令 `save_skipped_dates(dates: Vec<NaiveDate>) -> TimetableView`（整体替换；serde camelCase 入参 `dates`）。

**前端**：
- `TimetablePanel.tsx`：网格列头日期命中 `config.skippedDates` → 该列课程不渲染 + 列头加「休」小徽标 + 日期置灰（读 `timetable.config.skippedDates`，TimetableView 已含全量，零契约变更）；课表设置弹层加「跳过日期」区块（date input + 已选列表增删，YAGNI 不做日历面板）。
- `types.ts`：`CourseTableConfig.skippedDates: string[]`。

**验收**：标记 10-01 → 网格该列隐藏课 +「休」标 + ICS 无 10-01 VEVENT；采纳第 5 周停课通知 → ICS 第 5 周对应 VEVENT 消失；调课 → 原时段消失新时段出现（时间/教室正确）；补课 → 新增；custom 课（模型层构造）→ DTSTART 取 custom 时刻；expand_occurrences 与 build_ics 组合单测全绿；旧 JSON 无损。

**实施顺序**：campus-schedule 纯函数先行 → ICS → 命令 → 前端。
**提交**：① `feat(schedule): expand_occurrences override 展开纯函数` ② `feat(timetable): ICS 按生效结果展开（override/跳过日/自定义时间）` ③ `feat(timetable): skippedDates 字段命令与网格休标`。

### 批 3（P3）：组合作息

**范围**：决策 2 全量。`SlotRule` 模型、`effective_slots_at` 区间命中实现、`save_slot_rules` 命令、SlotsEditor 规则区块。ICS/网格/今日页经收敛点自动获得。

**后端**：`model.rs`（SlotRule + `slot_rules` 字段，serde default）；`commands/timetable.rs`（`effective_slots_at` 实现 + `validate_slot_rules`（复用 validate_time_slots + start<=end + 规则内 slots 非空）+ `save_slot_rules(rules: Option<Vec<SlotRule>>) -> TimetableView`）；lib.rs 注册。
**前端**：`TimetablePanel.tsx` SlotsEditor 加「按日期生效的作息规则」区块（每规则：起止两个 date input + 作息行编辑（复用现有行组件）+ 删除；「新增规则」按钮；主作息区块保留）；`types.ts` `SlotRule`。

**验收**：建「2026-12-01 起冬季作息」规则 → 12-01 前后导出 ICS 的 VEVENT 时刻分别按新旧作息；`build_timetable_view` 传不同 today 断言行数变化（单测）；区间重叠取首个（单测）；回落链 rules→config.slots→内置（单测）；旧 JSON 无 `slotRules` 打开无损。

**提交**：① `feat(timetable): slot_rules 模型与区间命中` ② `feat(timetable): save_slot_rules 命令与规则编辑区块`。

### 批 4（P4）：网格拖拽改课

**范围**：决策 4 全量，纯前端（零新命令、零后端改动）。

**改动**（`TimetablePanel.tsx`，约 +150 行）：拖拽状态机（pointerdown/move/up + 4px 阈值 + suppressClick + Esc 取消）；预览（原块半透明 + 目标虚线轮廓 + 落点列高亮）；落点几何（决策 4 公式，含显示列 → 实际星期的旋转反映射）；落库分叉（多周/导入 → 构造 candidate 调 `apply_override`；单周手动 → `update_course`；位置未变短路）；调整列表对 `drag:` 前缀来源显示「拖拽」标签（`overrideSummary` 附近小改）。

**验收**：导入课从周一 1-2 拖到周三 3-4 → 原位「已调出」虚块 + 新位实体块 + 调整列表出现 drag 项 + 撤销复原；1-16 周多周课拖拽只影响展示周（翻其他周验证原样）；单周手动课拖拽直改无 override；纯点击仍开详情、拖后不触发点击；ghost 块不可拖。

**实施顺序**：前端先行（唯一批次）。
**提交**：① `feat(timetable): 网格拖拽改课（override/直改双路径）`。

### 批 5（P5-a）：表单与编辑体验（5 条小件）

1. **手动课程自定义时间**（清单§5）：CourseForm 加「按时刻」开关（`source === "import"` 的编辑隐藏——导入课永远节次制）；开启后小节 select 换两个 `<input type="time">`；`ManualCourseInput` 增可选 `is_custom_time: bool`（default false）/`custom_start_time`/`custom_end_time`（Option，serde default，TS 可选）；`add_course_manual` 接线（替掉 :270-272 硬编码）；`submitCourse` 删 `isCustomTime: false` 硬编码（:883）；后端校验 custom 模式起止必填且 end > start；**网格渲染**：`buildWeekBlocks` 对 custom 课按「与各大节区间相交的 min..max 大节」落块（新增纯函数 `customBlockRange(hm1, hm2, slots)`，无相交大节不渲染、仅列表可见），时间非法跳过；ICS 侧批 2 已接。
2. **节次别名**（清单§1/§3）：SlotsEditor 每行加 alias 输入（maxlength 5，「别名（选填）」；state 已有）；保存随 `TimeSlot.alias` 透传（后端零改动）；时间列节号下有 alias 显示小字。
3. **未保存退出拦截**（清单§5）：CourseForm 与 SlotsEditor dirty 检测（state 与 initial 浅比较足够）；取消/Esc/遮罩关闭时 dirty → `window.confirm("放弃未保存的修改？")`。
4. **备注 300 字计数**（清单§1）：remark 换 textarea + maxLength 300 + 「N/300」计数；后端 add/update 各 `chars().take(300)` 截断（双保险）。
5. **单双周文案后缀**（清单§2.7）：TS `fmtWeeks`（:77-94）与 Rust `diff.rs:103` `format_weeks` 同步：全奇数且 ≥3 → `1-7(单周)`，全偶数 → `(双周)`，其余不变（两侧语义一致防漂移，各一处小 diff）。

**验收**：custom 开关保存后网格按相交大节渲染 + ICS 时刻正确；alias 显示；dirty Esc 弹 confirm、非 dirty 直关；301 字备注截 300；`1,3,5,7` 显示「1-7(单周)」；旧 JSON 无损（ManualCourseInput 可选字段）。

**提交**：① `feat(timetable): 手动课程自定义时间` ② `feat(timetable): 表单三小件（别名/dirty 拦截/备注计数）` ③ `feat(timetable): 周次单双周文案后缀`。

### 批 6（P5-b）：导入导出（3 条小件）

1. **单表 JSON 导出**：`export_timetable_json() -> String`（后端写下载目录 `课表.json` 返回路径，复用 export_ics 交付模式；内容 `{ formatVersion: 1, courses, overrides, config }`——**含 overrides**（本仓特有，丢了恢复不完整））。
2. **单表 JSON 导入**：`import_timetable_json(json: String) -> ImportResult`；校验（节次课 `1 <= start <= end`、custom 课 HH:MM 合法且起止齐全、config 内 slots/rules 走既有校验，非法中文报错不落库）；语义：**清空当前表 courses + overrides 后整体写入（一次 save，禁止先清后写两次落盘）**；config 字段「非空才覆盖」（上游语义，清单§7），缺省字段不抹旧值；colorIndex 原样采用。
3. **ICS VALARM**（清单§7）：`export_ics` 增可选参 `remind_minutes: Option<u8>`（serde default；0..=60 校验，越界报错）；有值时每 VEVENT 追加 `BEGIN:VALARM / ACTION:DISPLAY / TRIGGER:-PT{n}M / DESCRIPTION:课前提醒 / END:VALARM`；前端导出区加提醒选择（无/15/30/60，select + 导出按钮）。

**验收**：导出 → 清空数据目录 → 导入 → 课程/override/config（含自定义作息、skipped、rules）完整恢复；坏 JSON 报错且原库无损；VALARM=15 的 ICS 含 `TRIGGER:-PT15M`。

**提交**：① `feat(timetable): 课表 JSON 导入导出` ② `feat(timetable): ICS 课前提醒 VALARM`。

### 批 7（P5-c）：顶栏与渲染小件（4 条小件）

1. **周次选择弹窗**（清单§4）：顶栏「第 N 周 / 共 M 周」文本变按钮 → 弹层 1..totalWeeks 网格（10 列 wrap；当前周实底高亮、选中周描边；点选 `setViewWeek` 并关闭）；◀/本周/▶ 保留。
2. **顶栏标题态机**（清单§4，含复核②今日页副标题扩展）：`TimetableView` 增 `week_state`（后端在 `build_timetable_view` 的 current_week 处顺手产出：无开学日 → `unset`；`week_index < 1` → `before`；`> total` → `vacation`（学期末日 = `week_first + total*7 - 1`，清单§2.11）；否则 `normal`）。前端：unset → 「尚未设置开学日」（点击开设置弹层）；before → 「距离开学还有 N 天」；vacation → 「假期」。TS 镜像 `weekState: WeekState`。
3. **非本周课程降级显示**（清单§4）：`CourseTableConfig` 增 `#[serde(default)] pub show_non_current_week: bool`（false = 现状隐藏）；true 时 `buildWeekBlocks` 不过滤非本周课，渲染 `opacity-40` 降级样式（可点详情、不可拖）；设置弹层加开关。
4. **颜色池自定义（最简）**：`uiStore` persist 加 `customCourseColors: string[]`；色板 = 8 固定 + 自定义段；CourseForm 色板区「+」按钮弹原生 `<input type="color">`（不引取色器库）；手动课 colorIndex 语义 = 合成色板下标，导入课取色 `% 合成长度`。

**验收**：弹窗点第 10 周跳转且当前周高亮；三态文案正确切换；降级开关生效且降级块不可拖；自定义色刷新后仍在、手动课可选；旧 JSON 无损。

**提交**：① `feat(timetable): 周次选择弹窗与顶栏标题态机` ② `feat(timetable): 非本周课程降级显示` ③ `feat(timetable): 课程色板自定义扩展`。

### 批 8（P5-d）：调课搬迁 + 快速删除

1. **调课搬迁**（清单§13，三模式简化为「移动」单模式）：新命令 `move_day_courses(from_date: NaiveDate, to_date: NaiveDate) -> MoveResult { moved: u32, notice_id: String }`。语义：源/目标日期各自经 `week_index_at_date` 对齐算出 (week, day)（**禁用 epoch 直除**——上游三处口径不一致的坑勿照抄，清单§2.4）；同一天或未设学期报错；对「源 (week, day) 有课」的每门课生成 Rescheduled override（`weeks = [目标周]`、`new_day = 目标星期`、节次原值、`source_notice_id = "move:<from>:<to>"` 一批）——**统一 override（含手动课）**：批量操作可整批撤销，且与 P4 语义同链路。UI：设置弹层「数据」区入口 → 两 date input + 双侧课程数预览 + 执行。
2. **快速删除**（清单§13）：新命令 `quick_delete(weeks: Vec<u32>, days: Vec<u8>) -> u32`。语义：对每门课，从 `weeks` 移除「选中周 × 选中星期」命中组合（导入/手动/disabled 都可清）；**weeks 删空 → 删除整条课程**（上游「只删周次关联」，本仓无关联表，空 weeks 僵尸记录不如删净；delete_course 级联清 override 的既有链路自动生效）。日期区间第二维度**不做**（周次×星期够用，YAGNI）。UI：弹层周次多选 chips + 星期多选 + 实时预览受影响课程数 + 确认。

**验收**：第 6 周五 3 门课搬到第 7 周一 → 目标位出现 3 实体块 + 原位虚线 + 整批撤销恢复；快删「第 10 周×周三」→ 仅该组合移除、多周课其他周保留、单周课整条删除；预览数与实际一致。

**提交**：① `feat(timetable): 调课搬迁（批量 override）` ② `feat(timetable): 快速删除（周次×星期筛选）`。

### 批 9（P5-e）：今日页接本地课表

**范围**：决策 3 全量。新命令 `get_today_courses() -> TodayCoursesView`：

```rust
pub struct TodayCoursesView {
    pub date: String,               // "YYYY-MM-DD"
    pub current_week: Option<u32>,
    pub state: String,              // "normal" | "no_semester" | "vacation" | "skipped"
    pub has_local: bool,            // 本地课表已导入且有课程
    pub courses: Vec<TodayCourse>,  // 升序；停课不进列表
    pub next: Option<TodayCourse>,  // 第一门未结束
}
pub struct TodayCourse {
    pub course_id: String, pub name: String, pub room: String, pub teacher: String,
    pub start_hm: String, pub end_hm: String,  // "HH:MM"
    pub ongoing: bool,                            // 后端按 now 算好
}
```

**后端**：`commands/timetable.rs`（`today_courses(tt, now)` 纯函数 + 命令接线 + 单测：override 展开过滤、custom 时刻、skipped/vacation/no_semester 三态、next 与 ongoing 边界）；lib.rs 注册。
**前端**：`TodayPanel.tsx`——authed 时 `get_portal_overview`（钱包/学期）与 `get_today_courses` 并行（互不阻塞、各自四态）；`has_local` → 「下一节课」横幅与新增「今日课程」列表（每行 `HH:MM-HH:MM 课名 @教室`；ongoing 高亮、已结束置灰；不做时间轴刻度）用本地，state=skipped/vacation 显对应文案；`has_local == false` → 现状门户 nextCourse 不变。`types.ts` 镜像。

**验收**：本地含 override/停开/手动/custom/skipped 时列表与 next 准确（调课后 next = 新时段）；未导入回落门户（现状回归零变化）；放假日「今日放假」；已结束置灰、进行中高亮。

**提交**：① `feat(timetable): get_today_courses 本地今日视图` ② `feat(frontend): 今日页接本地课表`。

### 批 10（P5-F）：多课表（大件）

**范围**：决策 1 全量。存储 v2 + 懒迁移 + 5 命令 + 切换 UI。

**后端**：`infra/timetable.rs` 重写（`TimetableStore`/`TableEntry`/`load_store`（v1 探测迁移）/`save_store`/`empty_table(id)`；`mutate_timetable` → `mutate_current_table` 骨架替换，**所有既有写命令函数体零改动**）；`import_timetable` 的 `parse_kb_response(json, 当前表id)`；新命令：

| 命令 | 签名 | 语义 |
|---|---|---|
| `list_course_tables` | `() -> Vec<TableBrief>` | `{ id, name, created_at }` |
| `create_course_table` | `(name: String) -> TimetableView` | 建空表（config.slots=None 即内置作息）+ 自动切换 |
| `rename_course_table` | `(id: String, name: String) -> ()` | 名非空校验 |
| `delete_course_table` | `(id: String) -> TimetableView` | 禁删最后一张；删当前自动切首张；返回当前表视图 |
| `switch_course_table` | `(id: String) -> TimetableView` | 未知 id 报错 |

`TimetableView` 增 `table: TableBrief`。

**前端**：顶栏当前表名按钮 → 切换弹层（卡片列表带创建时间（上游复核④）+ 当前标记 + 新建/重命名（confirm 输入）/删除（confirm））；`types.ts` 镜像。

**验收**：旧 v1 文件打开 → 「默认课表」一切如旧（含批 1-9 全部特性）；建第二张表 → 切换、课程/作息/overrides/skipped/rules 完全隔离；导入落当前表；删除当前表自动切换；v2 roundtrip 单测；损坏文件回空 store 不删现场。

**提交**：① `feat(timetable): 多课表存储 v2 与懒迁移` ② `feat(timetable): 课表管理五命令` ③ `feat(frontend): 课表切换器`。

### 批 11（P5-G）：多方案编辑 + 全局课程管理（大件）

**范围**（清单§5、§14）：**零新命令**（纯前端组装既有 IPC）。

1. **多方案编辑**：详情浮层「编辑」进入时按课程名聚合当前表同名记录为方案集——表单顶部方案 chips（第 1..N 方案 + 「+ 新方案」）；切 chip 换表单 initial；新方案预填上一方案的教师/教室/备注/颜色（上游继承语义）、周次默认全选；保存 = 与初始集对比，逐条串行调 `update_course` / `add_course_manual` / `delete_course`（任一失败中断报错可重试，幂等性足够；体验不佳的升级路径 = 将来加批量命令 `save_course_schemes`，本轮 YAGNI）；色板区加「应用到全部方案」（把 colorIndex 写到全部同名课）。**diff 语义不受影响**：同名多条 Import 记录是不同教学班（class_id 不同），匹配键「名+class_id」各自独立——写进实现注释防执行者疑惑。
2. **全局课程管理**：设置弹层「数据」区「课程管理」入口 → 大弹层：左列课程名（去重 + 实例数 badge，字母序）；右列该名全部实例（星期/节次/周次摘要）多选删除；按名批量删 = 循环 `delete_course`（confirm 列出总数；override 级联由既有命令处理）。

**验收**：同名两方案改 A 不影响 B；新方案继承字段；「应用到全部」改色全变；按名删除清掉全部同名记录及其 override；详情浮层保存后状态刷新无残留。

**提交**：① `feat(frontend): 课程多方案编辑` ② `feat(frontend): 全局课程管理`。

## 三、P5 小件处置对照表（18 条全覆盖，含明确不做）

| HANDOFF P5 条目 | 归属 | 处置 |
|---|---|---|
| 手动调课搬迁 | 批 8 | 移动单模式，批量 override |
| 快速删除 | 批 8 | 周次×星期，weeks 删空删整条 |
| 单表 JSON 导入 | 批 6 | 见规格 |
| 全量备份/恢复 | 批 6 | 与 JSON 导出合并（WebDAV 留安卓储备） |
| 单双周文案后缀 | 批 5 | 前后端同步 |
| 非本周降级显示 | 批 7 | 开关 + 40% 降级 |
| 颜色池自定义 | 批 7 | 最简（原生 color input + uiStore） |
| 24h 绝对时间轴 | **不做** | 校本大节制无意义（HANDOFF 原文）；用户点名再启，届时启用 grid.rs Time24h 预留件 |
| 今日页接本地课表 | 批 9 | 决策 3 |
| 周次选择弹窗 | 批 7 | 见规格 |
| 顶栏标题态机 | 批 7 | weekState 后端下发 |
| 手动课程自定义时间 | 批 5（ICS 侧批 2） | 见规格 |
| 节次别名编辑 | 批 5 | 见规格 |
| 表单未保存拦截 | 批 5 | confirm 级 |
| 备注 300 字计数 | 批 5 | 双保险 |
| ICS VALARM | 批 6 | 见规格 |
| 多课表管理 | 批 10 | 决策 1 |
| 多方案编辑 + 全局课程管理 | 批 11 | 零新命令 |

## 四、风险清单（每批最易做错的一点）

- **批 1**：`todayCol` 必须对「显示列的 weekDates」findIndex，不能按 day 数学映射（旋转后必错）；ICS 对齐会让「开学日非周一」场景的既有导出变化——这是修复不是回归，需新测试钉住且周一开学 golden 必须不变；联动收口在后端 save（前端两处开关各自联动必漏）。
- **批 2**：`expand_occurrences` 与前端 `buildWeekBlocks`（:168-253）语义漂移——停课两档、仅换教室原位、逆序取最后、停课优先调课，必须逐分支对照抄并注释互锚；调课后新时段 UID 必须与原 UID 不同（日历端去重错乱）。
- **批 3**：`TimetableView.slots` 取今天而非视图周（跨区间换季周的已知取舍，注释写明）；回落链顺序（rules → config.slots → 内置）单测钉死；批 2 若没预留 date 参数，本批要改全部调用点（返工）。
- **批 4**：pointer capture 后 click 仍触发（suppress 标志位）；落点天列的旋转反映射（firstDay≠1 时 `colIdx → day` 方向易反）；分叉条件精确为 `weeks.length === 1 && source === "manual"` 才直改。
- **批 5**：custom 课网格「相交大节」边界（时刻恰在大节端点、课间空隙无相交 → 不渲染仅列表）；`buildWeekBlocks` 对 custom 时间非法的跳过要保留。
- **批 6**：导入「清空再写」必须一次 `save_timetable`（先清后写两次落盘，中间失败丢数据）；导入 config「非空才覆盖」，缺省字段不得抹旧值。
- **批 7**：weekState 三态判定口径（before = `week_index < 1`；vacation = `> total`；末日语义 `week_first + total*7 - 1`）；非本周降级块与 ghost 块叠加时的渲染优先级（ghost 本身非实体，降级只作用于 solid）。
- **批 8**：搬迁的「当周」必须 `week_index_at_date` 对齐 firstDay（上游三处口径不一致的坑，HANDOFF 已警告）；quick_delete 仅删部分周次时残留 override 无害（其 weeks 不含被删周即不生效），说明即可不清理。
- **批 9**：本地/门户双 Promise 竞态（各自 alive 标志 + 互不阻塞）；ongoing/next 全由后端算（前端不自算时钟）。
- **批 10**：v1/v2 探测键判定（`tables` 存在与否，v1 顶层仅 config/courses/overrides/updatedAt，安全）；`mutate_current_table` 骨架替换后**全部既有写命令逐一回归**（add/update/delete/apply/revoke/save_time_slots/save_semester_config/save_skipped_dates/save_slot_rules/import/move/quick_delete + JSON 导入）；契约文档与 wiki 同步 v2。
- **批 11**：多方案保存的删除/新增/修改混合序列中断后的部分成功态（报错 + 提示重试即可，不做事务）；保存后详情浮层与新方案的 stale state 清理。

## 五、依赖图与执行建议

```
批1 → 批2 → 批3 → 批4 → {批5, 批6, 批7（文件域几乎不相交）} → 批8（依赖4）→ 批9（依赖2+5）→ 批10 → 批11
```

- 批 5/6/7 虽文件域近于不相交，但同项目写入按 AGENTS.md 纪律**串行**派发。
- 交叉复核点：批 2（expand_occurrences 语义对照）、批 4（拖拽分叉与 suppressClick）、批 10（迁移与骨架替换）三批完成后各派一次交叉复核。
- 每批执行分身的 prompt 应附：本蓝图对应批次全文 + 契约文档对应修订段 + 「先写契约修订再动码」的指令。
