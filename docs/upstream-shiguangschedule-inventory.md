# 上游项目 shiguangschedule 全量功能与设计清单（存档）

> 调研时间：2026-09-19 · 方式：deepseek-flash 通读上游源码/配置/proto/资源（未运行构建），
> 主智能体抽查核实关键字段结论（`CourseTableConfig` 字段、`format_weeks`、`stable_color`、
> reminder/节假日/壁纸/调课搬迁在本仓零命中等）。
> 所有路径相对上游仓库根 `E:\ik\Documents\trae_projects\1\shiguangschedule`；`文件:行号` 均为上游证据。
>
> **用途**：① 本仓课表（M2.5）功能对账的对照底稿（对账结论见 `docs/HANDOFF-timetable-parity.md`）；
> ② **未来 CampusHub 出安卓端时的设计参考**（桌面小组件、课程提醒、勿扰自动化、系统日历同步等
> 平台强绑定设计全部收录在此，桌面端不实现但不丢弃——用户 2026-09-19 明确指示）。
>
> 上游定位：Kotlin Multiplatform（KMP）+ Compose Multiplatform 通用课表 App（Android 为主，
> desktop/iOS 为壳），versionName 2.0.1。

---

## 0. 总体架构摘要

- **模块**：`:shared`（KMP 共享层，源集 `commonMain` / `androidMain` / `jvmMain` / `iosMain`）、`:androidApp`（minSdk 26 / targetSdk 37，ABI 三分包）｜`settings.gradle.kts:25-27`、`androidApp/build.gradle.kts:19-21,45-53`
- **分层**：`data/db`（Room3 两个库）→ `data/repository`（10 个仓库）→ `ui/<功能域>/{Screen,ViewModel}`；横切 `data/model`（业务模型 + proto 映射）、`data/api`（Ktor 网络）、`data/sync`（主库→小组件库同步）、`tool`（跨平台工具）、`data/di`（Koin 模块）
- **关键库**：Compose Multiplatform 1.11.1 + Material3 + material-kolor（动态取色）、Navigation3、Koin 4.2.2、Room 3.0.2（主库 v6 渐进迁移 + 小组件库 v3 破坏迁移）、DataStore（Preferences ×3 + Okio Proto ×1）、Wire 6.4.5（3 个 `.proto`）、Ktor 3.5.2、kotlinx-datetime、kgit（JGit 移植，拉适配脚本仓库）、WorkManager、AppCompat（应用内语言切换）
- **平台现状**：Android 能力最全（4 种桌面小组件、AlarmManager 闹钟、WorkManager、系统日历 Provider、勿扰/静音、AndroidKeyStore、WebView+JS Bridge）；desktop 仅一个窗口跑同一 `App()`；iOS 仅有壳（`ZipUtils` 抛 `NotImplementedError`）
- **App 外壳**：3 个一级 Tab（今日课表 / 课表 / 我的），Navigation3 类型安全路由，自适应导航（窄屏底栏 / 宽屏 NavigationRail）｜`Navigation.kt:29-31`、`ui/components/NavigationComponents.kt:136-140`

## 1. 数据模型与持久化

