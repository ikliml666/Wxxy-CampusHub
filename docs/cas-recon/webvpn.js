// WebVPN（深澜 Srun）CAS 联动登录探针
// 流程：GET / 取初始 wengine_vpn_ticket → CAS 登录(service=webvpn回调) → ticket 回跳建立 WebVPN 会话
// 用法：node webvpn.js --creds <凭据文件> <验证码答案>
const path = require('path');
const { UA, jarHeaders, absorbCookies, readCreds, casLogin } = require('./cas.js');

const WVPN = 'https://webvpn.cwxu.edu.cn';
const SERVICE = `${WVPN}/login?cas_login=true`;

async function main() {
  const args = process.argv.slice(2);
  if (args[0] !== '--creds') { console.error('用法: node webvpn.js --creds <凭据文件> <答案>'); process.exit(1); }
  const { user, pass } = readCreds(args[1]);
  const answer = args[2];

  // 1. 初始访问：种 wengine_vpn_ticket
  const jar = {};
  const r0 = await fetch(`${WVPN}/`, { headers: { 'User-Agent': UA }, redirect: 'manual' });
  absorbCookies(jar, r0);
  console.log('1. 初始访问:', r0.status, '→', r0.headers.get('location') ?? '(无跳转)');
  console.log('   初始 Cookie:', Object.keys(jar).join(', ') || '(无)');

  // 2. CAS 登录，service 指向 WebVPN 回调
  const { ticket } = await casLogin(user, pass, answer, SERVICE, {});
  console.log('2. CAS 登录成功: ticket =', ticket.slice(0, 24) + '...');

  // 3. ticket 回跳 WebVPN
  let url = `${SERVICE}&ticket=${encodeURIComponent(ticket)}`;
  for (let hop = 1; hop <= 6 && url; hop++) {
    let res;
    try {
      res = await fetch(url, { headers: { 'User-Agent': UA, Cookie: jarHeaders(jar) }, redirect: 'manual' });
    } catch (e) {
      console.log(`   跳${hop}: FETCH FAILED ${url} cause=${e.cause?.code ?? e.cause?.message ?? e.message}`);
      console.log('   当前 Cookie:', Object.keys(jar).join(', ') || '(无)');
      break;
    }
    absorbCookies(jar, res);
    console.log(`   跳${hop}: ${res.status} ${url}`);
    const loc = res.headers.get('location');
    if (loc) { url = new URL(loc, url).href; continue; }
    url = null;
  }
  console.log('3. WebVPN Cookie:', Object.keys(jar).join(', '));

  // 4. 登录态验证：再 GET / ——已登录应不再 302 到 /login
  const r1 = await fetch(`${WVPN}/`, { headers: { 'User-Agent': UA, Cookie: jarHeaders(jar) }, redirect: 'manual' });
  absorbCookies(jar, r1);
  const loc = r1.headers.get('location');
  const stillLogin = loc && /\/login/.test(loc);
  console.log('4. 首页复查:', r1.status, loc ? '→ ' + loc : '(无跳转)');
  console.log(stillLogin ? '✗ 仍未登录（又跳回 /login）' : '★ WebVPN 会话有效（未再要求登录）');
  if (r1.status === 200) {
    const html = await r1.text();
    const m = html.match(/<title>([^<]*)<\/title>/i);
    console.log('   首页标题:', m ? m[1].trim() : '(无 title)');
  }
}

main().catch(e => { console.error('FAILED:', e.message); process.exit(1); });
