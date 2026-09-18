# 计划：M2 门户核心页数据接线（今日 / 资讯 / 待办 / 应用 / 日程）

> 2026-09-18（会话 `sess-c2ed9b35`）· 分支 `feat/sess-c2ed9b35-m2-portal-pages`
> 用户授权：「现在开始实施 M2，让子智能体执行」。范围 = `PLAN.md` §M2（不含 M2.5 课表、不含 M3+）。
> 主智能体负责侦察与规划，子智能体负责实现；每批完成后主智能体验收并合并。

---

## 一、侦察事实（2026-09-18 实机，真实账号，浏览器内 XHR 捕获 + 同源重放）

### 1.1 请求通则（**重要，决定实现方式**）

- 全部业务接口基址为门户同源相对路径 `/api/...`。
- 两种响应信封，**不要混用解析器**：
  - 门户自身服务：`{"meta":{"success":true,"statusCode":200,"message":"ok"},"data":...}`
  - 日程服务 `bs-schedule/*`：`{"code":"0","msg":"ok","data":...}`（成功判据是 `code=="0"`）
- **鉴权**：实测门户自身读接口（`upp/*`、`uppinfo/*`、`uppflow/*`、`uppcard/*`、`uppexcard/*`、`contentDisplay/*`）**仅凭会话 Cookie 即可**；但 `bs-schedule/*` 的 `getCountBetweenTime` / `findScheduleBetweenTime` 在裸请求下返回 `500 系统错误`，必须带完整头组。**统一做法：所有业务请求都带同一组头**（与门户前端一致，避免逐个接口试错）：

  | 头 | 值来源 |
  |---|---|
  | `Authorization` | `POST /tryLoginUserInfo` 的 `data.tokenId`（JWT，**无 `Bearer` 前缀**） |
  | `loginUserId` / `loginUserName` | 同上 `data.userId` |
  | `loginUserOrgId` | 同上 `data.orgId`（学生实测 `-1`） |
  | `appid` | 固定 `ly-upp` |
  | `csrfTimestamp` / `csrfToken` | 现算毫秒时间戳 + `csrf_token(ts)`（**复用 `crates/campus-auth/src/cas.rs` 已有实现，勿重写**） |
  | `X-Requested-With` | `XMLHttpRequest` |
  | `Accept` | `application/json, text/plain, */*` |
  | `Content-Type` | 仅 POST 需要：`application/json` |

- ⚠️ **敏感字段（红线）**：`Authorization` JWT 与 `queryAppointCard` 响应里的 `loginUrl`（邮箱免密登录链接，**内含 authkey，等同凭据**）——一律只在内存中使用，**绝不落盘、绝不写日志、绝不进文档/派单 prompt、绝不返回给前端**。解析结构体里对 `loginUrl` 直接丢弃（不要定义该字段）。

### 1.2 接口清单（全部实测，含精确查询参数）

