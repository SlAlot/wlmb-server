// Offline DOM test for the native fnOS bridge and the reused signal components.
const {JSDOM}=require('./ui-tests/node_modules/jsdom');
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const root=path.resolve(__dirname,'..');
const html=fs.readFileSync(path.join(root,'fnos/app/www/index.html'),'utf8').replace(/<script[^>]*src=[^>]*><\/script>/g,'');
const dom=new JSDOM(html,{url:'http://127.0.0.1:1117/',runScripts:'outside-only',pretendToBeVisual:true});
const w=dom.window;const errors=[];w.addEventListener('error',e=>errors.push(e.error));
w.matchMedia=()=>({matches:false,addEventListener(){},removeEventListener(){}});w.HTMLElement.prototype.scrollIntoView=function(){};
w.fetch=async(_url,opts)=>{const request=JSON.parse(opts.body);return {ok:true,status:200,json:async()=>({ok:true,data:request.operation==='status'?{selectedModem:'fixture',modems:[{id:'fixture',model:'rm500u-cnv'}],capabilities:{signal:true,neighbors:true,carrierAggregation:true},radio:{stale:false,serving:[{rat:'NR',networkMode:'5G SA',band:'n78',rsrp:-82,rsrq:-2,sinr:14,pci:1,channel:620000,role:'pcc',stale:false}],neighbors:[],carriers:[]},system:{board:{model:'Photonicat'},stale:false},connection:{mode:'independent'}}:{}})}};
for(const file of ['webui-icons-1.1.3.js','webui-data-1.1.3.js','webui-shell-1.1.3.js'])w.eval(fs.readFileSync(path.join(root,'frontend/www/luci-static/resources/wlmb-server',file),'utf8'));
w.eval(fs.readFileSync(path.join(root,'fnos/app/www/app.js'),'utf8'));
setTimeout(()=>{try{assert.equal(errors.length,0,String(errors));assert.equal(w.document.querySelector('#workspace').hidden,false);assert(w.document.body.textContent.includes('RSRP'));assert(w.document.body.textContent.includes('fnOS'));assert(w.document.body.textContent.includes('−82'));assert(!w.document.body.textContent.includes('免费 OpenWrt 插件'));console.log('fnOS bridge and real signal components render successfully')}finally{w.close()}},250);