- **Course 实体**（table `courses`）——`id`(UUID 主键)、`courseTableId`、`name`、`teacher`、`position`、`day`(1-7)、`startSection?`/`endSection?`（自定义时间时可为空）、`isCustomTime`、`customStartTime?`/`customEndTime?`("HH:MM")、`colorInt`(颜色池索引)、`remark?`(≤300 字)；外键 CASCADE｜`data/db/main/Course.kt:13-40`
- **CourseWeek 关联表**（`course_weeks`）——联合主键 `(courseId, weekNumber)`，课程↔周次多对多（单双周/指定周次都展开为周次集合，无掩码常量）｜`data/db/main/CourseWeek.kt:11-30`
- **CourseTable 课表元数据**（`course_tables`）——`id`/`name`/`createdAt`｜`data/db/main/CourseTable.kt:10-16`
- **CourseTableConfig**（1:1）——`showWeekends`(默认 false)、`semesterStartDate?`、`semesterTotalWeeks`(默认 20)、`firstDayOfWeek`(默认 1=周一)｜`data/db/main/CourseTableConfig.kt:28-36`
- **TimeSlot**（`time_slots`）——联合主键 `(timeTableId, number)`、`startTime`/`endTime`("HH:MM")、`alias?`(≤5 字别名，如"早自习")｜`data/db/main/TimeSlot.kt:11-30`
- **TimeTable 作息方案**（`time_tables`）——专属作息 id=courseTableId 且 name 为 null；公共作息独立 UUID 且 name 非空；`defaultClassDuration`(45min)/`defaultBreakDuration`(10min)｜`data/db/main/TimeTable.kt:6-19`
- **CourseTimeBinding**（`course_time_bindings`）——课表↔作息绑定，`targetType`(SINGLE/COMBO)、`targetId == courseTableId` 即专属作息｜`data/db/main/CourseTimeBinding.kt:12-36`
- **TimeTableCombo 组合作息**（`time_table_combos`）——`baseTimeTableId?`（null=动态专属作息为基准）｜`data/db/main/TimeTableCombo.kt:11-29`
- **TimeTableComboRule**（`time_table_combo_rules`）——`targetTimeTableId`(RESTRICT)、`startDate`/`endDate`，按日期区间命中｜`data/db/main/TimeTableCombo.kt:34-59`
- **Room 迁移链**（主库 v6）——手写 1_2/2_3/5_6 + 自动 3→4→5；schema 导出 `shared/schemas/`｜`data/db/main/DatabaseMigrations.kt:14-330`、`MainAppDatabase.kt:12-35`
- **DAO 排序（混合时间排序）**——`day ASC, (isCustomTime=0? startSection : 99), (isCustomTime=1? customStartTime : '99:99')`｜`data/db/main/CourseDao.kt:23-35`
- **课程写入策略**——`exists()` 后 `@Update`（避免 REPLACE 级联删除）或 `@Insert(ABORT)`；周次整体替换（事务内先删后插）｜`CourseDao.kt:59-77`、`CourseTableRepository.kt:166-188`
- **备注 300 字双保险**——仓库层截断 + UI 层计数截断｜`CourseTableRepository.kt:168-172`、`AddEditCourseViewModel.kt:246-250`
- **首个课表自动种子**——空库建"我的课表"+默认配置+13 节默认作息｜`CourseTableRepository.kt:41-85`
- **默认 13 节常量**——08:00 起含午休晚课｜`CourseTableRepository.kt:382-396`
- **DataStore 持久化**——Preferences（app_settings / school_history / api_config）+ Proto（`schedule_style_settings.pb`）｜`data/di/DataStoreModule.kt:21-63`
- **AppSettingsModel 全字段**——`currentCourseTableId`、`reminderEnabled`、`remindBeforeMinutes`(15)、`skippedDates:Set<String>`、`autoModeEnabled`、`autoControlMode`(DND/SILENT)、`compatWearableSync`、`showNonCurrentWeekCourses`、`startScreen`、`themeMode`(FOLLOW_SYSTEM/LIGHT/DARK)、`useDynamicColor`、`customLightPrimary`/`customDarkPrimary`、`developerModeEnabled`｜`data/model/AppSettingsModel.kt:17-165`
- **配置约束联动**——`firstDayOfWeek=周日 ⇒ showWeekends=true`，`!showWeekends ⇒ firstDayOfWeek=周一`｜`AppSettingsRepository.kt:52-58,120-131`
- **小组件专用库**（WidgetDatabase v3）——`WidgetCourse`(id=`课程id-日期`、含 `isSkipped`)、`WidgetAppSettings`(单行)｜`data/db/widget/WidgetCourse.kt:10-22`、`WidgetAppSettings.kt:8-15`

## 2. 时间 / 周次算法