| 用途 | 方法 | 端点与参数 | 响应要点 |
|---|---|---|---|
| 学期与当前周 | GET | `api/upp/config/querySemesterInfo` | `data:{grade:"2026",semester:"1",currentWeek:"2",weekCount:"19",startDate:"20260907",endDate:"20270117",currentDate,currentWeekDay:"星期五"}`（与 `campus-schedule` 的开学日一致） |
| 周课表 | GET | `api/uppcard/kbsz/queryAWeekSchedule?cardId=53dfb6845c3048e8bb0325672444a51a&results=` | `data:{resultsJsonArr: 7×10 字符串矩阵, zs(周次), xn, xq, xqj, zsms, swskjc, xwskjc, wsskjc, weekcount, dataSource}`；每格形如 `"课程名,教室,教学班,姓名"`，空串=无课，**每天占两列** |
| 全部学期 | GET | `api/uppcard/kbsz/queryAllTerm?cardId=53dfb6845c3048e8bb0325672444a51a` | `data:[{"XNXQ":"2026-2027-1"},…]` |
| **钱包三卡** | GET | `api/upp/contentDisplay/queryAppointCard/bd80fdbde8ed4f9abf9cef936bd907a8` | `data.data` 是**JSON 字符串**，`JSON.parse` 后为 `[{ZJ:"暂无",ZHYE:102.51,JYE:0.45,YE:102.51,SSYE:102.51,SL:8,mailNewCount:"1",mailFolderNewCount:"0",totalCount:"1",pageTotalCount:1,loginUrl:"⚠️丢弃"}]`；`YE/ZHYE/SSYE`=一卡通余额、`SL`=在借图书、`mailNewCount`=未读邮件 |
| 资讯栏目列表 | GET | `api/uppinfo/userSetting/queryUserSubscribeColumn` | `data:[{columnId,titleLocale:"{\"zh_CN\":\"通知公告\"}",sortNum,columnType,isRemind}]`（只含订阅的 3 个） |
| 资讯列表 | GET | `api/uppinfo/infoCenter/querySimpleInfoCenter?pageNum=1&pageSize=10&columnIds=<id>&columnType=&showNewDate=0` | `data:{pageNum,pageSize,total,pageCount,list}`；⚠️ **`total` 实测不可靠（pageSize=1 时返回 0）**，分页请以 `list.length` 与 `pageCount` 为准 |
| 资讯条目字段 | — | 同上 `list[]` | `infoId, infoTitle, extLink(官网正文 URL), publishTime("2026-09-18 03:31:43"), columnId, columnTitle, publishDeptName, publisher, hitCount, isReaded, detailType("link"), top, urgencyCode, labelList[], attachmentFileList[]` |
| 待办分栏 | GET | `api/uppflow/affairCenter/queryTabItems?isCount=1` | `data:[{tabId:"todo"\|"done"\|"apply"\|"unread"\|"read", tabName, tabDesc, count, selected:[{fieldId,fieldCode,fieldName}]}]` |
| 待办列表 | GET | `api/uppflow/process/queryFlowItems?groupType=time&isUrge=&isRead=&isDone=&applyStartime=&applyEndtime=&arriveStartime=&arriveEndtime=&pageSize=10&pageNum=1&tabId=todo&sorter=DESC` | `data:{pageNum,pageSize,pageCount,total,data:[]}`（当前账号三栏均 0 条，返回空数组） |
| 待办筛选项 | GET | `api/uppflow/affairCenter/querySeletedItem?tabId=todo&lstItem=processId,appId,urgency` | `data:{processId:[],appId:[],urgency:[{paramId,paramName:"一般/急件/平急/加急/特急/特提"}]}` |
| 应用（全部） | GET | `api/upp/appStore/queryApp?pageNum=1&pageSize=-1&excludeMobile=1` | `data: array[30]`，字段含 `appId,appName,appIcon(UUID),appLink,isCas,showType,isPopUp,orderId,isRecommend` |
| 应用（分组） | GET | `api/upp/appStore/v2/queryApp?pageNum=1&pageSize=999&appLabel=&appType=&appName=&isStore=&appSort=0&pageId=` | `data: array[8]` |
| 应用分类/部门 | GET | `api/upp/appStore/v2/queryClassifyList`、`api/upp/appStore/v2/queryDepLabel`、`api/upp/appStore/queryMyStore` | 依次 `{classifyUsing,lstType,lstLabel}`、`array[8]`、`array[2]` |
| 日程分类 | POST | `api/bs-schedule/innerPlaintext/scheduleRpcManage/findScheduleClassifyList`，body `{"pageSize":0,"pageNum":1}` | `data:[{classifyName:"个人日程",classifyCode:"Default-person",classifyColor:"#ff9ee1"},…]` 共 5 类：`Default-person`/`Default-Activity`/`Default-Meeting`/`Default-duty`/`Default-class` |
| 日程区间 | POST | `api/bs-schedule/innerPlaintext/scheduleRpcManage/findScheduleBetweenTime`，body `{"startTime":<ms>,"endTime":<ms>,"scheduleClassifyCodeList":[...],"scheduleName":null,"publishStatus":1,"collaborativeId":"","collaborativeType":""}` | `data:[{id,...,num,tpApp,...}]` 日程明细 |
| 每日计数 | POST | `api/bs-schedule/innerPlaintext/scheduleRpcManage/getCountBetweenTime`，body `{"startTime":<ms>,"endTime":<ms>}` | `data:[{day:"2026-09-01",count:0},…]` |
| 冲突检测 | POST | `api/bs-schedule/innerPlaintext/scheduleRpcManage/conflictInfo`，body 同上 | `data:[]` |
| 会议卡数据 | GET | `api/uppexcard/ext/dynamicData/10.1.90.34/ZCHY?DJZ=<周次会议名>&pageNum=1&pageSize=20` | `data:array[6]`（门户「第二周会议」卡的取数方式，`10.1.90.34` 是门户代理的内部主机） |
| 布局配置 | GET | `api/upp/layout/getPageContent?pageId=<每页不同>` | 卡片布局树（18KB 级）；**客户端不需要**，仅作对照 |
| 快捷入口 | GET | `api/uppcard/serviceTypeShow/selectAppByCardId?cardId=4296fa2582fa4ef1bb68b8793547e0be` | `data.appList[]`（含 `appLink`/`isCas`），首页快捷动作可由此驱动 |

