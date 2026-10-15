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
assert.ok(css.includes('grid-template-columns:repeat(auto-fit,minmax(min(180px,100%),1fr))'))
assert.equal(style(root.querySelector('.np-row>div')).minWidth,'0')
assert.equal(style(root.querySelector('.np-input')).maxWidth,'100%')
const matrix=[]
for(const viewport of [320,350,375,390,768,1024,1440,1920]){
 for(const collapsed of [false,true]){
  const mobile=viewport<=760,host=mobile?0:240,padding=mobile?20:viewport<=1100?28:36,gap=mobile?0:viewport<=1100?16:24,sidebar=mobile?0:collapsed?68:viewport<=1100?176:208
  const content=viewport-host-padding-gap-sidebar
  assert.ok(content>0,'内容区域不能为负')
  if(!mobile)for(const count of [3,4]){
   const columns=Math.max(1,Math.min(count,Math.floor((content+14)/(180+14))))
   const card=(content-14*(columns-1))/columns
   assert.ok(card>=Math.min(content,180)-.01,'桌面参数最小宽度由内容区域保证')
   assert.ok(card*columns+14*(columns-1)<=content+.01)
  }
  matrix.push({viewport,collapsed,content})
 }
}
assert.ok(css.includes('.np-parameter-reading{flex-wrap:wrap}')&&css.includes('.np-parameter-reading span{white-space:nowrap}'))
assert.ok(css.includes('max-width:350px')&&css.includes('font-size:28px'))
assert.ok(css.includes('prefers-reduced-motion:reduce'))
assert.ok(css.includes('.np-lock-controls')&&css.includes('gap:16px'))
dom.window.close()
console.log(JSON.stringify({cases:matrix.length,viewportRange:'320–1920',theme:'两主题另由WebUI DOM测试覆盖',scope:'规则与约束，不是真实渲染或字体测量'}))
