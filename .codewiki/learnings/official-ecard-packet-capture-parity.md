---
title: 官方一卡通动态抓包对照与对接修复（unlostCard 大小写 / flag 档位 / 账户 value 重复）
type: learning
source_files:
  - crates/campus-synjones/src/ecard_ops.rs
  - crates/campus-synjones/src/ecard.rs
  - tauri-app/frontend/src/components/ecard/SecureKeypad.tsx
  - tauri-app/frontend/src/components/ecard/EcardTransferView.tsx
tags:
  - synjones
  - ecard
  - packet-capture
  - official-parity
---

# 官方一卡通动态抓包对照

2026-09-20 用 browser-use 走完整官方链路（融合门户 `my.cwxu.edu.cn` → 一卡通宫格）并在页面里注入 XHR 记录器，抓到官方前端真实报文，据此修正四处对接 bug。

## 一、官方入口链（浏览器可复现）

1. CAS：`https://wxcas.cwxu.edu.cn/lyuapServer/login?service=<service>`（验证码是**算术题**图片，OCR 形态 a+b）。
2. 融合门户 service 是 `https://my.cwxu.edu.cn/shiro-cas`（不同于 lyCas 域）。
3. **直接 SSO 到 `campus-card-pc` 会报「服务大厅未授权(1)」空壳**——必须从门户首页点「一卡通」应用（`/berserker-auth/cas/redirect/lyCas?targetUrl=http://10.3.100.110/plat?name=loginTransit`）先种 plat 授权。
4. 官方所有 API 都带 `synAccessSource=h5`（官方 h5 用 h5 而非 app；服务端两者都认）。

## 二、四个真 bug（官方能用、我们对不上的原因）

1. **解挂 404**：官方 `unlostCard` **小写 l**，我们写成 `unLostCard`。该校路径大小写敏感。教训：**端点常量必须与 bundle 逐字符 diff，大小写不算「一致」**。
2. **圈存写成功但界面显示关闭**：官方 `queryCard` 卡级 `autotrans_flag` 实测为 **2**（自助及自动转账），旧解析 `== Some(1)` 读成 false。`modifyAcc` 提交其实早已成功。**档位语义：0=禁止、1=只允许自助、2=自助及自动；非 0 即开启**（accinfo 里的 `autotrans_flag` 恒 0，别读那层）。
3. **转账 select 选中换不了**：`queryCardByTransfer` 的 CARD/ACCOUNT 两账户 `account` 是**同一卡号**（42940），`<option value>` 重复导致 React 受控 select 永远选中第一项。**选项 value 必须用唯一 `code`**，提交时按 code 找回完整账户对象再取 `account`/`payacc`。
4. **限额/圈存金额类型**：官方 JSON body 金额是 **number 分**（`daycostlimit:50000`、`autotransAmt:2000`）；`autotransFlag` 是**字符串**；flag=1 不带 `autotransLimite`、flag=2 带。字符串分值服务端多半也收，但对齐官方形态最稳。

## 三、官方报文存档（retcode=0 实证）

```
payLimiteModify: {"account":"42940","acctype":"000","daycostlimit":50000,"nonpwdlimit":0,"singlelimit":0}
modifyAcc(flag=1): {"account":"42940","autotransFlag":"1","autotransAmt":2000}
modifyAcc(flag=2): {"account":"42940","autotransFlag":"2","autotransAmt":2000,"autotransLimite":2000}
getKeyboard: GET /berserker-secure/keyboard?type=Number&order=1   → numberKeyboard:"?pORVGCxj!"（10 个随机字符）
checkPwd: GET /berserker-app/ykt/tsm/checkPwd?account&pwd=1$1$...&pwdType=1（错误密码 → code=60005 账户密码错误）
modifyPwd: POST .../modifyPwd（错误密码 → code=1008 两次输入的密码不一致）
```

官方键盘组件（`security-keyboard`）**全部用 base64 图片键**：Number=10 随机字符九宫格（无刷新按钮，输错由调用方重新取键盘换批）；Standard=数字行+字母行（lower/upper 两套图、shift 切换）+符号区，`keyType` tab 切换。我们用文字键重排是合理替代，但 91 键摊平不可用 → **分区 tab**（数字/大写/小写/符号，键仍携全局下标）。

## 四、抓包方法（复用要点）

- 在页面 `evaluate` 里包一层 `XMLHttpRequest.open/send`，把 method/url/body/respText 存 `window.__reqlog`（响应只存前 1-2KB，避免凭据/卡号全量落内存）。
- **SPA 之外的整页跳转会丢 hook**——每次 `goto` 后必须重注入；plat-pc 与 campus-card 前端是不同应用，点宫格应用是整页导航。
- 受控输入（vant/React）用 `HTMLInputElement.prototype.value` setter + `input` 事件填值；官方按钮点击派发 `MouseEvent('click',{bubbles:true})`。

## 五、未实现（留待后续，接口已考察）

- **人脸采集**：独立 H5 `/overLightMobileH5`（uni-app）。表单=姓名/手机/学工号只读 + 照片上传 + 确认；接口 `POST /fapi/meeting/largeScreen/faceAcquisition/{userId}`（multipart 字段 `avatar`）与 `.../replaceFace/{userId}`，前置 `checkFaceScore` 检测人脸分数；token 是 H5 自有体系（对接需先验证鉴权链是否认 synjones token）。
- **「我的-设置」**（`/plat/user/setup`）：个人资料、安全设置（手机号+登录密码）、支付设置（脱机二维码开关 `getUserOfflienSwitch`、付款码支付顺序、校园卡支付设置=转账标识+限额【我们已有】）、设备管理、通用、关于、换账号、清缓存、退出。

## 相关

- [[learnings/ecard-write-protocol-json-body|一卡通写操作协议实测]] — form→JSON 与三套金额单位
- [[modules/ecard-panel|一卡通页]] — 命令面与验证状态
- [[modules/campus-synjones|慧新E校协议核心]] — SSO 链与信封