资讯栏目 id ↔ 名称（实测 7 个，正文一律在官网静态页）：

| columnId | 名称 | 正文域名与 URL 形式 |
|---|---|---|
| `9` | 通知公告 | `www.cwxu.edu.cn/content.jsp?urltype=news.NewsContentUrl&wbtreeid=1039&wbnewsid=<id>` |
| `f382fddd843b4058a486a9375ecf422d` | 校园要闻 | `www.cwxu.edu.cn/info/1033/<id>.htm` |
| `a8bc1e5a9225475b9841b5a237c690df` | 校园快讯 | `www.cwxu.edu.cn/info/1035/<id>.htm` |
| `ea0a5b2158bf48b3afeb026477c626e4` | 教务处 | `jwc.cwxu.edu.cn/info/1100/<id>.htm` |
| `4f5a7ccbc5704a6690f0d3ac429c2201` | 学工处 | `xgc.cwxu.edu.cn/info/1129/<id>.htm` |
| `5d2c45d23866497cb2bfe93e9f136bb2` | 规章制度 | `xgc.cwxu.edu.cn/content.jsp?...wbtreeid=1105&wbnewsid=<id>` |
| `d4901da2e5df4db9b6b551df4d5b85dd` | 团委 | `tw.cwxu.edu.cn/info/1164/<id>.htm` |

**正文获取方式**：`list[].detailType == "link"`，正文在官网静态页（`extLink`）。客户端抓 `extLink` → 提取正文容器 → 清洗后本地渲染（官方是跳转静态页，这是我们对官方的改进点）。

### 1.3 校本作息与课表矩阵语义（2026-09-18 实测，两个独立来源交叉验证）

**真实作息（一个「大节」= 100 分钟）**，来源是学校自身日程服务 `bs-schedule` 的 `Default-class` 事件（实测 15 条、跨 4 周）：

| 事件 | 实测时间 |
|---|---|
| 信息隐藏与取证技术（周一） | **10:10 → 11:50** |
| 马克思主义基本原理 / 信息安全 / 物联网安全与隐私保护 | **13:45 → 15:25** |

即：下午第一大节 **13:45** 开始，而不是 `campus-schedule::default_time_slots()` 上游默认值里的 14:00（该默认表是 shiguangschedule 原值，13 节、午休 12:15-14:00，**与本校不符**）。

**门户课表矩阵语义**：`queryAWeekSchedule.resultsJsonArr` 是 **7 行 × 10 列**，且

- **行 = 星期**（行 0 = 周一）；
- **10 列 = 5 个大节 × 2 小节**，一门课占据相邻两列（即一个大节），空串 = 无课。

列→大节：(1,2)=大节1、(3,4)=大节2、(5,6)=大节3、(7,8)=大节4、(9,10)=大节5。**验证方式**：用上表 4 门有真实时间的课程逐条比对矩阵位置，**四条全部吻合**（信息隐藏在列 3,4 → 大节2 → 10:10；马克思在列 5,6 → 大节3 → 13:45；信息安全周二列 5,6；物联网周四列 5,6）。

**校本大节表**（拟在 `campus-portal` 内定义；不必改 `campus-schedule` 的上游默认表）：

| 大节 | 时间 | 依据 |
|---|---|---|
| 1 | 08:00-09:40 | 由「大节2 10:10 开始 + 30 分钟课间」反推（**未实测**） |
| 2 | 10:10-11:50 | **实测** |
| 3 | 13:45-15:25 | **实测** |
| 4 | 15:35-17:15 | 由大节3 结束 + 10 分钟课间推出 |
| 5 | 18:30-20:10 | 晚上档位（**未实测**） |

⚠️ **两个坑**：① 早期真机把大节4 的起点显示成 14:50（错按默认 13 节表的第 7 节），已修；② 日程服务的课表数据**不完整**（实测缺「算法设计与分析」这门课），**不能当唯一事实源**，课表以门户矩阵为准、日程服务只用于锚定作息。

### 1.4 关于钱包「实时」的说明（PLAN.md §M2 第 6 条）