1. **周次偏移核心 `getWeekIndexAtDate`**——开学日与目标日都对齐到每周首日（`firstDayOfWeek`），`(alignedTarget - alignedStart)/7 + 1`；对齐用分支法 `currentDay >= targetDay ? currentDay-targetDay : 7-(targetDay-currentDay)`；开学前会得 ≤0，调用方以 `in 1..totalWeeks` 过滤｜`AppSettingsRepository.kt:138-224`
2. **反推开学日 `calculateSemesterStartDate`**——`startOfThisWeek - (week-1)*7`；传 null=清空（放假）｜`AppSettingsRepository.kt:183-210`
3. **当前周 `calculateCurrentWeekFromDb`**——仅 `1..totalWeeks` 内返回，否则 null｜`AppSettingsRepository.kt:163-178`
4. ⚠️ **三处周次换算口径不一致**（上游自身缺陷，迁移时勿照抄）：调课页/快速删除页用 `LocalDate.until(date, WEEK)+1`（未对齐周首日）｜`TweakScheduleViewModel.kt:124-127`、`QuickDeleteViewModel.kt:176-179`
5. **24h 时间轴与节次模式坐标互转**——`timeToGridScale`（24h：`1+分钟/60`；节次：命中节次 `number+已过分钟/时长`，早于首节 1.0，晚于末节 `size+1`）与逆函数 `gridScaleToTime`｜`WeeklyScheduleViewModel.kt:324-409`
6. **当前节次高亮**——时刻落在 `[start,end)` 的节次序号｜`WeeklyScheduleViewModel.kt:278-297`
7. **周次集合→文案 `formatWeeks`**——识别等差 2（`(单周)`/`(双周)` 后缀）、连续区间（`3-8`）、孤立周｜`CourseDetailBottomSheet.kt:442-476`
8. **ICS 遍历引擎 `processCourseInstances`**——对齐学期首周→每课每周算真实日期→跳过超总周数→跳过 `skippedDates`→按当天生效作息取时间（自定义时间课直接取）｜`tool/IcsExportTool.kt:32-102`
9. **ICS 生成细节**——`PRODID:-//ShiGuangSchedule//ZH`、硬编码 `VTIMEZONE Asia/Shanghai`、`UID=UUID@shiguangschedule.com`、`VALARM TRIGGER:-PT{n}M`（仅 0..60）、文本转义 `\ ; , \n`｜`IcsExportTool.kt:107-201`
10. **假日导入**——Ktor GET `https://timor.tech/api/holiday/year`，过滤 `isHoliday` 写入 `skippedDates`（UA 伪装移动 Chrome）｜`data/api/date/ApiDateImporter.kt:33-81`
11. **学期末日判定**——`firstWeekStart + totalWeeks*7 - 1`｜`TodayScheduleScreen.kt:170-183`

## 3. 作息体系：专属 / 公共 / 组合

- **三类并存**——专属（课表自带）、公共（可复用、有名）、组合（按日期区间自动切换）｜`TimeTable.kt:6-19`、`TimeScheduleRepository.kt:26-36`
- **生效作息解析**——无绑定→专属；SINGLE→目标 slots；COMBO→按日期匹配规则并"节次对齐"｜`TimeScheduleRepository.kt:44-95`
- **节次对齐 `alignTimeSlots`**——以基准作息节次骨架为准，同号节次覆盖 start/end/alias（结构不变、时间替换）｜`TimeScheduleRepository.kt:152-175`
- **组合规则匹配**——`dateStr in it.startDate..it.endDate` 字符串区间，`firstOrNull` 首个命中；命中基准直接返回基准 slots｜`TimeScheduleRepository.kt:180-187,115-147`
- **专属作息保存**——事务内 upsert + `deleteTimeSlotsGreaterThan(id, slots.size)`（允许自定义节次数量）｜`TimeScheduleRepository.kt:202-210`
- **公共作息删除连带清理**——组合基准置空→删指向它的规则→解除绑定→删 slots→删 TimeTable｜`TimeScheduleRepository.kt:236-257`
- **作息管理页**——三类型卡片（标签"专属/公共/组合"）、添加/编辑/复制/多选删除（专属不可删）｜`TimeScheduleManagementViewModel.kt:35-190`
- **节次编辑推算与校验**——新增节次 = `max(endTime)+breakDuration` 起 `+classDuration` 止；`end<=start` 报错；与既有节次区间相交报错｜`TimeSlotEditBottomSheet.kt:262-346`

## 4. 课表展示（周视图 / 日视图）

