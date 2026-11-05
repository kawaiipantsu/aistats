// Regenerate the bundled 1200 × 630 social image with Node and Chromium.
// Uses only the vendored fonts and original SVG; no network or npm packages.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp, readFile, writeFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const temporary = await mkdtemp(join(tmpdir(), 'ai-stats-social-'));
const sources = JSON.parse(await readFile(join(root, 'assets/fonts/sources.json'), 'utf8'));
const fonts = await Promise.all(sources.map(async source => {
    const bytes = await readFile(join(root, `assets/fonts/${source.family}-latin.woff2`));
    return source.css.replace(/url\([^)]+\)/, `url(data:font/woff2;base64,${bytes.toString('base64')})`);
}));
const mascot = await readFile(join(root, 'assets/fun-mascot.svg'), 'utf8');
await writeFile(join(temporary, 'card.html'), `<!doctype html><html><head><meta charset="utf-8"><style>
${fonts.join('\n')}
*{box-sizing:border-box}body{margin:0;width:1200px;height:630px;overflow:hidden;background:#11131d;color:#f1eff8;font-family:'DM Sans',sans-serif}
.canvas{width:100%;height:100%;padding:55px 64px;position:relative;background:radial-gradient(ellipse at 92% 4%,#6650a44a,transparent 60%),radial-gradient(ellipse at 10% 110%,#40634540,transparent 65%)}
.dots{position:absolute;inset:0;background-image:radial-gradient(#b4abc325 1px,transparent 1px);background-size:25px 25px;z-index:0}
.copy{position:relative;z-index:1}.brand{font:700 25px 'Space Grotesk';letter-spacing:-1px}.brand span{color:#c3f69a}.brand small{font-size:13px;color:#b4b3c7;letter-spacing:2px;margin-left:17px;font-weight:500}
.eyebrow{margin:42px 0 18px;font:600 12px 'Space Grotesk';letter-spacing:3px;text-transform:uppercase;color:#d4c3ff}
h1{font:750 88px/1.04 'Bricolage Grotesque';letter-spacing:-5px;margin:0;width:740px}h1 em{color:#c3f69a;font-style:normal}
.deck{font-size:21px;color:#c4c4d5;line-height:1.6;margin:24px 0 25px}.tags{display:flex;gap:10px}.tags span{border:1px solid #49485e;border-radius:99px;padding:8px 14px;font:500 13px 'Space Grotesk';background:#262838;color:#dfdbea}
.art{position:absolute;width:335px;right:55px;top:145px;border:1px solid #5b5875;border-radius:18px;background:linear-gradient(140deg,#343147,#1c2b32);transform:rotate(5deg);box-shadow:14px 17px 0 #05070c50;z-index:1;overflow:hidden}
.chrome{font:12px 'Space Grotesk';padding:15px 18px;color:#c6c3d7;display:flex;justify-content:space-between;border-bottom:1px solid #555068}.chrome span:first-child{color:#d4c3ff;letter-spacing:3px}.mascot{width:100%;display:block}.caption{margin:0 0 23px;text-align:center;font:11px 'Space Grotesk';color:#c3f69a}
.foot{position:absolute;bottom:34px;left:64px;right:64px;display:flex;justify-content:space-between;align-items:center;border-top:1px solid #454257;padding-top:17px;font:500 12px 'Space Grotesk';color:#aaaec3;letter-spacing:1px}.foot strong{color:#c3f69a;font-weight:500}
</style></head><body><div class="canvas"><div class="dots"></div><div class="copy"><div class="brand">ai<span>stats</span><small>/ AFTER HOURS</small></div><p class="eyebrow">Your AI chats, by the numbers</p><h1>The logs<br>have <em>receipts.</em></h1><p class="deck">Big prompts. Tiny patience.<br>Questionable life choices, beautifully quantified.</p><div class="tags"><span>Swear jars</span><span>Night owls</span><span>AI whisperers</span></div></div><div class="art"><div class="chrome"><span>● ● ●</span><span>vibe-check.log</span></div>${mascot}<p class="caption">&gt;_ statistically concerned</p></div><div class="foot"><span>CLAUDE + CODEX · FUN STATS</span><strong>PSEUDONYMS ON. RECEIPTS READY.</strong></div></div></body></html>`);

const chrome = spawn(process.env.CHROMIUM || 'chromium', [
    '--headless', '--no-sandbox', '--disable-gpu', '--disable-background-networking',
    '--no-first-run', '--remote-debugging-port=0', `--user-data-dir=${join(temporary, 'profile')}`, 'about:blank',
], {stdio:['ignore','ignore','pipe']});
let socket;
const deadline = setTimeout(() => { chrome.kill(); }, 30_000);
try {
    const endpoint = await new Promise((resolve,reject) => {
        let output='';
        chrome.stderr.on('data', data => {
            output+=data.toString();
            const found=output.match(/DevTools listening on (ws:\/\/[^\s]+)/);
            if(found) resolve(found[1]);
        });
        chrome.once('exit', code => reject(new Error(`Chromium exited before rendering (${code})`)));
    });
    socket = new WebSocket(endpoint);
    await new Promise((resolve,reject) => {socket.onopen=resolve;socket.onerror=reject;});
    let id=0;
    const pending=new Map();
    socket.onmessage = event => {
        const response=JSON.parse(event.data), waiter=pending.get(response.id);
        if(!waiter) return;
        pending.delete(response.id);
        response.error ? waiter.reject(new Error(JSON.stringify(response.error))) : waiter.resolve(response.result);
    };
    const command=(method,params={},sessionId) => new Promise((resolve,reject) => {
        pending.set(++id,{resolve,reject});
        socket.send(JSON.stringify({id,method,params,sessionId}));
    });
    const {targetId}=await command('Target.createTarget',{url:'about:blank'});
    const {sessionId}=await command('Target.attachToTarget',{targetId,flatten:true});
    const send=(method,params={}) => command(method,params,sessionId);
    await send('Emulation.setDeviceMetricsOverride',{width:1200,height:630,deviceScaleFactor:1,mobile:false});
    await send('Page.navigate',{url:pathToFileURL(join(temporary,'card.html')).href});
    // Navigation plus font loading completes before taking the raster snapshot.
    const ready = await send('Runtime.evaluate',{
        expression:`new Promise(resolve => {const done=()=>document.fonts.ready.then(()=>resolve([...document.fonts].every(f=>f.status==='loaded'))); document.readyState==='complete'?done():window.addEventListener('load',done,{once:true});})`,
        awaitPromise:true, returnByValue:true,
    });
    assert.equal(ready.result.value,true,'Embedded fonts did not load');
    const {data}=await send('Page.captureScreenshot',{format:'png'});
    const output=join(root,'assets/ai-stats-social-v1.png');
    await writeFile(output,Buffer.from(data,'base64'));
    console.log(`Rendered ${output}`);
} finally {
    clearTimeout(deadline);
    socket?.close();
    if(chrome.exitCode === null) {
        const exited = new Promise(resolve=>chrome.once('exit',resolve));
        chrome.kill();
        await exited;
    }
    await rm(temporary,{recursive:true,force:true,maxRetries:5,retryDelay:100});
}
