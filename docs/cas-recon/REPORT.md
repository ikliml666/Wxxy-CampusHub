# CAS 统一身份认证登录协议 · 侦察报告

> 日期：2026-09-17 · 方法：前端 JS 逆向（app.2fb1f8a1ec5d2342de95.js）+ 协议实测（probe.js）
> 结论：**协议已完整还原并实测打通**（假账号直达账号校验环节），待真实账号做最终确认。

## 一、登录全流程（账密方式）

```
1. GET  /lyuapServer/kaptcha
   ← {"kaptchaType":"1","uid":"<32位hex>","content":"data:image/png;base64,..."}
     算术题验证码图（如 3*1= / 4+3=），两个一位数，答案纯数字

2. POST /lyuapServer/v1/tickets        （Content-Type: application/x-www-form-urlencoded）
   头: token: <RSA("lyasp"+毫秒时间戳)>
   体: username=<学号>
       password=<RSA(密码明文)>          ← 见第二节
       service=https://my.cwxu.edu.cn/shiro-cas
       loginType=                        （空）
       id=<验证码uid>                    （对应前端 v.id）
       code=<验证码答案>                 （对应前端 v.code）
       otpcode=                          （空）
   ← 成功: {"meta":{...},"data":{"ticket":"ST-xxx","tgt":"TGT-xxx"}}
     失败: {"meta":{...},"data":{"code":"CODEFALSE|NOUSER|USERLOCK|TWOVERIFY|..."}}

3. GET  https://my.cwxu.edu.cn/shiro-cas?ticket=<ST>    → 302 → 门户首页，门户会话建立
```

- 无 Cookie 依赖：验证码与 uid 绑定，不依赖会话 Cookie（curl 全程无 Cookie 实测通过）。
- 前端登录前会先清旧 CASTGC cookie；服务端响应种 CASTGC（TGT），实测假账号流程未走到种 Cookie 一步。
- 错误码全集（来自前端处理逻辑）：`CODEFALSE`(验证码错) `NOUSER`(账号或密码错) `USERLOCK`(锁定) `USERDISABLED` `PASSERROR` `USERNOTONLY` `PEOPLEMOREACCOUNT`(多账号) `TWOVERIFY`(二次验证) `ISBINDOTP` `ISBINDWX` `ISMODIFYPASS`(强制改密) `NETWORKCOMMITMENT`(上网承诺书) `NOREGISTER` `NOAUTHORIZATION` `OTPERROR` `PENDINGACTIVATE`(未激活)；错误次数提示在 data（如 `"已连续错误N次,阈值M"`）。
- 二次验证/滑块等分支由服务端返回的 code 驱动；当前部署登录口为「图形验证码」模式（kaptchaType=1），滑块（AJ-Captcha 变体，端点 `/kaptcha`+`/validateKaptcha`）代码存在但未启用。

## 二、密码加密（textbook RSA，Shapiro RSA.js 系）

- 公钥：e=`0x010001`(65537)，n=`0x00b5ee...17e7b1`（**1024 位**，258 hex 字符，首字节 00）
- `setMaxDigits(131)` → `RSAKeyPair(e, '', n)` → `encryptedString(key, plaintext)`
- 组块：chunkSize = 2 × biHighIndex(n) = **126 字节**；明文按 latin1 字节流，块内 **little-endian**（b0 为最低字节）组成整数，块尾补 0x00
- 密文 c = m^65537 mod n；输出每块 **hex 小写、不补前导零**，多块空格分隔
- 密码 ≤126 字节 → 恒为单块，输出 256 hex 左右
- `token` 头 = 同法加密 `"lyasp" + Date.now()`（TAG=lyasp）
- 验证：提取原 JS 模块（rsa30.js）与独立 BigInt 实现（little-endian 组块）对 3 组明文输出完全一致

## 三、实测记录（probe.js，2026-09-17）