- **周视图无限横向 Pager**——`HorizontalPager` 以 `MAX/2` 居中无限翻周，预载邻周，拖拽时禁用｜`WeeklyScheduleScreen.kt:85,104-127`
- **三周窗口预取缓存**——前/当前/后三周课程 + 当天生效作息 → `Map<日期, List<MergedCourseBlock>>`｜`WeeklyScheduleViewModel.kt:164-213`
- **顶部标题态机**——"未设置学期/距离开学 N 天/第 N 周/假期"；点击跳设置或弹周次选择器｜`WeeklyScheduleScreen.kt:164-177`
- **周次选择弹窗**——总周数网格，高亮当前周与选中周，点击跳转｜`WeekSelectorBottomSheet.kt:50-121`
- **课表快速切换**——右上角 swap 图标 → 课表选择对话框（含新建）｜`WeeklyScheduleScreen.kt:236-242`
- **星期/周首日重排**——按 `firstDayOfWeek` 旋转表头；`showWeekends=false` 只渲染前 5 列｜`ScheduleGridComponents.kt:479-489`、`ScheduleGrid.kt:57-62`
- **非本周课程降级显示**——~62% 遮罩 + 斜纹线，不可拖动｜`CourseBlock.kt:182-199`
- **今日高亮**——当日列与当前节次行加 `primaryContainer 40%` 背景；时间列可点跳作息管理｜`ScheduleGridComponents.kt:232,313`
- **课程块内容渲染**——24h 模式全显起止时间；节次模式仅自定义时间课显示时间，普通课按开关显示开始时间；可隐藏老师/地点、去 `@` 前缀、字号/字距缩放、水平/垂直居中｜`CourseBlock.kt:66-90,165-178`
- **边框三态**——无/实线/虚线（`dashPathEffect(20,10)`）｜`CourseBlock.kt:98-116`
- **课程详情底部弹窗**——课名/老师/地点/周次文案/星期节次/备注；重叠课程横向翻页（胶囊"虫形"分页指示器）｜`CourseDetailBottomSheet.kt:95-240,393-402`
- **日视图（今日课表）**——纵向时间轴；自动滚到"第一门未结束"；已结束划线+50% 透明；空态"今日无课"｜`TodayScheduleScreen.kt:122-343`
- **今日页状态机**——Normal/NoSemesterConfig/SemesterEnded/Vacation；跳过日期返回空列表｜`TodayScheduleViewModel.kt:68-96`
- **背景壁纸**——课表整屏 AsyncImage + 透明 Scaffold｜`WeeklyScheduleScreen.kt:190-197`
- **底部导航随滚动隐藏**——`collapsedFraction` 上报外层｜`WeeklyScheduleScreen.kt:179-187`

## 5. 课程编辑

- **"一个课程 = 多条 CourseScheme"**——课程名 + 多方案（各设老师/地点/备注/颜色/星期/节次或自定义时间/周次），每条方案落一条 Course｜`AddEditCourseViewModel.kt:33-47,274-306`
- **点空白格新建**——自动带该格星期/节次与当前周（24h 模式带整点时段）；学期外提示｜`WeeklyScheduleScreen.kt:327-385`
- **按名聚合编辑**——载入同名全部记录为多方案；保存时删除被移除的方案｜`AddEditCourseViewModel.kt:103-118,280-285`
- **新增方案继承上一条**——老师/地点/备注/颜色沿用，周次默认全选｜`AddEditCourseViewModel.kt:206-218`
- **周次选择器**——网格多选 + 全选/单周(奇)/双周(偶) FilterChip｜`CourseOtherSelectors.kt:145-270`
- **节次三联滚轮 + 自定义时间四滚轮**——开始>结束拦截｜`CourseTimeDialogs.kt:42-272`
- **颜色选择 + "应用到全部方案"**——6 列色板（浅/深）｜`CourseOtherSelectors.kt:272-383`
- **保存校验 / 未保存退出拦截 / 编辑页删整门课**｜`AddEditCourseScreen.kt:96-171`、`AddEditCourseViewModel.kt:308-316`

## 6. 手势交互

