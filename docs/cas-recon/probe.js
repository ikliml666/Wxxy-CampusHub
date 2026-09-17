// CAS 登录协议端到端验证探针（无依赖，node 18+ 内置 fetch）
// 用法：
//   node probe.js --captcha                       # 抓验证码图存 captcha.png，uid 存 state.json
//   node probe.js <学号> <密码> <答案>             # 提交登录（密码不落日志）
//   node probe.js --creds <凭据文件> <答案>        # 从文件读凭据（格式：账号xxx，密码yyy）
// 成功判据：响应 data.code == "NOUSER" 表示验证码+加密+字段全部通过（仅账号不存在）
//          data.ticket 存在表示登录成功 → 自动跟随 SSO 重定向验证门户会话
const fs = require('fs');
const path = require('path');
const rsa = require('./rsa30.js');

const CAS = 'https://wxcas.cwxu.edu.cn/lyuapServer';
const SERVICE = 'https://my.cwxu.edu.cn/shiro-cas';
const PUB_E = '010001';
const MOD = '00b5eeb166e069920e80bebd1fea4829d3d1f3216f2aabe79b6c47a3c18dcee5fd22c2e7ac519cab59198ece036dcf289ea8201e2a0b9ded307f8fb704136eaeb670286f5ad44e691005ba9ea5af04ada5367cd724b5a26fdb5120cc95b6431604bd219c6b7d83a6f8f24b43918ea988a76f93c333aa5a20991493d4eb1117e7b1';
const TAG = 'lyasp';
const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36';
const HERE = __dirname;
const STATE = path.join(require('os').tmpdir(), 'cas-probe-state.json');

function enc(plain) {
  rsa.a(131);
  const key = rsa.b(PUB_E, '', MOD);
  return rsa.c(key, plain);
}

// 极简内存 cookie jar：key=name, value=cookie 行
function jarHeaders(jar) {
  return Object.entries(jar).map(([k, v]) => `${k}=${v}`).join('; ');
}
function absorbCookies(jar, res) {
  for (const c of (res.headers.getSetCookie?.() ?? [])) {
    const [pair] = c.split(';');
    const i = pair.indexOf('=');
    if (i > 0) jar[pair.slice(0, i).trim()] = pair.slice(i + 1).trim();
  }
}

function readCreds(file) {
  const text = fs.readFileSync(file, 'utf8');
  const m = text.match(/账号\s*([^\s，,]+)\s*[，,]\s*密码\s*(\S+)/);
  if (!m) throw new Error('凭据文件格式应为：账号xxx，密码yyy');
  return { user: m[1], pass: m[2] };
}

async function main() {
  const args = process.argv.slice(2);
  let user, pass, answer;
  if (args[0] === '--captcha') {
    const r = await fetch(`${CAS}/kaptcha`, { headers: { 'User-Agent': UA } });
    const j = await r.json();
    fs.writeFileSync(path.join(HERE, 'captcha.png'), Buffer.from(j.content.split(',')[1], 'base64'));
    fs.writeFileSync(STATE, JSON.stringify({ uid: j.uid, kaptchaType: j.kaptchaType }));
    console.log(`kaptchaType=${j.kaptchaType} uid=${j.uid}`);
    console.log(`图片已存 ${path.join(HERE, 'captcha.png')}，看图后运行: node probe.js <学号> <密码> <答案>`);
    return;
  }
  if (args[0] === '--creds') {
    ({ user, pass } = readCreds(args[1]));
    answer = args[2];
  } else {
    [user, pass, answer] = args;
  }
  if (!user || !pass || !answer) { console.error('用法: node probe.js <学号> <密码> <答案> | node probe.js --creds <文件> <答案>'); process.exit(1); }
  const { uid } = JSON.parse(fs.readFileSync(STATE, 'utf8'));
  const ts = Date.now();
  const body = new URLSearchParams({
    username: user,
    password: enc(pass),        // 密码 RSA 密文，不打印
    service: SERVICE,
    loginType: '',
    id: uid,                    // 验证码 uid → v.id
    code: answer.trim(),        // 验证码答案 → v.code
    otpcode: '',
  });
  const jar = {};               // cookie jar（CAS + 门户）
  const r = await fetch(`${CAS}/v1/tickets`, {
    method: 'POST',
    headers: {
      'User-Agent': UA,
      'Content-Type': 'application/x-www-form-urlencoded',
      token: enc(TAG + ts),     // token 头 = RSA(TAG + 毫秒时间戳)
      Referer: `${CAS}/login?service=${encodeURIComponent(SERVICE)}`,
      Origin: 'https://wxcas.cwxu.edu.cn',
    },
    body: String(body),
  });
  console.log('CAS HTTP', r.status);
  absorbCookies(jar, r);
  const j = await r.json();
  console.log('响应:', JSON.stringify(j));
  if (typeof j?.data?.code === 'string') {           // 失败/特殊分支
    process.exit(2);
  }
  // 成功：data 可能是 {ticket,tgt} 对象，也可能直接是 ticket 字符串（旧版兼容）
  const ticket = typeof j?.data === 'string' ? j.data : (j?.data?.ticket ?? j?.ticket);
  const tgt = j?.tgt ?? j?.data?.tgt;
  if (!ticket) { console.error('未取到 ticket'); process.exit(3); }
  console.log('登录响应: ticket =', ticket.slice(0, 24) + '...', '| tgt =', (tgt ?? '').slice(0, 16) + '...');
  console.log('CAS 会话 Cookie:', Object.keys(jar).join(', ') || '(无)');

  // SSO 回跳：手动跟随重定向链，验证 my.cwxu.edu.cn 会话建立
  let url = `${SERVICE}?ticket=${encodeURIComponent(ticket)}`;
  for (let hop = 1; hop <= 6 && url; hop++) {
    let res;
    try {
      res = await fetch(url, { headers: { 'User-Agent': UA, Cookie: jarHeaders(jar) }, redirect: 'manual' });
    } catch (e) {
      console.log(`  跳${hop}: FETCH FAILED ${url} cause=${e.cause?.code ?? e.cause?.message ?? e.message}`);
      console.log('当前 Cookie:', Object.keys(jar).join(', ') || '(无)');
      break;
    }
    absorbCookies(jar, res);
    console.log(`  跳${hop}: ${res.status} ${url}`);
    const loc = res.headers.get('location');
    if (loc) { url = new URL(loc, url).href; continue; }
    url = null;
    const names = Object.keys(jar).join(', ');
    console.log('会话 Cookie:', names || '(无)');
    const ok = /shiro-cas|JSESSIONID|rememberMe/i.test(names);
    console.log(ok ? '★ 门户会话 Cookie 已建立' : '⚠ 未见门户会话 Cookie，需人工确认');
  }
  // 最终验证：带会话 GET 门户首页
  const home = await fetch('https://my.cwxu.edu.cn/', { headers: { 'User-Agent': UA, Cookie: jarHeaders(jar) }, redirect: 'manual' });
  absorbCookies(jar, home);
  console.log('门户首页:', home.status, home.headers.get('location') ?? '(无跳转)');
  if (home.status === 200) {
    const html = await home.text();
    console.log('首页标题:', (html.match(/<title>([^<]*)<\/title>/) ?? [, '(未找到)'])[1].trim());
  }
}

main().catch(e => { console.error('FAILED:', e.message); process.exit(1); });