`PLAN.md` 原写「对接慧新E校实时接口（门户卡片数据有缓存偏差）」。本轮侦察结论：门户该卡片返回的**是结构化 JSON**（不是 HTML 占位符），数据来自门户服务端聚合，且**仅凭会话 Cookie 即可取**，与官方页面显示完全一致。慧新E校（`10.3.100.110`）属校内网，需额外 SSO 打通与再侦察。
**本轮决定**：批次 1 先用门户接口（官方同源、零新增鉴权、满足 M2 验收「与官方一致」）；「慧新E校实时直连」列为**批次 4（可选增强）**，是否推进在主智能体汇报后由用户定。

---

## 二、冻结契约（子智能体不得擅自改动；如需变更先在计划里写明并向主智能体说明）

### 2.1 IPC 命令（一律 `CommandResult { success, message?, data? }`，camelCase）

统一约定：所有命令未登录时返回 `success:false, message:"请先登录"`；网络/解析失败返回可读中文 `message`，**不 panic、不吞错**。

**批次 1**
```
get_portal_overview() -> PortalOverview
  PortalOverview {
    semester: SemesterInfo | null      // 取失败不阻塞其余字段
    wallet:   WalletSummary | null
    nextCourse: CourseBrief | null     // 无课为 null（前端隐藏该卡）
    fetchedAt: number                  // epoch ms
  }
  SemesterInfo  { grade, semester, currentWeek, weekCount, startDate, endDate, currentWeekDay }
  WalletSummary { cardBalance: number|null, bookBorrowed: number|null, mailUnread: number|null }
  CourseBrief   { name, room, teachingClass }
```

**批次 2**
```
get_info_columns() -> InfoColumn[]                     // InfoColumn { id, name, sortNum }
get_info_list(columnId: string, page: number, pageSize: number) -> InfoPage
  InfoPage { page, pageSize, pageCount, total, items: InfoItem[] }
  InfoItem { id, title, columnTitle, publishTime, dept: string|null, url }
get_info_detail(url: string) -> InfoDetail             // InfoDetail { title, html }
  // url 白名单：仅允许 *.cwxu.edu.cn 域名，其他一律拒绝（防 SSRF/钓鱼）
get_todo_tabs() -> TodoTab[]                           // { id, name, desc, count }
get_todo_list(tabId: string, page: number, pageSize: number) -> TodoPage
  // TodoPage { page, pageSize, pageCount, total, items: TodoItem[] }
  // TodoItem { id, title, applicant, applyTime, source, node, urgency }
```

**批次 3**
```
get_app_catalog() -> AppCatalog       // { groups: AppGroup[] }
  AppGroup { id, name, apps: AppItem[] }
  AppItem  { id, name, iconUrl, link, isCas: boolean, showType }
get_schedule_classify() -> ScheduleClassify[]          // { name, code, color }
get_schedule_month(startMs: number, endMs: number, codes: string[]) -> ScheduleEvent[]
  ScheduleEvent { id, title, startMs, endMs, place, classifyCode, classifyName, color }
open_app(url: string, isCas: boolean) -> ()           // 打开策略在执行器内决定
```

### 2.2 后端结构

- 新建 crate **`crates/campus-portal`**（与 `campus-auth` 平级、同样**不依赖 tauri**），职责：门户业务接口调用 + 响应解析。协议层放这里，IPC 只做薄封装。
- **会话复用**：不得复制一份会话/登录管理。先读 `crates/campus-auth/src/cas.rs` 与 `tauri-app/src-tauri/src/commands/auth.rs`，用现有的已登录 `reqwest::Client`（含 Cookie jar）构造；若现有暴露不足，**在 `campus-auth` 上加最小 getter**（例：把已有的 `session_client` 提升可见性），不要在 `campus-portal` 里重建。
- **统一请求头**：一个内部 helper 负责注入 1.1 表格里的全部头；JWT 与 profile **按会话内存缓存**（避免每个接口都多打一次 `tryLoginUserInfo`），会话失效/退出时清空。**缓存不落盘。**
- **解析全部写成纯函数**（`parse_*(&str) -> Result<T>`）并用**脱敏 fixture** 单测；真实姓名/学号/邮箱/authkey/JWT 一律不进测试样本（用 `2023001`、`张三`、`示例学院` 之类占位）。
- 每个接口的超时与错误映射遵循 `campus-auth` 现有风格（自定义 error enum + 中文 message）。

### 2.3 前端