- **点击块→详情；点空白→新建（或浮动课落位）**｜`ScheduleGrid.kt:202-209,375-386`
- **长按进入编辑态**——放大阴影 + 上下圆形拉伸手柄（12dp 蓝点、热区 32dp）、网格禁滚、点空白退出｜`ScheduleGrid.kt:204-215`
- **拖动主体**（改星期+节次）/ **拖手柄拉伸**（改时长，minGap 节次 1 节、24h 0.25h）｜`ScheduleGrid.kt:224-356`
- **拖到左右边缘→跨周"浮动搬运"**——阈值 -6.18dp，底部浮条提示，目标周任意格点一下落位并保持原时长｜`ScheduleGrid.kt:241-249`、`FloatingCourseBar.kt:37-99`
- **编辑态自动边缘滚动**——距边 40dp 内 8dp/帧｜`ScheduleGrid.kt:118-169`
- **24h 模式拖拽实时显示分钟刻度**｜`ScheduleGrid.kt:78-116`
- **落库"单周直改 / 多周克隆拆分"**——仅 1 周直接改该 Course；多周课克隆新 UUID 只含目标周，原课剔除该周｜`WeeklyScheduleViewModel.kt:548-627`
- **位置未变短路**、落库后重置编辑态｜`WeeklyScheduleViewModel.kt:596-604`、`ScheduleGrid.kt:50-53`

## 7. 导入导出 / 备份恢复 / 日历同步

- **单表 JSON 导入/导出**——`CourseTableExportModel{courses[], timeSlots[], config{}}`；`ignoreUnknownKeys + encodeDefaults + coerceInputValues`；导入**始终清空**该表课程，timeSlots/config 非空才覆盖｜`CourseImportExport.kt:22-127`、`CourseConversionRepository.kt:200-314`
- **导入校验**——节次编号从 1 连续、HH:MM 正则、start<end、不重叠；自定义时间课必须有合法起止｜`CourseConversionRepository.kt:43-101`
- **导入颜色分配**——按课程名去重同色；JSON 带合法 color 用之，否则随机起点循环取色；备注截 300｜`CourseConversionRepository.kt:103-155`
- **导入后作息绑定重置专属**｜`CourseConversionRepository.kt:185-190`
- **ICS 导出（先选课表再选提醒分钟）**｜`CourseTableConversionDialogs.kt:38-105`、`CourseConversionRepository.kt:427-451`
- **同步系统日历（Android）**——本地日历账户（`ACCOUNT_TYPE_LOCAL`、名=App 名、色 `#4285F4`），先删后批量插 Events+Reminders；jvm 返回 false 未实现｜`CalendarAccountManager.android.kt.kt:33-149`、`CalendarAccountManager.jvm.kt:8-18`
- **全量多课表备份（CBOR 信封）**——`TotalAppBackupEnvelope{backupTimestamp, appVersionCode(=COURSE_SCHEMA_VERSION), currentCourseTableId, allTables[]}`｜`CourseImportExport.kt:15,39-56`、`BackupRepository.kt:147-170`
- **模块化备份包（ZIP）**——`meta.json + course.cbor + style.cbor`；模块 COURSE/STYLE｜`BackupRepository.kt:28-109`
- **恢复版本校验**——schemaVersion > 本地报错；恢复课表先清空；恢复后校准 currentCourseTableId｜`BackupRepository.kt:114-217`
- **样式独立备份**——导出剥离壁纸路径，恢复保留本地壁纸｜`StyleSettingsRepository.kt:50-114`
- **WebDAV 云备份/恢复**——Basic Auth、MKCOL 逐级建目录、PUT/GET、固定子目录 `Backup/`｜`WebDavClient.kt:20-135`、`WebDavConfig.kt:10-24`
- **WebDAV 密码加密存储**——AES/GCM；Android 用 AndroidKeyStore，JVM 用 PKCS12 密钥库（固定口令，弱于 KeyStore）｜`ApiConfigRepository.kt:19-107`、`SecureCrypto.android.kt:12-68`、`SecureCrypto.jvm.kt:12-94`
- **导出后"分享"引导**——Android FileProvider+ACTION_SEND；iOS UIActivityViewController；桌面关闭｜`ShareManager.kt:19-56` 及各平台实现
- **统一文件选择抽象 `FileManager`**——pickImage/importFile/exportFile｜`tool/FileManager.kt:10-44`

## 8. 教务系统适配（WebView + JS Bridge）——上游为"适配任意学校"设计

