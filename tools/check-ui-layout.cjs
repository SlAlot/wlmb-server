'use strict'
// 尺寸约束和样式规则回归，不模拟像素布局，也不冒充真实视觉验收。
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict')
const {JSDOM}=require('./ui-tests/node_modules/jsdom')
const css=fs.readFileSync(path.join(__dirname,'../frontend/www/luci-static/resources/wlmb-server/panel.css'),'utf8')
const dom=new JSDOM('<style></style><main id="wlmb-panel" class="np-root"><div class="np-row"><div>非常长的WiFi名称和说明</div><input class="np-input"></div></main>')
dom.window.document.querySelector('style').textContent=css
assert.ok(dom.window.document.styleSheets[0].cssRules.length>0)
const root=dom.window.document.querySelector('#wlmb-panel'),style=node=>dom.window.getComputedStyle(node)
assert.equal(style(root).getPropertyValue('--np-muted').trim(),'#78818f')
assert.equal(style(root.querySelector('.np-row>div')).minWidth,'0')
assert.equal(style(root.querySelector('.np-input')).maxWidth,'100%')
const matrix=[]
for(const viewport of [320,350,375,390,768,1024,1440,1920]){
 const mobile=viewport<=760,host=mobile?0:240,padding=mobile?20:viewport<=1100?28:36
 const content=Math.min(viewport-host-padding,1360)
 const columns=content>=720?Math.floor((content-28-40-270)*.43):Math.floor((content-28-10)/3)
 assert.ok(columns>=40,'最窄布局仍为每个指标保留独立空间')
 matrix.push({viewport,content,columns})
}
assert.ok(css.includes('container:wlmb-panel / inline-size'))
assert.ok(css.includes('@container wlmb-panel (min-width:720px)'))
assert.ok(css.includes('grid-template-columns:repeat(3,minmax(0,1fr))'))
assert.ok(css.includes('.np-hero-number { flex-wrap:wrap'))
assert.ok(css.includes('.np-primary-tabs { overflow-x:auto'))
assert.ok(css.includes('@container wlmb-panel (max-width:320px)'))
assert.ok(css.includes('.np-neighbor-filter { grid-template-columns:minmax(0,1fr) }'))
assert.ok(!/<animate\b/i.test(fs.readFileSync(path.join(__dirname,'../frontend/www/luci-static/resources/wlmb-server/signal-ribbon.svg'),'utf8')))
assert.ok(css.includes('prefers-reduced-motion:reduce'))
assert.ok(css.includes('.np-lock-controls')&&css.includes('gap:16px'))
dom.window.close()
console.log(JSON.stringify({cases:matrix.length,viewportRange:'320–1920',theme:'两主题另由WebUI DOM测试覆盖',scope:'规则与约束，不是真实渲染或字体测量'}))