| 步骤 | 结果 |
|---|---|
| GET /kaptcha | kaptchaType=1，uid + base64 PNG（算术题 `3*1=`、`3*2=`、`1*2=`、`0*0=`、`0*0=`、`4+3=`） |
| POST /v1/tickets（假账号 22000000 + 正确验证码） | `{"code":"NOUSER"}` ← 验证码/加密/字段全部通过，直达账号校验 |
| POST /v1/tickets（错误验证码） | `{"code":"CODEFALSE"}` ← 对照组 |
| **POST /v1/tickets（真实账号，2026-09-17）** | **`{"tgt":"TGT-...","ticket":"ST-..."}` 登录成功**（响应顶层即 tgt/ticket，无 data 包裹、无 Set-Cookie——CASTGC 由前端 JS 写入，客户端可忽略） |
| **SSO 回跳 my.cwxu.edu.cn** | **门户会话建立**：shiro-cas 验票 302 → `http://my.cwxu.edu.cn/#/index`，种 `customsid`（Shiro 会话）/ `Authorization`（门户 API 令牌）/ `rememberMe`。302 目标是 http:// 明文，客户端应替换为 https |

## 三·二、WebVPN（深澜 Srun）CAS 联动登录（webvpn.js 实测通过，2026-09-17）

```
1. GET  https://webvpn.cwxu.edu.cn/
   ← 302 /login + Set-Cookie: wengine_vpn_ticketwebvpn_cwxu_edu_cn=<初始值>
2. CAS 登录（POST /lyuapServer/v1/tickets，service=https://webvpn.cwxu.edu.cn/login?cas_login=true）
   ← {"tgt":..., "ticket":"ST-..."}
3. GET  https://webvpn.cwxu.edu.cn/login?cas_login=true&ticket=<ST>   （带初始 wengine_vpn_ticket）
   ← 302 wengine-vpn-token-login?token=<一次性token> → 200
   ← Cookie: wengine_vpn_ticketwebvpn_cwxu_edu_cn(登录态) + show_vpn/show_fast/heartbeat/show_faq
4. 复查 GET / → 302 到门户的 WebVPN 代理页（非 /login）＝ 会话有效
```

- WebVPN 本身就是 CAS 客户端：未登录访问 /login 会 302 到「WebVPN 代理路径下的 CAS 登录页」，协议层可直接用 CAS REST 流程拿 ST 回跳，**无需走代理登录页**。
- 深澜代理 URL 样本（M4 逆向素材）：`/https/77726476706e69737468656265737421<加密hex>/...`
  - `wxcas.cwxu.edu.cn` → `e7ef429d347e6b47661dc7a99c406d3676`
  - `my.cwxu.edu.cn` → `fdee0f9f30287d1e7b0c9ce29b5b`
  - 前缀 `77726476706e69737468656265737421` 为深澜固定特征串

## 四、文件说明

- `probe.js` — 门户 SSO 探针：`node probe.js --captcha` 抓验证码 → 看图 → `node probe.js <学号> <密码> <答案>` 或 `node probe.js --creds <凭据文件> <答案>`（成功后自动回跳验证门户会话）
- `webvpn.js` — WebVPN 联动探针：`node webvpn.js --creds <凭据文件> <答案>`
- `cas.js` — CAS 协议公共库（加密/cookie jar/登录函数，探针共用）
- `rsa30.js` — 从线上 app.js 原样提取的 webpack 模块 30（Rust 实现的对照基准）
- `extract-rsa.js` — 提取脚本（含加密自测，输入 app.js 路径）
- app.js 原件不入库（602KB 第三方产物），如需重跑提取脚本：浏览器另存登录页主 JS 后传入

## 五、对 M1 实现的结论

1. Rust 协议核心无需浏览器、无 Cookie jar 依赖即可完成 CAS 登录（reqwest + num-bigint 即可）；成功响应顶层即 `{tgt, ticket}`。
2. 验证码为算术题（两操作数一位数，+/-/*），识别可纯本地：分色提取字符 → 模板匹配 → 计算，样本字体/布局高度规律。
3. 密码加密为无 padding textbook RSA，实现时按第二节参数逐条对照 rsa30.js 做 golden test。
4. 掉线检测可探测门户已登录页特征（沿用 Wxxy-CampusLogin 页面特征字符串模式）。
5. 门户 SSO 与 WebVPN 登录共用同一 CAS 登录函数，仅 `service` 参数不同（门户=`https://my.cwxu.edu.cn/shiro-cas`，WebVPN=`https://webvpn.cwxu.edu.cn/login?cas_login=true` 且需先取初始 wengine_vpn_ticket）。
6. CAS TGT 过期续签路径（`POST /v1/tickets/{CASTGC}` 签发新 ST）已在 JS 中确认存在，M1 保活可评估是否采用（客户端自持 TGT）。