- **离线适配资源内置 + 首启解压**——`offline_schools.zip`（`school_index.pb` + 每校 JS 脚本）｜`shared/build.gradle.kts:166-176`、`ResourceInitializerManager.kt:26-122`
- **学校索引 proto**——`SchoolIndex{protocol_version, version_id, schools[]}`；`Adapter{adapter_id, category(GENERAL_TOOL/BACHELOR_AND_ASSOCIATE/POSTGRADUATE), asset_js_path, import_url?}`｜`school_index.proto:12-75`
- **学校选择页**——分类 Tab + 搜索 + 拼音索引条 + 每分类记忆最近访问｜`SchoolSelectionViewModel.kt:27-101`、`AlphabetIndexerList.kt:26-80`
- **适配仓库热更新**——kgit 浅拉 `resources/` 与 `index-pb-release` 分支；协议版本协商（>2 致命）；支持自定义/私有仓库 token｜`GitUpdater.kt:52-331`、`git_repos.json`
- **JS Bridge 协议（Promise 化）**——`window.shiguangBridgePromise`: showAlert/showPrompt(带 JS 校验)/showSingleSelection/saveImportedCourses/saveCourseConfig/savePresetTimeSlots；`notifyTaskCompletion` 收尾跳回｜`WebBridgeProtocol.kt:29-223`、`WebBridgeHandler.kt:36-322`
- **WebView 桌面模式（反爬）**——桌面 UA、TEXT_AUTOSIZING、viewport 修复注入、删 `X-Requested-With`｜`WebCompatDelegate.kt:25-132`
- **XHR/Fetch/Form POST 全拦截**——body 注册到 `WebPostService`，注入 `X-WebView-Post-Id`｜`AndroidWebConstants.kt:6-188`
- **Native 接管请求**——Ktor CIO 重放（主框架禁跟随重定向、Cookie 双向同步、剔 Content-Encoding、解析 charset）｜`WebViewRequestInterceptor.kt:31-227`
- **WebView 页 UI**——地址栏（开发者模式才显示）/前进/刷新/手机-桌面切换/DevTools 开关/执行导入脚本｜`WebViewScreen.kt:93-353`

## 9. 通知与提醒 / 上课自动化（Android）

- **课前提醒**——`reminderEnabled` + `remindBeforeMinutes`（默认 15，弹窗校验范围）｜`AppSettingsModel.kt:79-83`、`NotificationSettingsViewModel.kt:142-148`
- **精确闹钟槽位机制**——占用 50010–50110 共 101 槽；先全量 cancel 再为未来 7 天未跳过课程按时间排序分配；`setExactAndAllowWhileIdle`｜`CourseNotificationWorker.kt:39-144`
- **通知呈现**——IMPORTANCE_HIGH 渠道、BigTextStyle(地点+老师)、CATEGORY_EVENT、关闭动作按钮；穿戴兼容切 `setOngoing(!compat)`/`setAutoCancel(compat)`；Android 16+ `setRequestPromotedOngoing` + 实时状态文本｜`CourseAlarmReceiver.kt:134-214`
- **上课自动化（勿扰/静音）**——DND 用 `setInterruptionFilter`，静音用 `ringerMode`；闹钟 ID 50001(开)/50002(关)，只调度"下一个开始/结束"两个时点，执行后自我重排｜`CourseAlarmReceiver.kt:56-73,102-109`、`DndSchedulerWorker.kt:68-196`
- **跳过节假日的贯穿**——被跳过日期：不排提醒、今日页空、小组件 `isSkipped`、ICS/日历跳过｜`AdvancedSettingsCard.kt:30-104`、`TodayScheduleViewModel.kt:67-96`、`WidgetDataSynchronizer.kt:248`
- **权限处理**——通知/精确闹钟/DND/后台自启/忽略电池优化，各有引导与 `ActivityNotFoundException` 回退｜`NotificationSettingsScreen.android.kt:31-98`、`NotificationSettingUtils.kt:16-116`
- **设置变更自动重排**——监听设置变化→重新入队 Worker + 刷新小组件｜`SyncManager.kt:32-61`

## 10. 桌面小组件（Android）

