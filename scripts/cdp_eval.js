// cd_eval.js <expr> [targetSubstr]
const expr = process.argv[2] || '1+1';
const sub = process.argv[3] || 'main.asar/main.html';
(async () => {
  const list = await (await fetch('http://127.0.0.1:9222/json/list')).json();
  const t = list.find(x => x.type === 'page' && x.url.includes(sub)) || list[0];
  const ws = new WebSocket(t.webSocketDebuggerUrl);
  let id = 0; const pend = new Map();
  const send = (method, params={}) => new Promise(res => { const i=++id; pend.set(i,res); ws.send(JSON.stringify({id:i,method,params})); });
  ws.onmessage = ev => { const m = JSON.parse(ev.data); if (m.id && pend.has(m.id)) { pend.get(m.id)(m); pend.delete(m.id); } };
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true, userGesture: true });
  console.log(JSON.stringify(r.result, null, 2));
  ws.close();
})().catch(e => { console.error('ERR', e.message); process.exit(1); });