- 面板现有实现：`tauri-app/frontend/src/panels/{Today,Info,Todo,Apps,Schedule}.tsx`，已含占位注释（如 `InfoPanel.tsx:11` 标注「M2 接 querySimpleInfoCenter」）。本轮把占位换成真实数据。
- 取数统一走 `src/shared/tauriApi.ts::invokeCommand`；状态用组件内 `useState`（或按现有模式）表达**四态：加载中 / 有数据 / 空 / 出错**；错误态必须能重试。
- **不伪造数据**：取不到就显示空态或错误态，不用假数字占位。
- 视觉沿用现有设计系统（`PanelHeader`/`EmptyState`/`Surface`、域色 token）；不新引入 UI 依赖。
- 资讯正文渲染：只吃后端清洗过的 HTML 片段，前端不再做二次清洗；图片/链接需转绝对地址由后端完成。

---

## 三、分批任务（每批一个子智能体独立完成，主智能体验收）

### 批次 1 · 基建 + 今日页
1. 新建 `crates/campus-portal`（加入 workspace `Cargo.toml`），实现：会话/头 helper、`querySemesterInfo`、钱包卡、`queryAWeekSchedule`（只要「下一节课」所需的最小解析）三个接口 + 解析纯函数 + 单测。
2. `tauri-app/src-tauri`：新增命令 `get_portal_overview` 并在 `lib.rs` 注册（命令数 13 → 14）。
3. 前端 `TodayPanel.tsx`：问候区保留；钱包三卡接真实数字（一卡通余额 / 未读邮件 / 在借图书）；「下一节课」横幅（无课隐藏）；首次加载骨架、失败可重试。
4. 验证：`cargo test --workspace` 全绿；`tsc --noEmit` 0 错误；`vite build` 通过；真机 `tauri dev` 看到真实数字（截图）。

### 批次 2 · 资讯页 + 待办页
1. `campus-portal`：栏目列表、资讯列表、资讯正文抓取与清洗、待办分栏、待办列表 + 单测。
2. 命令：`get_info_columns` / `get_info_list` / `get_info_detail` / `get_todo_tabs` / `get_todo_list`（命令数 14 → 19）。
3. 前端 `InfoPanel.tsx`（栏目 rail + 列表 + 内嵌正文，含分页与「返回列表」）与 `TodoPanel.tsx`（三栏 + 列表 + 空态）。
4. 验证同批次 1（另需真机验证一条资讯正文内嵌渲染成功）。

### 批次 3 · 应用页 + 日程页
1. `campus-portal`：应用目录（分组/分类）、日程分类与区间查询 + 单测。
2. 命令：`get_app_catalog` / `get_schedule_classify` / `get_schedule_month` / `open_app`（命令数 19 → 23）。
3. 前端 `AppsPanel.tsx`（分组网格 + 名称/部门副标题 + 常用钉选）+ `SchedulePanel.tsx`（周/月视图 + 5 类过滤 + 日程详情）。
4. `open_app` 打开策略：按 `isCas`/`appLink` 与设计文档附录 A 的 A/B/C 分类决定直开或 WebVPN 包装；优先复用 Tauri 既有能力，需要时用官方 `tauri-plugin-opener`（不要手写 Win32 调用）。
5. 验证同批次 1。

### 批次 4（可选，待用户拍板）· 慧新E校实时数据
需先侦察 `10.3.100.110` 的 SSO 与实时余额接口；仅在用户要求时启动。

---

## 四、铁律与验收

**红线（违反即返工）**
1. JWT、邮箱 `loginUrl`(authkey)、账号密码、DPAPI 密文 —— 不落盘、不进日志、不进文档、不进派单 prompt、不返回前端。
2. 日志中的用户名/学号一律打码（沿用现有做法）。
3. 资讯正文抓取必须限制域名白名单（`*.cwxu.edu.cn`），禁止把用户可控 URL 直接透传给 HTTP 客户端。
4. 不伪造数据；不因某接口失败让整个页面白屏。
5. 不新增前端 UI 依赖；不重写会话管理。

**每批完成的统一验收**
- `cargo test --workspace` / `cargo fmt`（**仅格式化本次改动文件**，仓库整体 fmt 是历史欠账，不要全量重排）
- `tsc --noEmit`、`vite build`
- 真机 `tauri dev` 实测该批页面，截图取证
- 文档：`CHANGELOG.md` 一条、设计文档附录（接口与最终形态）、`.codewiki/` 相应文章 + `cw index` + `cw meta update`
- 分支上按批次提交，然后 ff 合并到 `master`