- **4 种规格**——超小 2x1(Tiny)/紧凑 2x2(Compact)/近日 4x2(DoubleDays)/垂直列表 4xN(ListVertical)，各自 `appwidget-provider`（`updatePeriodMillis=1800000`）｜`strings.xml:31-34`、`res/xml/*_widget.xml`
- **数据预处理**——`WidgetDataSynchronizer` 监听变化（500ms 防抖）预计算未来 7 天写独立库；无配置/学期结束清空｜`WidgetDataSynchronizer.kt:56-271`
- **快照 proto**——`WidgetSnapshot{courses[], current_week, style}`｜`widget_snapshot.proto:11-28`
- **统一分发 + RemoteViews 渲染**——`updateAllWidgets` 依次调 4 个 Renderer（间隔 300ms）；Compact 状态机（假期遮罩/今日剩余/明日预告/已结束）；Tiny 只显最近一节并按 color 上色；DoubleDays 左右两列今天/明天｜`WidgetUpdateHelper.kt:39-120`、各 Renderer
- **刷新调度**——UI 每 15 分钟 + 每天全量同步，全部禁用则取消｜`WorkManagerHelper.kt:14-43`
- **深浅色适配**——`layout-night/`、`drawable-night/` 备用资源

## 11. 设置与主题

- **课表参数**——显示非本周课程、显示周末、开学日期选择器、总周数(1..30 滚轮)、当前周手动设置（反推开学日，含"假期"项）、每周起始日｜`SettingsScreen.kt:168-358`
- **样式项全集**——24h 模式开关、隐藏左侧时间/星期下日期/网格线、页面文字颜色；节次高度 40-120dp、时间列宽 20-80dp、表头高 30-80dp；课程文字颜色/显示开始时间/隐藏地点/隐藏老师/去 `@`/水平垂直居中/字号 0.5-2.0/圆角 0-24dp/内外边距/不透明度 0.1-1.0/边框三态｜`StyleSettingsComponents.kt:180-272`
- **颜色池自定义**——浅/深各 12 色逐个改（HSV 取色 + RGB 输入 + Hex）；重置样式（留壁纸）/彻底重置｜`StyleSettingsComponents.kt:240-330,619-696`、`AdvancedColorPicker.kt`
- **壁纸**——选图→按屏裁剪→JPEG(90) 私有目录；更换删旧文件；备份剥离路径｜`StyleSettingsViewModel.kt:93-388`、`ImageCropper.*`
- **主题**——跟随系统/浅/深；动态取色 Material You（Android 12+）；自定义主色浅/深各一｜`ThemeSettingsScreen.kt:121-291`、`Theme.kt:24-120`
- **语言**——跟随系统 + 列表（zh/zh-rCN/zh-rTW/en）；Android `setApplicationLocales`，JVM 写 properties｜`LanguageSettingScreen.kt:36-127`
- **启动页**——课表 / 今日课表｜`MoreOptionsDialogs.kt:52-89`
- **更多页**——图标/版本/开发者模式（连点 5 次解锁）/检查更新/语言/主题/启动页/仓库/开源许可/更新适配仓库/贡献者/鸣谢｜`MoreOptionsScreen.kt:129-241`

## 12. 多课表 / 多作息管理

- **课表管理页**——新增/重命名/删除（禁删最后一个，删除当前自动切换）/卡片显示当前标记；新建课表事务化建专属作息+默认配置｜`ManageCourseTablesScreen.kt:96-311`、`CourseTableRepository.kt:107-129`
- **当前课表兜底**——DataStore 空则回落最早创建的课表｜`AppSettingsRepository.kt:66-69`
- **作息绑定/解绑**——可绑公共/组合作息或回退专属；导入课程/JSON 强制重置专属｜`TimeScheduleManagementViewModel.kt:152-166`

## 13. 快捷操作：调课 / 快速删除

- **调课（Tweak）页**——选课表、源日期、目标日期，预览两天课程；三模式 `MERGE`(合并移动) / `OVERWRITE`(先删目标日该周关联再移动) / `EXCHANGE`(切断双方该周关联交叉移动)；同一天/未设学期报错｜`TweakScheduleViewModel.kt:45-232`、`CourseTableRepository.kt:218-270`
- **内部移动算法**——单周课直接改 `day` 并搬迁周次；多周课克隆新 UUID 只含目标周｜`CourseTableRepository.kt:275-316`
- **快速删除页**——双维度筛选（周次×星期多选 / 日期范围自动换算）；实时预览受影响课程与数量；只删 `course_weeks` 关联（保留课程基础信息）｜`QuickDeleteViewModel.kt:43-247`、`CourseTableRepository.kt:323-339`

## 14. 全局课程管理

- **课程名列表（Master）**——按当前课表聚合唯一课程名+实例数，字母序｜`CourseNameListViewModel.kt:46-83`
- **按名批量删除**——多选课程名一次删全部同名记录（级联删周次）｜`CourseNameListViewModel.kt:77-83`
- **课程实例列表（Detail）**——同名全部实例（星期/节次/周次范围），多选批量删除｜`CourseInstanceListViewModel.kt:46-127`

## 15. 其他 App 外壳

- **导航骨架**——30 个类型安全 Destination、主界面清栈、自定义转场 300ms｜`Navigation.kt:22-131`
- **应用更新检查**——GitHub/Gitee 双渠道 `releases/latest`、语义化版本比较、按 ABI 匹配 APK｜`UpdateTool.kt:16-151`
- **开源许可列表**——aboutLibraries 构建期导出｜`shared/build.gradle.kts:143-153`
- **贡献者页**——读 contributors.json｜`ContributionScreen.kt:65-262`
- **Toast 与无障碍**——平台分发；540 条 string 资源 + a11y 语义｜`ToastManager.kt:3-13`
- **滚轮选择器/日期选择组件**｜`NumberPicker.kt:42-60`、`DatePickerModal.kt:25-45`

## 16. 核心算法细节补充（对账最关键的几条）

### 16.1 重叠课程分列（渲染引擎，本仓 grid.rs 已 1:1 移植）
`WeeklyScheduleViewModel.kt:632-754`：时间归一化（解析失败**丢弃不渲染**）→ 坐标换算（两模式）→ `minSafeHeight` 兜底（节次模式 0.3 格）→ 边界夹取（`start ∈ [1.0, limit-0.1]`，`end ∈ [1.1, limit]`）→ 按天分组 → 聚类（容差 0.01，团簇传递合并）→ 簇内贪心列分配（`columnEnds[i] <= start+0.01`）→ 输出 `MergedCourseBlock{startSection/endSection 为 0 基浮点、isVisualDemoted=非本周、nonActiveRanges=(列索引,总列数)、clusterCourses=整簇}`。

### 16.2 手势落库语义（拖拽改课的核心规则）
`WeeklyScheduleViewModel.kt:438-627`：24h 反算 custom 时间（跨午夜截 23:59）；位置无变化短路；**`weeks.size <= 1` 原地更新，多周克隆拆分**；finally 清空 floating 状态。

### 16.3 颜色分配与自愈
网格新建随机取色；导入按课程名去重同色+随机起点循环；渲染前扫描越界 `colorInt` 写回合法随机值，仍越界回落首色｜`WeeklyScheduleViewModel.kt:309-318`、`CourseBlock.kt:51-57`、`CourseConversionRepository.kt:103-155`。

### 16.4 提醒/小组件的时点与排序
提醒集合=小组件库今天..+7 天、`!isSkipped`、按 startTime 排序、最多 101 条、`remindTime = 开始 - 提前分钟`，仅未来才设｜`CourseNotificationWorker.kt:53-80`。小组件条目 id=`courseId-dateString`；同步起点=`max(today, alignedSemesterStart)` 连续 7 天｜`WidgetDataSynchronizer.kt:199-251`。

### 16.5 导出前置条件
ICS 导出必须有 `semesterStartDate` 且 `totalWeeks > 0`；单表导出课程为空且无配置返回 null｜`CourseConversionRepository.kt:364-438`。

## 17. 全项目未发现实现的能力（对账时确认"未漏"）

课程/课表搜索（仅学校搜索）、撤销/重做、成绩管理、考试安排、课表图片导出/截图分享、iCloud 等其他云同步（仅 WebDAV）、iOS 小组件与 iOS 本地 ZIP 备份、桌面端托盘/多窗口/快捷键、蓝牙/穿戴原生读取（只有"兼容穿戴通知"开关）。
