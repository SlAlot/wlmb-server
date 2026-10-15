// 网络面板自有前端；仅处理展示和已授权RPC调用，不直接操作模组。
'use strict';
'require view';
'require rpc';
'require poll';
'require ui';

var statusRPC = rpc.declare({ object: 'network-panel', method: 'status', params: ['modem'] });
var settingsRPC = rpc.declare({ object: 'network-panel', method: 'settings' });
var refreshRPC = rpc.declare({ object: 'network-panel', method: 'refresh', params: ['modem'] });
var saveRPC = rpc.declare({ object: 'network-panel', method: 'configure', params: ['interval', 'allowUnsafeAt'] });
var serviceRPC = rpc.declare({ object: 'network-panel', method: 'service_status' });
var controlRPC = rpc.declare({ object: 'network-panel', method: 'service_control', params: ['action', 'enabled'] });
var prepareRPC = rpc.declare({ object: 'network-panel', method: 'at_prepare', params: ['modem', 'command'] });
var executeRPC = rpc.declare({ object: 'network-panel', method: 'at_execute', params: ['modem', 'command', 'confirmation'] });
var diagnosticsRPC = rpc.declare({ object: 'network-panel', method: 'diagnostics' });
var lockStatusRPC = rpc.declare({ object: 'network-panel', method: 'lock_status', params: ['modem', 'force'] });
var lockPrepareRPC = rpc.declare({ object: 'network-panel', method: 'lock_prepare', params: ['modem', 'operation'] });
var lockExecuteRPC = rpc.declare({ object: 'network-panel', method: 'lock_execute', params: ['modem', 'operation', 'revision', 'confirmation'] });
var resultRPC = rpc.declare({ object: 'network-panel', method: 'control_result', params: ['job'] });
var hostPrepareRPC = rpc.declare({ object: 'network-panel', method: 'host_prepare', params: ['operation'] });
var hostExecuteRPC = rpc.declare({ object: 'network-panel', method: 'host_execute', params: ['operation', 'revision', 'confirmation'] });
var smsListRPC = rpc.declare({ object: 'network-panel', method: 'sms_list', params: ['modem'] });
var smsSettingsPrepareRPC = rpc.declare({ object: 'network-panel', method: 'sms_settings_prepare', params: ['modem', 'enabled'] });
var smsSettingsExecuteRPC = rpc.declare({ object: 'network-panel', method: 'sms_settings_execute', params: ['modem', 'enabled', 'confirmation'] });
var smsRefreshRPC = rpc.declare({ object: 'network-panel', method: 'sms_refresh', params: ['modem'] });
var smsReadStateRPC = rpc.declare({ object: 'network-panel', method: 'sms_read_state', params: ['modem', 'id', 'unread'] });
var smsPrepareRPC = rpc.declare({ object: 'network-panel', method: 'sms_prepare', params: ['modem', 'operation'] });
var smsExecuteRPC = rpc.declare({ object: 'network-panel', method: 'sms_execute', params: ['modem', 'operation', 'confirmation'] });
var forwardGetRPC = rpc.declare({ object: 'network-panel', method: 'sms_forward_settings' });
var forwardSaveRPC = rpc.declare({ object: 'network-panel', method: 'sms_forward_configure', params: ['enabled', 'modem', 'pollSeconds', 'token', 'clearToken', 'receiveEnabled'] });
function task(promise) {
    return promise.then(unwrap).then(function(receipt) {
        if (!receipt.job) throw new Error('没有操作回执，请检查插件版本');
        var until = Date.now() + 60000;
        function wait() {
            if (Date.now() > until) throw new Error('结果未确认；操作未重发，请先读取状态');
            return new Promise(function(resolve) { setTimeout(resolve, 500); }).then(function() { return resultRPC(receipt.job); }).then(unwrap).then(function(row) {
                if (row.state === 'done') return row.result;
                if (row.state === 'error') throw new Error(row.error && row.error.message || '操作未确认');
                return wait();
            });
        }
        return wait();
    });
}
function unwrap(value) {
    if (!value || value.ok !== true) throw new Error(value && value.error && value.error.message || '服务暂不可用');
    return value.data;
}
function optional(promise) { return promise.then(unwrap).catch(function(error) { return { failure: error.message }; }); }
function retainStatus(previous, value, modem, receivedAt, now) {
    if (!value.failure || !previous || previous.failure) return value;
    var elapsed=now-receivedAt, same=previous.selectedModem===modem;
    var copy=Object.assign({},previous,{failure:'状态暂未更新：'+value.failure});
    copy.radio=Object.assign({},same?previous.radio:{serving:[],carriers:[],neighbors:[],pending:true,stale:true});
    copy.system=Object.assign({},previous.system);
    copy.selectedModem=modem;
    if(!same)copy.capabilities={};
    var age=Number.isFinite(copy.radio.ageMs)?copy.radio.ageMs:0;
    var serverAt=Number.isFinite(copy.radio.observedAtMs)?copy.radio.observedAtMs+age:null;
    function expired(at,limit){return elapsed<0||elapsed+(Number.isFinite(serverAt)&&Number.isFinite(at)?Math.max(0,serverAt-at):0)>limit;}
    ['serving','carriers','neighbors'].forEach(function(key){copy.radio[key]=(copy.radio[key]||[]).map(function(row){var next=Object.assign({},row,{stale:!!row.stale||expired(row.observedAtMs,15000)});Object.keys(row.metricObservedAtMs||{}).forEach(function(metric){if(expired(row.metricObservedAtMs[metric],15000))next[metric]=null;});return next;});});
    copy.radio.ageMs=age+Math.max(0,elapsed);
    copy.radio.stale=!!copy.radio.stale||!same||elapsed<0||age+elapsed>15000;
    copy.system.stale=!!copy.system.stale||expired(copy.system.observedAtMs,15000);
    return copy;
}
function text(value, unit) { return value == null || value === '' ? '—' : String(value) + (unit || ''); }
function band(row) { return row.band || (row.bandCandidates || []).join(' / ') + ((row.bandCandidates || []).length ? '（候选）' : '') || '未知频段'; }
function speed(value) { return typeof value === 'number' ? (value * 8 / 1000000).toFixed(2) + ' Mbps' : '—'; }
function mb(value) { return typeof value === 'number' ? (value / 1048576).toFixed(0) + ' MB' : '—'; }
function uptime(value) { return typeof value === 'number' ? Math.floor(value / 86400) + ' 天 ' + Math.floor(value % 86400 / 3600) + ' 小时 ' + Math.floor(value % 3600 / 60) + ' 分钟' : '—'; }
function button(label, action, style) {
    var node = E('button', { type: 'button', 'class': 'np-btn ' + (style || '') }, label);
    node.addEventListener('click', function() {
        if (node.disabled || node.busy) return;
        node.busy = true; node.disabled = true; node.setAttribute('aria-busy', 'true');
        Promise.resolve().then(action).catch(function(error) { ui.addNotification(null, E('p', {}, error.message), 'error'); })
            .finally(function() { node.busy = false; node.setAttribute('aria-busy', 'false'); node.disabled = typeof node.isBlocked === 'function' ? node.isBlocked() : false; });
    });
    return node;
}
function link(label, href) {
    return E('a', { 'class': 'np-btn outline np-help-link', href: href, target: '_blank', rel: 'noopener noreferrer', click: function(event) {
        var node=event.currentTarget,doc=node.ownerDocument,root=doc.getElementById('wlmb-panel');
        if(!root)return;
        var url=new doc.defaultView.URL(node.href),mode=doc.defaultView.getComputedStyle(root).getPropertyValue('--np-scheme').trim();
        if(mode==='light'||mode==='dark')url.searchParams.set('appearance',mode);
        node.href=url.href;
    } }, label);
}
function row(label, hint, control) { return E('div', { 'class': 'np-row' }, [E('div', {}, [E('div', { 'class': 'np-label' }, label), E('div', { 'class': 'np-hint' }, hint || '')]), control]); }
function toggle(label, checked, action) {
    var input = E('input', { type: 'checkbox', 'aria-label': label }); input.checked = checked;
    input.addEventListener('change', function() {
        if (node.busy) { input.checked = node.pendingValue; return; }
        var next = input.checked; input.disabled = true; node.busy = true; node.pendingValue = next; node.setAttribute('aria-busy', 'true');
        Promise.resolve().then(function() { return action(next); }).catch(function(error) {
            input.checked = !next; ui.addNotification(null, E('p', {}, error.message), 'error');
        }).finally(function() { node.busy = false; node.pendingValue = null; node.setAttribute('aria-busy', 'false'); input.disabled = typeof input.isBlocked==='function'?input.isBlocked():false; });
    });
    var node = E('label', { 'class': 'np-toggle' }, [input, E('span', { 'class': 'np-track' })]);
    node.input = input; return node;
}
function card(title, children) { return E('section', { 'class': 'np-card np-extension-card' }, [E('div', { 'class': 'np-card-head' }, E('div', { 'class': 'np-card-title', role: 'heading', 'aria-level': '3' }, title)), E('div', { 'class': 'np-card-body' }, children)]); }
function panelModal(title, children) {
    var dialog = ui.showModal(title, children, 'np-root', 'np-dialog');
    var root = typeof document !== 'undefined' && document.getElementById('wlmb-panel');
    if (dialog && root && typeof window !== 'undefined') {
        var palette = window.getComputedStyle(root);
        ['scheme','accent','text','muted','line','card','soft','bg','track','blue','orange','green','purple','danger'].forEach(function(name) {
            var value = palette.getPropertyValue('--np-' + name).trim();
            if (value) dialog.style.setProperty('--np-' + name, value);
        });
        dialog.setAttribute('data-np-theme', palette.getPropertyValue('--np-scheme').trim() || 'light');
        if (dialog.npEntry) dialog.npEntry.cancel();
        if (typeof dialog.animate === 'function' && (!window.matchMedia || !window.matchMedia('(prefers-reduced-motion: reduce)').matches)) {
            dialog.npEntry = dialog.animate([{ opacity:0, transform:'translateY(8px) scale(.985)' }, { opacity:1, transform:'translateY(0) scale(1)' }], { duration:180, easing:'cubic-bezier(.2,.7,.2,1)' });
            dialog.npEntry.finished.catch(function() {});
        }
    }
    return dialog;
}
function fitLuci(root) {
    var owned=[],surfaces=[];
    function mark(node,name){if(!node.classList.contains(name)){node.classList.add(name);owned.push([node,name]);}}
    function surface(node){mark(node,'np-luci-surface');['surface','text'].forEach(function(name){var property='--np-page-'+name;surfaces.push({node:node,property:property,value:node.style.getPropertyValue(property),priority:node.style.getPropertyPriority(property),last:null});});}
    for(var node=root.parentElement;node&&node!==document.body;node=node.parentElement){
        var name=node.matches('.luci-content, #maincontent, #view, .container')?'np-luci-fullwidth':
            node.matches('main, .main')&&document.body.classList.contains('c2000-ui')?'np-luci-host':null;
        if(name)mark(node,name);
        if(name||node.matches('.main-right, main'))surface(node);
    }
    surface(document.body);
    function sync(){var palette=window.getComputedStyle(root);surfaces.forEach(function(row){var color=palette.getPropertyValue(row.property==='--np-page-surface'?'--np-bg':'--np-text').trim();if(!color)return;row.node.style.setProperty(row.property,color);row.last=row.node.style.getPropertyValue(row.property);});}
    var restore=function(){surfaces.forEach(function(row){if(row.node.style.getPropertyValue(row.property)!==row.last)return;if(row.value)row.node.style.setProperty(row.property,row.value,row.priority);else row.node.style.removeProperty(row.property);});owned.forEach(function(item){item[0].classList.remove(item[1]);});};
    restore.sync=sync;sync();return restore;
}
function copyCode(value) {
    function fallback(){
        var previous=document.activeElement,selection=window.getSelection(),ranges=[];
        if(selection)for(var i=0;i<selection.rangeCount;i++)ranges.push(selection.getRangeAt(i).cloneRange());
        var node=document.createElement('textarea');node.value=value;node.readOnly=true;
        node.setAttribute('aria-label','复制验证码');node.style.cssText='position:fixed;left:-9999px;top:0;opacity:0';
        document.body.appendChild(node);
        try{node.focus({preventScroll:true});node.select();if(!document.execCommand||!document.execCommand('copy'))throw new Error('复制失败，请手动选择验证码');}
        finally{node.remove();if(previous&&previous.isConnected)previous.focus({preventScroll:true});if(selection){selection.removeAllRanges();ranges.forEach(function(range){selection.addRange(range);});}}
    }
    if(window.isSecureContext&&navigator.clipboard&&navigator.clipboard.writeText)return navigator.clipboard.writeText(value).catch(fallback);
    try{fallback();return Promise.resolve();}catch(error){return Promise.reject(error);}
}
var guide = function(hash) { return L.resource('wlmb-server/guide.html') + '?v=1.0.3&rev=release1_0_3' + (hash || ''); };
function webuiScript(name) {
    return new Promise(function(resolve,reject) {
        var script = document.createElement('script'), settled = false;
        var timer = setTimeout(function() { settled=true; script.remove(); reject(new Error('前端组件加载超时')); },12000);
        script.src = L.resource('wlmb-server/' + name) + '?v=1.0.3&rev=release1_0_3';
        script.onload = function() { if (!settled) { settled=true; clearTimeout(timer); resolve(); } };
        script.onerror = function() { if (!settled) { settled=true; clearTimeout(timer); script.remove(); reject(new Error('前端组件加载失败')); } };
        document.head.appendChild(script);
    });
}

return view.extend({
    handleSaveApply: null, handleSave: null, handleReset: null,
    load: function() {
        return Promise.all([optional(serviceRPC()), optional(statusRPC('')), optional(settingsRPC())]);
    },
    render: function(data) {
        var self = this;
        this.runtime = data[0]; this.selected = data[1].selectedModem || ''; this.lastRuntime = 0;
        this.lastGoodStatus=data[1].failure?null:data[1];this.lastGoodStatusAt=Date.now();this.updateToken=null;
        this.recentSms={modem:this.selected,loaded:false,messages:[]};this.recentSmsLast=0;this.recentSmsPending=null;this.smsGeneration={};
        this.readonly = typeof L.hasViewPermission === 'function' && !L.hasViewPermission();
        this.selector = E('select', { 'class': 'np-input', 'aria-label': '选择模组' });
        this.selector.addEventListener('change', function() { self.selected = self.selector.value; self.update(true); });
        this.state = E('span', {}, '检查中'); this.dot = E('span', { 'class': 'np-dot' });
        this.count = E('strong', {}, '0'); this.time = E('strong', {}, '—'); this.controlHint = E('p', { 'class': 'np-hint' });
        this.startButton = button('启动服务', function() { return self.control('start'); }, 'primary');
        this.stopButton = button('停止服务', function() { return self.confirm('stop'); }, 'danger');
        this.restartButton = button('重启服务', function() { return self.confirm('restart'); }, 'outline');
        this.startButton.isBlocked = function() { return self.readonly || !!self.runtime.running; };
        this.stopButton.isBlocked = this.restartButton.isBlocked = function() { return self.readonly || !self.runtime.running; };
        this.boot = toggle('开机自动启动', !!this.runtime.bootEnabled, function(value) { return self.control('boot', value); });
        var controls = card('运行控制', [
            row('开启网络面板', '只控制本插件，不影响拨号和路由器供网', E('div', { 'class': 'np-actions' }, [this.startButton, this.stopButton, this.restartButton])),
            row('开机自动启动', '关闭后仍可手动启动', this.boot), this.controlHint]);
        this.overview = E('div'); this.cells = E('div'); this.networks = E('div'); this.flow = E('div');
        this.pages = [E('div', {}, [this.overview]), this.cells, this.consolePage(), E('div',{},[controls,this.settingsPage(data[2])]), this.networks, this.lockPage(), this.smsPage(), this.forwardPage(),this.flow];
        var tabs = E('div', { 'class': 'np-tabs', role: 'tablist', 'aria-label': '网络面板功能' });
        this.tabs = tabs;
        this.activePage = 0;
        this.selectPage = function(index) {
            self.activePage = index;
            self.pages.forEach(function(page,i) { page.hidden = i !== index; });
            Array.from(tabs.children).forEach(function(node,i) { node.classList.toggle('selected',i === index); node.setAttribute('aria-selected',String(i === index)); });
            if (index === 6 && self.capabilities.sms) self.readSms(false).catch(function(e) { ui.addNotification(null,E('p',{},e.message),'error'); });
            if ((index === 6 || index === 7) && self.capabilities.sms) self.readForward().catch(function(e) { ui.addNotification(null,E('p',{},e.message),'error'); });
            if (index === 0) self.readRecentSms().catch(function(){});
        };
        ['网络信息', '邻区扫描', 'AT 控制台', '插件设置', 'WiFi 与设备', '频段与小区', '短信', '短信转发', '设备与流量'].forEach(function(label, index) {
            var tab = E('button', { type: 'button', 'class': 'np-tab', role: 'tab', id: 'np-tab-' + index, 'aria-selected': String(index === 0), 'aria-controls': 'np-page-' + index }, label);
            self.pages[index].id = 'np-page-' + index; self.pages[index].hidden = index !== 0;
            self.pages[index].setAttribute('role', 'tabpanel'); self.pages[index].setAttribute('aria-labelledby', tab.id);
            tab.classList.toggle('selected', index === 0);
            tab.addEventListener('click', function() {
                self.selectPage(index);
            }); tabs.appendChild(tab);
        });
        this.draw(data[1]);
        poll.add(function() { return self.update(false); }, 3);
        var root = E('div', { id: 'wlmb-panel', 'class': 'np-root' }, [
            E('link', { rel: 'stylesheet', href: L.resource('wlmb-server/panel.css') + '?v=1.0.3&rev=release1_0_3' }),
            E('div', { 'class': 'np-head' }, [E('div', {}, [E('div', { 'class': 'np-title', role: 'heading', 'aria-level': '2' }, '网络面板'), E('p', { 'class': 'np-desc' }, 'wlmb-server · 根据qmodem为参考开发接入')]), link('新手教程', guide())]),
            E('div', { 'class': 'np-statusbar', role: 'status', 'aria-live': 'polite' }, [
                E('span', { 'class': 'np-chip' }, [this.dot, this.state]), E('span', { 'class': 'np-chip' }, ['模组 ', this.count]),
                E('span', { 'class': 'np-chip' }, ['更新 ', this.time]), E('span', { 'class': 'np-chip' }, '专用端口 1117')]),
            E('aside', { 'class': 'np-guide' }, [E('div', {}, [E('strong', {}, '第一次使用？'), E('p', { 'class': 'np-hint' }, '启动服务 → 检查模组通道 → 小程序连接。')]), link('查看连接教程', guide('#connect'))]),
            E('div', { 'class': 'np-toolbar' }, [this.selector, button('刷新数据', function() {
                if (!self.runtime.running || !self.selected) throw new Error('请先启动服务并选择已识别模组');
                return task(refreshRPC(self.selected)).then(function() { return self.update(true); });
            }, 'outline')]), tabs].concat(this.pages).concat([E('footer', { 'class': 'np-footer' }, 'SlAlot · 网络面板 ' + (this.runtime.version || '开发版'))]));
        if (typeof window !== 'undefined' && typeof document !== 'undefined') setTimeout(function() {
            if (!root.isConnected) return;
            var restoreLayout=fitLuci(root),shell=null;
            var lifetime = new MutationObserver(function() {
                if(!root.isConnected){if(shell)shell.unmount();restoreLayout();lifetime.disconnect();if(self.webuiBridge===bridge){self.webuiBridge=null;self.smsGeneration={};}}
            });
            lifetime.observe(document.body,{childList:true,subtree:true});
            var original = Array.from(root.children).filter(function(node) { return node.tagName !== 'LINK'; });
            var statusbar = root.querySelector('.np-statusbar'), toolbar = root.querySelector('.np-toolbar'), footer = root.querySelector('.np-footer');
            var pageContainer = E('div', { 'class': 'np-page-container' });
            var mount = E('div', { 'class': 'np-webui-mount' });
            root.appendChild(mount);
            var bridge = {tabs:tabs,active:self.activePage,status:statusbar,toolbar:toolbar,pages:pageContainer,footer:footer,guide:guide(),select:self.selectPage,snapshot:self.snapshot,syncSurface:restoreLayout.sync,
                runtime:function(){return self.runtime;},
                canRefresh:function(){return !!self.runtime.running && !self.runtime.failure && !self.snapshot.failure && !!self.selected && !self.readonly;},
                refresh:function(){return task(refreshRPC(self.selected)).then(function(){return self.update(true);});},
                chooseCell:function(item){self.fillLockCandidate(item.source);self.selectPage(5);},
                sms:function(){return self.recentSms;},
                canReadSms:function(){return !self.smsBlocked('sms')&&!self.recentSmsPending;},
                readSms:function(){return self.readSms(true);},
                copySmsCode:function(item){return self.copySmsCode(item);},
                notify:function(error){ui.addNotification(null,E('p',{},error.message),'error');},
                dark:function() { return window.getComputedStyle(root).getPropertyValue('--np-scheme').trim() === 'dark'; },
                ready:function() { original.forEach(function(node) { if (node.parentNode === root) node.hidden=true; }); root.classList.add('np-webui-ready'); },
                fail:function(error) {
                    if(!root.isConnected)return;
                    original.forEach(function(node) { root.appendChild(node); node.hidden=false; });
                    self.selectPage(self.activePage); mount.hidden=true; root.classList.remove('np-webui-ready');
                    ui.addNotification(null,E('p',{},error.message+'；当前使用兼容界面，操作接口保持不变。'),'error');
                }};
            self.webuiBridge = bridge;
            (window.WLMBIcons && window.WLMBIcons.version==='1.0.3' ? Promise.resolve() : webuiScript('webui-icons-1.0.3.js')).then(function() {
                return window.WLMBUIData && window.WLMBUIData.version==='1.0.3' ? null : webuiScript('webui-data-1.0.3.js');
            }).then(function() {
                return window.WLMBWebUI && window.WLMBWebUI.version==='1.0.3' ? null : webuiScript('webui-shell-1.0.3.js');
            }).then(function() {
                if (!root.isConnected) return;
                self.pages.forEach(function(page) { pageContainer.appendChild(page); });
                shell = window.WLMBWebUI.mount(mount,bridge);
                self.readRecentSms().catch(function(){});
            }).catch(bridge.fail);
        },0);
        return root;
    },
    smsPage: function() {
        var self = this;
        this.receiveEnabled=toggle('自动收取短信',false,function(enabled){
            if(self.smsBlocked('sms'))throw new Error('短信通道未就绪');
            return forwardGetRPC().then(unwrap).then(function(value){
                return task(forwardSaveRPC(!!value.enabled,self.selected,Math.max(value.minimumPollSeconds||5,value.pollSeconds||10),'',false,enabled));
            }).then(function(){return self.readForward();});
        });
        this.receiveNote=E('p',{'class':'np-hint'},'事件优先，慢速查询兜底；与转发共用缓存，不修改模组短信设置。');
        this.smsRows = E('div', {}, E('p', { 'class': 'np-hint' }, '点击读取短信。仅按需查询，不跟随信号刷新轮询。'));
        this.smsTo = E('input', { 'class': 'np-input np-entry', type: 'tel', maxlength: 21, 'aria-label': '短信收件人', placeholder: '收件人，例如 10086' });
        this.smsBody = E('textarea', { 'class': 'np-input np-entry', maxlength: 70, rows: 4, 'aria-label': '短信内容', placeholder: '单条短信，最多 70 个 UTF-16 单元' });
        this.smsReadButton = button('读取短信', function() { return self.readSms(true); }, 'outline');
        this.smsReadButton.isBlocked = function() { return self.smsBlocked('sms'); };
        this.smsSendButton = button('确认并发送', function() {
            if (self.smsBlocked('smsSend')) throw new Error('当前通道未开放发送短信');
            var modem = self.selected, operation = { type: 'send', number: self.smsTo.value.trim(), content: self.smsBody.value };
            if (!modem || !operation.number || !operation.content) throw new Error('请选择模组并填写号码、内容');
            return smsPrepareRPC(modem, JSON.stringify(operation)).then(unwrap).then(function(prepared) {
                return new Promise(function(resolve, reject) {
                    panelModal('发送短信？', [E('p', {}, prepared.warning), E('p', {}, '收件人：' + operation.number), E('div', { 'class': 'np-actions right' }, [
                        E('button', { 'class': 'np-btn outline', click: function() { ui.hideModal(); resolve(); } }, '取消'),
                        button('确认发送一次', function() {
                            ui.hideModal();
                            if (self.smsBlocked('smsSend')) { reject(new Error('服务或写权限已改变，请重新确认')); return; }
                            if (self.selected !== modem) { reject(new Error('模组已更换，请重新确认')); return; }
                            return task(smsExecuteRPC(modem, JSON.stringify(operation), prepared.confirmation)).then(function() { self.smsBody.value = ''; ui.addNotification(null, E('p', {}, '短信已发送。设备是否保存发件记录由模组决定。')); resolve(); }, reject);
                        }, 'primary')])]);
                });
            });
        }, 'primary');
        this.smsSendButton.isBlocked = function(){return self.smsBlocked('smsSend');};
        return E('div', {}, [card('设备短信', [row('自动收取短信','独立开关；转发开启时也会启动收取',this.receiveEnabled),this.receiveNote,E('p',{'class':'np-hint'},'已读由插件统一管理；首次读取收件短信默认为未读。查询可能改变设备后台的未读标记。'),E('div', { 'class': 'np-actions' }, [this.smsReadButton]), this.smsRows]), card('发送短信', [
            E('p', { 'class': 'np-hint' }, '使用当前模组，不切换 SIM 卡；发送可能产生费用，超时不自动重发。'), this.smsTo, this.smsBody, this.smsSendButton])]);
    },
    readSms: function(refresh) {
        var self = this, modem = this.selected,generation=this.smsGeneration;
        if (refresh && this.smsBlocked('sms')) return Promise.reject(new Error('需要已运行服务、短信能力和写权限'));
        if (!modem) return Promise.reject(new Error('请先选择已识别的模组'));
        if(this.recentSmsPending&&this.recentSmsPending.modem===modem)return this.recentSmsPending.promise;
        var pending={modem:modem};this.recentSmsPending=pending;
        pending.promise=(refresh ? task(smsRefreshRPC(modem)) : Promise.resolve()).then(function() { return smsListRPC(modem); }).then(unwrap).then(function(value) {
            if (self.selected !== modem||self.smsGeneration!==generation) return;
            self.recentSms=Object.assign({},value,{modem:modem});self.recentSmsLast=Date.now();
            self.smsLoadedModem = value.loaded ? modem : null;
            var rows = value.messages || [];
            self.smsRows.replaceChildren.apply(self.smsRows, rows.length ? rows.map(function(item) {
                var mark = item.direction === 'incoming' ? button(item.unread ? '标为已读' : '标为未读',function(){
                    if(self.selected!==modem||self.smsBlocked('sms'))throw new Error('权限或模组已改变');
                    return smsReadStateRPC(modem,item.id,!item.unread).then(unwrap).then(function(){return self.readSms(false);});
                },'outline') : E('span');
                mark.disabled = self.readonly;
                var body=item.complete===false?'长短信分段 '+item.pendingPart+'/'+item.expectedParts+' · 接收中\n'+item.content:item.content;
                return E('article', { 'class': 'np-sms-message' }, [E('div', { 'class': 'np-row' }, [E('strong', {}, item.phone), E('span', { 'class': 'np-hint' }, item.direction === 'incoming' ? (item.unread?'收件箱 · 未读':'收件箱 · 已读') : item.direction === 'outgoing' ? '已发送' : '未分类')]), E('p', { 'class': 'np-hint' }, item.timestamp || '设备未提供时间'), E('p', { 'class': 'np-sms-body' }, body), E('div',{'class':'np-actions'},[mark,self.smsBlocked('smsDelete') ? E('span') : button('删除', function() { return self.deleteSms(modem, item.id); }, 'danger')])]);
            }) : [E('p', { 'class': 'np-hint' }, value.loaded ? '当前存储区暂无短信' : '尚未查询，请点击「读取短信」。不自动切换存储区或短信编码。')]);
        }).catch(function(error) {
            if(self.selected===modem&&self.smsGeneration===generation){self.recentSmsLast=Date.now();self.recentSms=Object.assign({},self.recentSms,{error:error.message,stale:true});self.smsRows.textContent=error.message;}
            throw error;
        }).finally(function(){
            if(self.recentSmsPending===pending)self.recentSmsPending=null;
            if(self.smsGeneration===generation&&self.webuiBridge&&self.webuiBridge.redraw)self.webuiBridge.redraw();
        });
        return pending.promise;
    },
    readRecentSms: function() {
        if(!this.runtime.running||this.runtime.failure||this.snapshot.failure||!this.selected||!this.capabilities.sms||![0,6].includes(this.activePage)||Date.now()-this.recentSmsLast<(this.activePage===6?3000:30000))return Promise.resolve();
        return this.readSms(false);
    },
    copySmsCode: function(item) {
        var state=this.recentSms||{},data=window.WLMBUIData;
        var message=state.modem===this.selected&&(state.messages||[]).find(function(row){return row.id===item.id;});
        var code=message&&message.complete!==false&&message.direction!=='outgoing'&&data&&data.verificationCode(message.content);
        if(!code||code!==item.code)return Promise.reject(new Error('短信已更新，请重新选择验证码'));
        return copyCode(code).then(function(){ui.addNotification(null,E('p',{},'验证码已复制'));});
    },
    deleteSms: function(modem, id) {
        var self = this, operation = JSON.stringify({ type: 'delete', id: id });
        if (this.smsBlocked('smsDelete')) return Promise.reject(new Error('需要已运行服务、删除能力和写权限'));
        return smsPrepareRPC(modem, operation).then(unwrap).then(function(prepared) {
            return new Promise(function(resolve, reject) {
                panelModal('删除这条短信？', [E('p', {}, prepared.warning), E('div', { 'class': 'np-actions right' }, [E('button', { 'class': 'np-btn outline', click: function() { ui.hideModal(); resolve(); } }, '取消'), button('确认删除', function() {
                    ui.hideModal(); if (self.selected !== modem || self.smsBlocked('smsDelete')) { reject(new Error('模组、服务或写权限已改变，请重新读取')); return; }
                    return task(smsExecuteRPC(modem, operation, prepared.confirmation)).then(function() { return self.readSms(false); }).then(resolve, reject);
                }, 'danger')])]);
            });
        });
    },
    forwardPage: function() {
        var self = this;
        this.forwardEnabled = toggle('启用短信转发', false, function() { return Promise.resolve(); });
        this.forwardToken = E('input', { 'class': 'np-input np-entry', type: 'password', autocomplete: 'new-password', maxlength: 128, 'aria-label': 'PushPlus Token', placeholder: '填写 Token；留空保留已保存的 Token' });
        this.forwardInterval = E('input', { 'class': 'np-input np-number', type: 'number', min: 5, max: 300, value: 10, 'aria-label': '短信兜底查询间隔' });
        this.forwardState = E('p', { 'class': 'np-hint', role: 'status' }, '点击此页读取配置。');
        this.forwardUnavailable = E('p', { 'class': 'np-notice' });
        this.forwardSaveButton = button('保存短信转发', function() {
            if (self.smsBlocked('smsForward')) throw new Error('需要已运行服务、转发能力和写权限');
            var token = self.forwardToken.value, modem = self.selected;
            if (self.forwardEnabled.input.checked && !modem) throw new Error('请先选择模组');
            self.forwardToken.value = '';
            return task(forwardSaveRPC(self.forwardEnabled.input.checked, modem, Number(self.forwardInterval.value), token, false,!!self.receiveEnabled.input.checked)).then(function() { return self.readForward(); });
        }, 'primary');
        this.forwardClearButton = button('关闭并删除 Token', function() {
            if (self.smsBlocked('smsForward')) throw new Error('需要已运行服务、转发能力和写权限');
            self.forwardToken.value = '';
            return task(forwardSaveRPC(false, self.selected, Number(self.forwardInterval.value), '', true,!!self.receiveEnabled.input.checked)).then(function() { return self.readForward(); });
        }, 'outline');
        this.forwardSaveButton.isBlocked = this.forwardClearButton.isBlocked = function() { return self.smsBlocked('smsForward'); };
        return E('div', {}, card('PushPlus 短信转发', [row('转发新收到的短信', '首次开启不推送历史；关闭后停止转发任务，独立收取开关不受影响', this.forwardEnabled),
            row('兜底查询间隔', '新短信事件优先收取；默认 10 秒兜底（扫描完成后计时）。忙碌会退避；推送按提供商限制排队', this.forwardInterval), this.forwardUnavailable, this.forwardToken, this.forwardState,
            E('div', { 'class': 'np-actions' }, [this.forwardSaveButton, this.forwardClearButton]), E('p', { 'class': 'np-hint' }, 'Token 只保存在设备私有目录，不会回传。短信内容通过 HTTPS 发送给 PushPlus，启用前请确认其隐私政策和账号配置。'), E('div', {'class':'np-help-actions'}, link('查看转发教程', guide('#sms-forward')))]));
    },
    readForward: function() {
        var self = this;
        return forwardGetRPC().then(unwrap).then(function(value) {
            self.forwardEnabled.input.checked = !!value.enabled; self.forwardInterval.value = value.pollSeconds || 10; self.forwardInterval.min = value.minimumPollSeconds || 5;
            if(!self.receiveEnabled.busy)self.receiveEnabled.input.checked=!!value.receiveEnabled;
            var state = value.runtime || {}, labels = { disabled: '已关闭', starting: '建立基线中', running: '运行中', waiting: '等待重试', stopping: '正在停止', failed: '未启动，请查看诊断' };
            self.forwardState.textContent = (labels[state.state] || '未启动') + ' · ' + (value.tokenSet ? 'Token 已保存' : '未填写 Token') + (state.error ? ' · ' + state.error : '');
            self.receiveNote.textContent=state.receiveRunning?(state.eventConnected?'事件监听中 · 慢速兜底':'收取运行中 · 事件尚未就绪，慢速兜底')+(state.receiveError?' · '+state.receiveError:''):'自动收取未开启；手动读取仍可用。转发开启时共用同一收取任务。';
        });
    },
    control: function(action, enabled) {
        var self = this; this.controlling = true;
        return controlRPC(action, !!enabled).then(unwrap).then(function(value) { self.runtime = value.service; self.boot.input.checked = !!value.service.bootEnabled; return self.update(true); })
            .finally(function() { self.controlling = false; });
    },
    confirm: function(action) {
        var self = this;
        return new Promise(function(resolve, reject) {
            panelModal(action === 'stop' ? '停止网络面板？' : '重启网络面板？', [
                E('p', {}, '小程序连接会暂时中断，路由器拨号与供网不会被本操作修改。'),
                E('div', { 'class': 'np-actions right' }, [
                    E('button', { 'class': 'np-btn outline', click: function() { ui.hideModal(); resolve(); } }, '取消'),
                    E('button', { 'class': 'np-btn danger', click: function() { ui.hideModal(); self.control(action).then(resolve, reject); } }, '确认')])]);
        });
    },
    update: function(force) {
        var self = this;
        if (!force && (this.updating || this.controlling)) return Promise.resolve();
        this.updating = true;var token={},modem=this.selected;this.updateToken=token;
        var request = force || Date.now() - this.lastRuntime > 6000 ? optional(serviceRPC()).then(function(value) {
            if(self.updateToken!==token)return;
            if(value.failure){self.runtime=Object.assign({},self.runtime,{failure:value.failure,controlReady:false});}
            else {self.runtime=value;self.lastRuntime=Date.now();}
        }) : Promise.resolve();
        return request.then(function() { if(self.updateToken!==token)return null;return self.runtime.running ? optional(statusRPC(modem)) : self.runtime.failure?{failure:self.runtime.failure}:{ modems: [], radio: {} }; })
            .then(function(value) {
                if(self.updateToken!==token||!value)return;
                if(value.failure)value=retainStatus(self.lastGoodStatus,value,modem,self.lastGoodStatusAt,Date.now());
                else {self.lastGoodStatus=value;self.lastGoodStatusAt=Date.now();}
                self.draw(value);
                self.readRecentSms().catch(function(){});
                if (force && self.runtime.running) return optional(settingsRPC()).then(function(settings) {
                    if (!settings.failure) { self.interval.value = settings.interval; self.unsafe.input.checked = !!settings.allowUnsafeAt; }
                });
            }).finally(function() { if(self.updateToken===token)self.updating = false; });
    },
    smsBlocked: function(capability) { return this.readonly || !this.runtime.running || !!this.runtime.failure || !!this.snapshot?.failure || !(this.capabilities || {})[capability]; },
    saveSmsReceiving: function(enabled) {
        var self=this,modem=this.selected;
        if(this.smsBlocked('smsReceivingConfigure'))return Promise.reject(new Error('当前模组未开放此短信配置协议'));
        this.imsBusy=true;
        return task(smsSettingsPrepareRPC(modem,enabled)).then(function(prepared){
            return new Promise(function(resolve,reject){panelModal('保存短信接收配置？',[
                E('p',{},prepared.warning),E('pre',{},prepared.command),E('div',{'class':'np-actions right'},[
                    E('button',{'class':'np-btn outline',click:function(){ui.hideModal();resolve(null);}},'取消'),
                    E('button',{'class':'np-btn danger',click:function(){ui.hideModal();if(self.selected!==modem||self.smsBlocked('smsReceivingConfigure')){reject(new Error('模组或权限已改变'));return;}task(smsSettingsExecuteRPC(modem,enabled,prepared.confirmation)).then(resolve,reject);}},'理解风险并保存')])
            ]);});
        }).then(function(value){
            if(value&&self.selected===modem){self.imsOverride={modem:modem,at:Date.now(),state:value.settings};ui.addNotification(null,E('p',{},'配置已读回；不代表已经生效或短信送达。不自动重连供网。'));}
        }).finally(function(){self.imsBusy=false;self.draw(self.snapshot);});
    },
    draw: function(status) {
        this.snapshot = status;
        this.native = status.backend === 'rust' || this.runtime.backend === 'rust';
        var capabilities = status.capabilities || {};
        this.capabilities = capabilities;
        this.tabs.children[6].hidden = this.tabs.children[7].hidden = false;
        if (this.tabs && this.native) {
            [[2,capabilities.at],[4,capabilities.wifi || capabilities.deviceList],[5,capabilities.lockBand || capabilities.lockCell || (status.radio?.lockPresets?.NR||[]).length || (status.radio?.lockPresets?.LTE||[]).length]].forEach(function(item) {
                this.tabs.children[item[0]].hidden = !item[1];
                if (!item[1]) this.pages[item[0]].hidden = true;
            }, this);
        }
        var rows = status.modems || [], radio = status.radio || {}, serving = radio.serving || [];
        var primary = serving.find(function(item) { return item.rat === 'NR'; }) || serving[0] || {};
        this.selected = status.selectedModem || '';
        if(this.recentSms.modem!==this.selected){
            this.smsGeneration={};this.recentSmsPending=null;this.imsOverride=null;
            this.recentSms={modem:this.selected,loaded:false,messages:[]};this.recentSmsLast=0;this.smsLoadedModem=null;
            this.smsRows.textContent='尚未查询当前模组的短信。';
        }
        if(this.imsToggle){
            var ims=radio.smsReceiving||{};
            if(this.imsOverride&&this.imsOverride.modem===this.selected&&Date.now()-this.imsOverride.at<90000&&!(ims.observedAtMs>this.imsOverride.at))ims=this.imsOverride.state;
            var imsKnown=!radio.stale&&!ims.stale&&typeof ims.enabled==='boolean';
            this.imsKnown=imsKnown;
            if(!this.imsToggle.busy)this.imsToggle.input.checked=imsKnown&&ims.enabled;
            this.imsToggle.input.disabled=!!this.imsBusy||this.smsBlocked('smsReceivingConfigure')||!imsKnown;
            this.imsNote.textContent=capabilities.smsReceivingConfigure?(imsKnown?(ims.enabled?'IMS 配置已开启':'IMS 配置已关闭')+' · 不保证送达；保存不自动重连，需先允许配置型操作。':'等待后台读取此模组的 IMS 状态，未读取不当作关闭。'):'此模组未声明需要此 IMS 开关；短信按实际协议读取，不发送通用开启命令。';
        }
        this.lockSnapshot = radio;
        if (this.refreshLock) this.refreshLock(false);
        if(this.lockSaveButton&&this.lockNeedsRefresh)this.lockSaveButton.disabled=true;
        if (this.lockResultMessage && this.lastLockReceipt && !this.lockNeedsRefresh) this.lockResultMessage.textContent=window.WLMBUIData.lockFeedback([this.lastLockReceipt],status.radio,this.selected);
        var choices = JSON.stringify(rows.map(function(item) { return [item.id, item.model, item.port]; }));
        if (this.choices !== choices) {
            this.choices = choices;
            var options = rows.length ? rows.map(function(item) { return E('option', { value: item.id }, item.model + ' · ' + item.port); }) : [E('option', { value: '' }, '未发现可查询模组')];
            this.selector.replaceChildren.apply(this.selector, options);
        }
        this.selector.value = this.selected;
        var running = !!this.runtime.running;
        this.state.textContent = this.runtime.failure ? '状态查询失败' : running ? this.runtime.controlReady ? '运行中' : '启动中 / 接口未就绪' : '已停止';
        this.dot.classList.toggle('green', running && !!this.runtime.controlReady);
        this.count.textContent = String(rows.length); this.time.textContent = radio.observedAtMs ? new Date(radio.observedAtMs).toLocaleTimeString() : '—';
        this.startButton.disabled = !!this.startButton.busy || this.readonly || running;
        this.stopButton.disabled = !!this.stopButton.busy || this.readonly || !running;
        this.restartButton.disabled = !!this.restartButton.busy || this.readonly || !running;
        if (!this.controlling) this.boot.input.checked = !!this.runtime.bootEnabled;
        this.boot.input.disabled = !!this.boot.busy || this.readonly || !!this.runtime.failure;
        this.saveButton.disabled = !!this.saveButton.busy || this.readonly || !running || capabilities.configure === false;
        [this.smsReadButton,this.smsSendButton,this.forwardSaveButton,this.forwardClearButton].forEach(function(node) { node.disabled = !!node.busy || node.isBlocked(); });
        this.forwardEnabled.input.disabled = this.forwardInterval.disabled = this.forwardToken.disabled = this.smsBlocked('smsForward') || !!this.forwardSaveButton.busy || !!this.forwardClearButton.busy;
        this.receiveEnabled.input.disabled=!!this.receiveEnabled.busy||this.smsBlocked('sms');
        this.forwardUnavailable.textContent = !capabilities.smsForward ? '当前服务未开放转发。请检查短信通道与 HTTPS 构建；关闭时不会创建转发线程或轮询。' : status.smsMode === 'compat-read-only' ? '兼容通道仅读取/转发，不更改短信模式或存储区，不支持发送/删除。' : '';
        this.forwardUnavailable.hidden = !this.forwardUnavailable.textContent;
        if(!capabilities.sms)this.smsRows.textContent='短信通道暂未就绪。请检查独立端点或现有兼容通道；不会抢占串口。';
        if(capabilities.sms && status.smsMode === 'compat-read-only' && this.smsLoadedModem !== this.selected)this.smsRows.textContent='当前可尝试读取短信；此兼容通道仅读取 / 转发，发送和删除未开放。若读取失败，会显示固件实际拒绝的原因，不将整个短信功能标为不支持。';
        this.interval.disabled = this.unsafe.input.disabled = capabilities.configure === false || this.readonly;
        this.controlHint.textContent = this.runtime.failure || (running ? '页面共用后台快照，不重复查询模组。' : '服务已停止，可在上方重新启动。WiFi 和拨号继续由原系统管理。');
        var modem = rows.find(function(item) { return item.id === status.selectedModem; });
        var system = status.system || {}, memory = system.memory || {}, rates = system.network && system.network.rates || {};
        var metrics = ['rsrp', 'rsrq', 'sinr', 'rssi'].filter(function(key) { return key !== 'rssi' || primary[key] != null; });
        var signal = card(system.board && (system.board.model || system.board.board_name) || 'OpenWrt 设备', [
            E('div', { 'class': 'np-metrics' }, metrics.map(function(key) { return E('div', { 'class': 'np-metric' }, [E('span', { 'class': 'np-hint' }, key.toUpperCase()), E('strong', { 'class': 'np-value' }, text(primary[key])), E('small', {}, key === 'rsrp' || key === 'rssi' ? ' dBm' : ' dB')]); })),
            E('p', { 'class': 'np-desc' }, text(primary.networkMode) + ' · ' + band(primary) + ' · PCI ' + text(primary.pci) + ' · ARFCN ' + text(primary.channel)),
            !modem ? E('div', { 'class': 'np-notice' }, running ? '未发现可查询模组。按教程检查独立 AT 端点，不需要重新拨号。' : '启动服务后自动读取设备信息。') : modem.transport === 'dedicated' ? (modem.channelReady ? E('span') : E('div', { 'class': 'np-notice' }, '独立 AT 端点未就绪：' + text(modem.channelState) + '。不会抢占串口或改拨号。')) : !modem.shared ? E('div', { 'class': 'np-notice' }, '旧版通道未开启。本插件不会抢占串口或改拨号。') : E('span'),
            radio.pending ? E('p', { 'class': 'np-hint np-loading-status', role:'status' }, [E('span', { 'class':'np-spinner', 'aria-hidden':'true' }), '后台正在更新，先展示已读数据。']) : radio.stale ? E('p', { 'class':'np-hint' }, '数据已过期，请检查模组端点。') : E('span'),
            status.failure ? E('p', { 'class': 'np-notice' }, status.failure) : E('span'),
            E('span')]);
        var host = card('设备与实时速率', [
            row('蜂窝模组', '模组型号不是路由器型号', modem ? text(modem.model) : '—'),
            row('CPU 占用', '真实采样差值；首次采样暂不显示', text(system.stale ? null : system.cpuUsage, '%')),
            row('内存', '按可用内存计算', text(system.stale ? null : memory.usedPercent, '%') + ' · ' + mb(memory.usedBytes) + ' / ' + mb(memory.totalBytes)),
            row('设备温度', '系统 CPU 温度传感器；不使用网口温度冒充', text(system.stale ? null : (system.temperature || {}).celsius, '°C')),
            row('5G 模组温度', '模组实际报告；未支持时不估算', text(radio.stale || radio.temperature?.stale ? null : (radio.temperature || {}).celsius, '°C')),
            row('运行时间', '', uptime(system.uptimeSeconds)),
            row('下行 / 上行', system.network ? '接口 ' + system.network.interface + ' · 不重复累计桥接接口' : '未确定上网接口时不猜测速率', speed(system.stale ? null : rates.downloadBytesPerSecond) + ' / ' + speed(system.stale ? null : rates.uploadBytesPerSecond))]);
        var traffic = system.stale ? {} : system.traffic || {}, budget = system.trafficLimit || {}, self = this;
        var trafficCard = card('流量统计', [
            row('今日 / 本周期', '上下行合计 · 每月 ' + (budget.startDay || 1) + ' 日起算', mb(traffic.todayBytes) + ' / ' + mb(traffic.monthBytes)),
            row('月流量限额', budget.applyError ? '停网状态未确认，请查看插件日志/系统防火墙' : budget.cutoffActive ? '达到限额，已暂停上网转发，LAN 管理不受影响' : budget.warning ? '已达到提醒比例' : '不补造启用前流量', budget.supported ? button(budget.enabled ? mb(budget.limitBytes) + ' · 编辑' : '设置限额', function() { self.editBudget(budget); }, 'outline') : '等待采样'),
            E('p', { 'class': 'np-hint' }, '从启用插件起统计，不补造之前的流量，不等同运营商账单。每 15 分钟保存一次；断电可能丢失最近未保存的部分。'),
            traffic.persistenceOk === false ? E('p', { 'class': 'np-notice' }, '历史流量暂未保存，请检查插件存储目录。') : E('span') ]);
        this.overview.replaceChildren(signal, host.cloneNode ? host.cloneNode(true) : host);
        this.flow.replaceChildren(host, trafficCard);
        var lan = system.lan || {}, lanStale = !!system.stale || !!lan.stale;
        var wifiControls = lan.controls || {};
        var wifi = lanStale ? (wifiControls.profiles || []).map(function(item) { return Object.assign({}, item, { enabled: undefined }); }) : lan.wireless || [];
        if(!lanStale&&!wifiControls.stale)wifi=wifi.concat((wifiControls.profiles||[]).filter(function(profile){return !wifi.some(function(item){return item.id===profile.id;});}).map(function(profile){return Object.assign({},profile,{enabled:undefined});}));
        var devices = lanStale ? [] : lan.devices || [];
        this.networks.replaceChildren(card('无线网络', wifi.map(function(item) {
            var ht=String(item.channelWidth||''),generation={HT:'WiFi 4',VHT:'WiFi 5',HE:'WiFi 6',EHT:'WiFi 7'}[ht.replace(/\d+$/,'')];
            var wirelessText=generation?generation+' · '+ht.replace(/^[A-Z]+/,'')+' MHz':text(item.channelWidth);
            return row((item.band === '2g' ? '2.4 GHz' : item.band === '5g' ? '5 GHz' : item.band === '6g' ? '6 GHz' : 'WiFi') + ' · ' + item.ssid,
                (lanStale||typeof item.enabled!=='boolean'?'运行状态未确认':item.enabled?'实际运行中':'当前未运行')+' · 信道 ' + text(item.channel) + ' · ' + wirelessText, (wifiControls.profiles || []).some(function(row) { return row.id === item.id; }) && !wifiControls.stale ? button('编辑', function() { self.editWifi(Object.assign({},item,(wifiControls.profiles || []).find(function(row){return row.id===item.id;}),{runtimeEnabled:!lanStale?item.enabled:undefined})); }, 'outline') : lanStale||typeof item.enabled!=='boolean' ? '运行状态待更新' : item.enabled ? '已开启 · 只读' : '已关闭 · 只读');
        }).concat([wifiControls.bandUnifySupported && !wifiControls.stale ? row('双频合一', wifiControls.bandUnifyStateKnown===false?'状态尚未确认，请刷新后重试':'2.4G / 5G 同名同密码，不等同 MLO 或强制漫游', wifiControls.bandUnifyStateKnown===false?E('span',{'class':'np-hint'},'未确认'):button(wifiControls.bandUnifyEnabled ? '关闭合一' : '开启合一', function() { return self.hostAction({kind:'wifi_unify',enabled:!wifiControls.bandUnifyEnabled}); }, 'outline')) : E('span'), E('p', { 'class': 'np-hint' }, lan.stale ? '无线运行/设备快照已过期；仍可编辑经过独立核验的 AP 配置。' : '编辑后系统读取新无线配置；多 AP、访客、企业和 MLO 配置不冒充可编辑。') ])),
        card('已连接设备 · ' + (lanStale ? '待更新' : lan.deviceListSupported ? devices.length : '未读取'), [E('div', { 'class': 'np-scroll' }, E('table', { 'class': 'np-table' }, [
            E('thead', {}, E('tr', {}, ['设备', 'IPv4 / MAC', '连接', '下行 / 上行', '上网控制'].map(function(label) { return E('th', {}, label); }))),
            E('tbody', {}, devices.length ? devices.map(function(item) { return E('tr', {}, [
                E('td', {}, item.name), E('td', {}, [E('div', {}, text(item.ip)), E('small', {}, item.mac)]), E('td', {}, item.type),
                E('td', {}, [E('div',{},speed(item.rates && item.rates.downloadBytesPerSecond) + ' / ' + speed(item.rates && item.rates.uploadBytesPerSecond)),item.rateScope==='port'?E('small',{'class':'np-hint'},'网口 '+text(item.rateInterface)+' · 端口统计'):E('span')]),
                E('td', {}, lan.deviceBlockSupported && !lanStale ? button((lan.blocked || []).includes(item.mac) ? '解除限制' : '限制上网', function() { return self.hostAction({kind:'device_block',mac:item.mac,blocked:!(lan.blocked || []).includes(item.mac)}); }, 'outline') : '当前系统未开放')
            ]); }) : [E('tr', {}, E('td', { colspan:5 }, lanStale ? '设备快照已过期，等待后台更新' : lan.deviceListSupported ? lan.complete === false ? '设备列表尚未完整读取' : '暂未发现在线设备' : '当前系统未提供可读取的设备列表'))]) ])),
            !lanStale && lan.complete === false && devices.length ? E('p', {'class':'np-notice'}, '部分无线接口未完成读取，此列表可能不完整。') : E('span'),
            (lan.blocked || []).length ? E('div', { 'class': 'np-actions' }, (lan.blocked || []).map(function(mac) { return button('解除 ' + mac, function() {return self.hostAction({kind:'device_block',mac:mac,blocked:false});}, 'outline'); })) : E('span'),
            lan.blockApplyError && (lan.blocked || []).length ? E('p', {'class':'np-notice'}, '限制配置已保存，但实际生效尚未确认；可以先解除限制。') : E('span'),
            E('p', { 'class': 'np-hint' }, '速率来自实际字节差分，不使用协商速率。有线为网口统计（含广播与局域网流量）；端口共享时不分摊给设备，首轮或计数重置显示 —。') ]),
        !lanStale&&(lan.ports||[]).length?card('网口实时速率',[
            E('div',{'class':'np-port-grid'},lan.ports.map(function(port){return E('div',{'class':'np-port-stat'},[E('strong',{},text(port.interface)),E('div',{},'↓ '+speed(port.rates&&port.rates.downloadBytesPerSecond)+' · ↑ '+speed(port.rates&&port.rates.uploadBytesPerSecond)),E('span',{'class':'np-hint'},port.peerCount>1?'共享端口 · 不冒充单设备速率':'端口统计 · 非单设备精确计量')]);})),
            E('p',{'class':'np-hint'},'复用后台约12–18秒采样窗口，不抓包、不改防火墙或加速配置。')]):E('span'));
        this.cells.replaceChildren(this.cellTable('当前服务小区 · 真实测量', serving),
            this.cellTable('载波聚合 · ' + (radio.carriers || []).length + ' 条载波', radio.carriers || []),
            this.cellTable('邻区 · ' + (radio.neighbors || []).length, radio.neighbors || []),
            E('p', { 'class': 'np-hint' }, 'SSB 服务频点与载波中心频点不是同一概念。固件没有报告载波角色或子信号时，保留频点和带宽，不猜主/子载波关系。'));
        if (this.webuiBridge && this.webuiBridge.redraw) {
            this.webuiBridge.snapshot=status;
            this.webuiBridge.redraw();
        }
    },
    hostAction: function(operation) {
        var self = this;
        if (this.readonly || !this.runtime.running || this.runtime.failure || this.snapshot.failure) return Promise.reject(new Error('需要状态已确认的运行服务和写权限'));
        var encoded = JSON.stringify(operation);
        return task(hostPrepareRPC(encoded)).then(function(prepared) {
            return new Promise(function(resolve, reject) {
                panelModal('保存设备设置？', [E('p', {}, prepared.warning), E('div', { 'class': 'np-actions right' }, [
                    E('button', { 'class': 'np-btn outline', click: function() { ui.hideModal(); resolve(); } }, '取消'),
                    E('button', { 'class': 'np-btn primary', click: function() { ui.hideModal(); task(hostExecuteRPC(encoded, prepared.revision, prepared.confirmation)).then(function() { return self.update(true); }).then(resolve, reject); } }, '确认保存')])]);
            });
        });
    },
    editWifi: function(item) {
        var self = this, ssid = E('input', {'class':'np-input',value:item.ssid,'aria-label':'WiFi 名称'}), key = E('input', {'class':'np-input',type:'password',placeholder:'留空保持原密码',autocomplete:'new-password','aria-label':'WiFi 新密码'});
        var enabledKnown=typeof item.enabled==='boolean';
        var enabled = E('input', {type:'checkbox','aria-label':'启用 WiFi',checked:enabledKnown&&item.enabled?'':null}); enabled.checked = enabledKnown&&item.enabled;
        enabled.indeterminate=!enabledKnown;enabled.disabled=!enabledKnown;
        var enabledSwitch=E('label',{'class':'np-toggle'},[enabled,E('span',{'class':'np-track'})]);
        var stateNote=!enabledKnown?'配置状态未知，保持原设置':(item.enabled?'配置已启用':'配置已关闭')+' · '+(item.runtimeEnabled===true?'实际运行中':item.runtimeEnabled===false?'当前未运行':'运行状态待确认');
        function choice(label,options,current) {
            var value=current==null?'':String(current), values=(options||[]).slice();
            if(!values.some(function(o){return String(o.value)===value;}))values.unshift({value:value,label:'当前设置（保持不变）'});
            var field=E('select',{'class':'np-input','aria-label':label},values.map(function(o){return E('option',{value:o.value},o.label);}));field.value=value;return field;
        }
        var channel=choice('WiFi 信道',item.channelOptions,item.channel), mode=choice('无线代际',item.modeOptions,item.mode), security=choice('加密方式',item.securityOptions,item.authMode);
        function widths() { return (item.bandwidthOptions||[]).filter(function(o){return !o.mode||o.mode===mode.value;}); }
        var width=choice('WiFi 频宽',widths(),item.bandwidth||item.channelWidth);
        mode.addEventListener('change',function(){
            var options=widths(), old=String(width.value).replace(/^[A-Z]+/,'');
            var selected=options.find(function(o){return String(o.value).replace(/^[A-Z]+/,'')===old;})||options[0];
            width.replaceChildren.apply(width,options.map(function(o){return E('option',{value:o.value},o.label);}));
            width.value=selected?selected.value:'';
        });
        panelModal('编辑无线网络', [row('WiFi 名称', '最多 32 字节', ssid), row('新密码', '8–63 位英文字符；不显示已有密码', key), row('启用 WiFi', stateNote, enabledSwitch),
            item.modeSupported?row('无线代际','',mode):E('span'),item.securitySupported?row('加密方式','WiFi 7 请使用 WPA3 或 WPA2 / WPA3',security):E('span'),
            item.channelSupported?row('信道','',channel):E('span'),item.bandwidthSupported?row('带宽','',width):E('span'),E('p',{'class':'np-hint'},'保存后 WiFi 可能短暂断开，请准备网线或恢复方式。'),
            E('div', {'class':'np-actions right'}, [E('button', {'class':'np-btn outline',click:function(){ui.hideModal();key.value='';}}, '取消'), button('下一步', function() {
                var changes = {}; if (ssid.value !== item.ssid) changes.ssid = ssid.value; if (key.value) changes.password = key.value; if (enabledKnown&&enabled.checked !== item.enabled) changes.enabled = enabled.checked;
                if(item.channelSupported&&channel.value!==String(item.channel==null?'':item.channel))changes.channel=channel.value;
                if(item.modeSupported&&mode.value!==String(item.mode||''))changes.mode=mode.value;
                if(item.securitySupported&&security.value!==String(item.authMode||''))changes.authMode=security.value;
                if(item.bandwidthSupported&&width.value!==String(item.bandwidth||item.channelWidth||''))changes.bandwidth=width.value;
                if (!Object.keys(changes).length) throw new Error('没有需要保存的修改'); key.value = ''; ui.hideModal(); return self.hostAction({kind:'wifi_edit',band:item.band,changes:changes});
            }, 'primary')])]);
    },
    editBudget: function(budget) {
        var self = this;
        function field(label, value) { return E('input', {'class':'np-input',type:'number',value:String(value),'aria-label':label}); }
        var enabled = E('input', {type:'checkbox','aria-label':'启用月限额'}); enabled.checked = !!budget.enabled;
        var amount = field('月限额 GB', (budget.limitBytes || 0) / 1073741824), start = field('统计起始日', budget.startDay || 1), threshold = field('提醒比例', budget.threshold || 90);
        var cutoff = E('input', {type:'checkbox','aria-label':'达到限额停止转发'}); cutoff.checked = !!budget.cutoffEnabled; cutoff.disabled = !budget.cutoffSupported;
        panelModal('流量统计与限额', [row('启用月限额', '', enabled), row('月限额 GB', '按插件记录上下行合计', amount), row('统计起始日', '1–28', start), row('提醒比例', '1–100%', threshold),
            row('达到限额停止转发', budget.cutoffSupported ? '保留 LAN 管理；插件停止后恢复转发' : '当前系统仅支持统计提醒，停网未开放', cutoff),
            E('div', {'class':'np-actions right'}, [E('button', {'class':'np-btn outline',click:function(){ui.hideModal();}}, '取消'), button('下一步', function() {
                var operation = {kind:'traffic_limit',enabled:enabled.checked,limitBytes:enabled.checked ? Math.round(Number(amount.value) * 1073741824) : 0,startDay:Number(start.value),threshold:Number(threshold.value),cutoffEnabled:enabled.checked && !cutoff.disabled && cutoff.checked};
                ui.hideModal(); return self.hostAction(operation);
            }, 'primary')])]);
    },
    cellTable: function(title, rows) {
        var hasSignal = rows.some(function(item) { return item.rsrp != null || item.rsrq != null || item.sinr != null; });
        var hasPci = rows.some(function(item) { return item.pci != null; });
        var columns = ['类型', '频段', '频点'];
        if (hasPci) columns.push('PCI');
        if (hasSignal) columns.push('RSRP', 'RSRQ', 'SINR');
        columns.push('下行 / 上行');
        return card(title, [E('div', { 'class': 'np-scroll' }, E('table', { 'class': 'np-table' }, [
            E('thead', {}, E('tr', {}, columns.map(function(value) { return E('th', { scope: 'col' }, value); }))),
            E('tbody', {}, rows.length ? rows.map(function(item) {
                var role = { pcc: '主载波', scc: '子载波', neighbor: '邻区', unknown: '角色未报告' }[item.role] || '';
                var values = [item.rat + ' ' + role + (item.active === false ? '（未激活）' : '') + (item.stale ? '（旧数据）' : ''), band(item), text(item.channel) + (item.channelType === 'center' ? '（中心）' : item.channelType === 'ssb' ? '（SSB）' : '')];
                if (hasPci) values.push(text(item.pci));
                if (hasSignal) values.push(text(item.rsrp, ' dBm'), text(item.rsrq, ' dB'), text(item.sinr, ' dB'));
                values.push(text(item.dlBandwidthMHz, ' MHz') + ' / ' + text(item.ulBandwidthMHz, ' MHz'));
                return E('tr', {}, values.map(function(value) { return E('td', {}, value); }));
            }) : [E('tr', {}, E('td', { colspan: columns.length, 'class': 'np-hint' }, '暂未读到设备数据'))])])),
            rows.length && !hasSignal ? E('p', { 'class': 'np-hint' }, '频点与带宽已读取；此列表未报告 PCI/信号，主小区的真实信号见上方。') : E('span')]);
    },
    consolePage: function() {
        var self = this, input = E('input', { 'class': 'np-input np-entry', placeholder: '例如 AT^MONSC', maxlength: 192, 'aria-label': 'AT 指令' });
        var output = E('textarea', { 'class': 'np-console', readonly: true, 'aria-label': '指令响应', placeholder: '响应仅保留在当前页面，不写入系统日志' });
        var send = button('发送指令', function() {
            var command = input.value.trim(), modem = self.selected;
            if (!self.runtime.running || self.runtime.failure || self.snapshot.failure || !modem) throw new Error('请先确认服务状态并选择模组');
            return prepareRPC(modem, command).then(unwrap).then(function(prepared) {
                if (prepared.readOnly) return task(executeRPC(modem, command, prepared.confirmation));
                return new Promise(function(resolve, reject) {
                    panelModal('确认配置型 AT 指令', [E('p', {}, prepared.warning), E('pre', {}, command), E('div', { 'class': 'np-actions right' }, [
                        E('button', { 'class': 'np-btn outline', click: function() { ui.hideModal(); resolve(null); } }, '取消'),
                        E('button', { 'class': 'np-btn danger', click: function() { ui.hideModal(); task(executeRPC(modem, command, prepared.confirmation)).then(resolve, reject); } }, '理解风险并执行')])]);
                });
            }).then(function(result) { if (result) output.value = result.response; });
        }, 'primary');
        send.disabled = this.readonly;
        return card('AT 控制台', [E('p', { 'class': 'np-desc' }, '默认只读，共用现有模组队列。配置型指令需开启权限并逐次确认。'), input, send, output, link('查看 AT 安全说明', guide('#at'))]);
    },
    lockPage: function() {
        var self = this;
        var rat = E('select', { 'class': 'np-input', 'aria-label': '锁定制式' }, [E('option', { value: 'NR' }, '5G NR'), E('option', { value: 'LTE' }, '4G LTE')]);
        var mode = E('select', { 'class': 'np-input', 'aria-label': '锁定类型' }, [E('option', { value: 'band' }, '选频段'), E('option', { value: 'cell' }, '选小区'), E('option', { value: 'auto' }, '恢复自动')]);
        rat.value = 'NR'; mode.value = 'band'; var lastLockType = 'band';
        var bandGrid = E('div', { 'class': 'np-band-grid' });
        var cellBand = E('input', { 'class': 'np-input', 'aria-label': '小区频段', placeholder:'例如 n78 或 B3' });
        var cellChoice = E('select', { 'class': 'np-input', 'aria-label': '已读小区' });
        var channel = E('input', { 'class': 'np-input', type: 'number', min: 0, 'aria-label': '服务频点', placeholder: 'ARFCN / EARFCN' });
        var pci = E('input', { 'class': 'np-input', type: 'number', min: 0, 'aria-label': '小区 PCI', placeholder: 'PCI' });
        var scs = E('select', { 'class': 'np-input', 'aria-label': 'NR SCS' }, [E('option', { value: '' }, '选择已确认 SCS')].concat([15,30,60,120,240].map(function(v,i){return E('option',{value:String(i)},v+' kHz');})));
        scs.value = '';
        var bandRequired = row('频段', '仅此协议需要，唯一候选会自动填入', cellBand), scsRequired = row('SCS', '仅在模组要求且未报告时补充', scs);
        var selectionInfo = E('p', { 'class':'np-lock-selection', role:'status' });
        bandRequired.hidden=scsRequired.hidden=selectionInfo.hidden=true;
        var drafts = [], bands = [], choices = [], chosen = null, context = '', cellRows = E('div'), hint = E('p', { 'class': 'np-hint' });
        function paintCells() {
            cellRows.replaceChildren.apply(cellRows, drafts.map(function(item,index) {
                return row((item.band ? (rat.value==='NR'?'n':'B')+item.band : (rat.value==='NR'?'5G NR':'4G LTE')) + ' · PCI ' + item.pci, '频点 ' + item.arfcn, button('移除',function(){ drafts.splice(index,1); paintCells(); return Promise.resolve(); },'outline'));
            }));
        }
        var add = button('加入待保存', function() {
            var item = draftItem();
            var maxCells = self.lockSnapshot?.lockPresets?.maxCells || 4;
            if (maxCells === 1) drafts=[];
            if (drafts.length >= maxCells) throw new Error('此协议最多保存 ' + maxCells + ' 个小区');
            if (drafts.some(function(v){return v.arfcn===item.arfcn&&v.pci===item.pci;})) throw new Error('此小区已加入');
            drafts.push(item); paintCells(); return Promise.resolve();
        }, 'outline');
        var advanced = E('details', { 'class': 'np-lock-advanced' }, [E('summary', {}, '补充参数'), E('p',{'class':'np-hint'},'PCI 与服务频点始终可手动填写。频段唯一时自动补齐；SCS 不使用猜测值，只有模组协议需要时才显示。')]);
        var cellForm = E('div', {}, [row('快捷选择（可选）', '从当前小区或邻区填入，也可以直接手动输入', cellChoice), E('div',{'class':'np-lock-basic'},[E('label',{'class':'np-lock-field'},[E('span',{},'PCI'),pci]),E('label',{'class':'np-lock-field'},[E('span',{},'ARFCN / EARFCN'),channel])]), selectionInfo, bandRequired, scsRequired, E('div',{'class':'np-actions'},[add]), cellRows, advanced]);
        function showMode() { bandGrid.hidden = mode.value !== 'band'; cellForm.hidden = mode.value !== 'cell'; scs.hidden = rat.value !== 'NR'||self.lockSnapshot?.lockPresets?.scsRequired===false; }
        mode.addEventListener('change', function(){if(mode.value!=='auto')lastLockType=mode.value; self.refreshLock(true);showMode();});
        function resolveDraft() {
            var presets=self.lockSnapshot?.lockPresets||{},control=self.lockSnapshot?.controls||{};
            var fields=control.cellFields?.[rat.value]||presets.cellFields?.[rat.value]||{scsRequired:presets.scsRequired!==false};
            var result=window.WLMBUIData.resolveCellDraft({rat:rat.value,pci:pci.value,arfcn:channel.value,band:cellBand.value,scsType:scs.value},{fields:fields,ranges:presets.cellRanges?.[rat.value],choices:choices});
            if(result.cell?.band)cellBand.value=result.cell.band;
            if(result.cell?.scsType!==undefined)scs.value=String(result.cell.scsType);
            bandRequired.hidden=fields.bandRequired===false||!result.missingBand&&!advanced.open;
            scsRequired.hidden=rat.value!=='NR'||fields.scsRequired===false||!result.missingScs&&!advanced.open;
            return result;
        }
        function draftItem() {
            var result=resolveDraft();
            if(result.error||result.missingBand||result.missingScs)throw new Error(result.error||(result.missingBand?'请补充模组需要的频段':'请补充模组需要的 SCS'));
            var item={arfcn:result.cell.arfcn,pci:result.cell.pci};
            if(result.cell.band)item.band=Number(result.cell.band.slice(1));
            if(result.cell.scsType!==undefined)item.scsType=result.cell.scsType;
            return item;
        }
        function stageSelection() {
            if(!chosen)return;
            var resolved=resolveDraft();
            selectionInfo.hidden=false;selectionInfo.textContent='已填入 PCI '+pci.value+' · 频点 '+channel.value;
            if (!resolved.error&&!resolved.missingBand&&!resolved.missingScs) {
                drafts = self.lockSnapshot?.lockPresets?.maxCells === 1 ? [] : drafts;
                var draft = draftItem();
                var existing=drafts.findIndex(function(row){return row.arfcn===draft.arfcn&&row.pci===draft.pci;});
                if(existing>=0)drafts[existing]=draft;
                else if(drafts.length < (self.lockSnapshot?.lockPresets?.maxCells || 4))drafts.push(draft);
                paintCells(); advanced.open=false;
                hint.textContent='已加入待保存；确认后才会提交，不会自动执行锁定。';
            } else { hint.textContent='仅补充上方缺少的参数即可，其他已读值无需重填。不会猜测 SCS。'; }
            advanced.open=false;
        }
        cellChoice.addEventListener('change', function() {
            if (cellChoice.value === '') {chosen=null;bandRequired.hidden=scsRequired.hidden=selectionInfo.hidden=true;return;}
            var item = choices[Number(cellChoice.value)];
            if (!item) return;
            if(self.lockSnapshot?.lockPresets?.maxCells===1){drafts=[];paintCells();}
            chosen=item;channel.value = String(item.channel); pci.value = String(item.pci);
            var match = /^(?:n|B)(\d+)$/i.exec(item.band || '');
            cellBand.value = match ? (rat.value==='NR'?'n':'B')+match[1] : '';
            scs.value = Number.isInteger(item.scsType) && item.scsType >= 0 && item.scsType <= 4 ? String(item.scsType) : '';
            stageSelection();
        });
        [channel,pci].forEach(function(node){node.addEventListener('input',function(){chosen=null;cellChoice.value='';cellBand.value=scs.value='';selectionInfo.hidden=true;resolveDraft();});});
        cellBand.addEventListener('input',function(){resolveDraft();});
        advanced.addEventListener('toggle',function(){resolveDraft();});
        scs.addEventListener('change',function(){if(chosen)stageSelection();else resolveDraft();});
        this.fillLockCandidate=function(item){
            if (!item || item.stale || (self.lockSnapshot || {}).stale || self.readonly || !self.runtime.running || !self.capabilities.lockCell) throw new Error('当前小区不可用于锁定');
            rat.value=item.rat; mode.value='cell'; lastLockType='cell'; self.refreshLock(true); showMode();
            var index=choices.findIndex(function(candidate){return candidate.rat===item.rat&&candidate.channel===item.channel&&candidate.pci===item.pci;});
            if(index<0)throw new Error('小区数据已改变，请重新读取');
            cellChoice.value=String(index); cellChoice.dispatchEvent(new Event('change'));
        };
        this.refreshLock = function(force) {
            var radio = self.lockSnapshot || {}, presets = radio.lockPresets || {}, control = radio.controls || {}, cellGroups = control.groups || {}, group = control.bandGroups || cellGroups;
            var hasNr=(presets.NR||[]).length>0,hasLte=(presets.LTE||[]).length>0;
            rat.children[0].disabled=!hasNr&&hasLte;rat.children[1].disabled=!hasLte&&hasNr;
            if(rat.value==='NR'&&!hasNr&&hasLte)rat.value='LTE';
            else if(rat.value==='LTE'&&!hasLte&&hasNr)rat.value='NR';
            var nrMode = (radio.serving || []).some(function(row){return row.networkMode==='5G NSA';}) && group.NR_NSA ? 'NR_NSA' : 'NR';
            var bandRat = rat.value==='NR' ? nrMode : rat.value;
            var writable=!!(self.capabilities.lockBand||self.capabilities.lockCell);
            if(read&&save){read.disabled=self.readonly||!writable||!!read.busy;save.disabled=self.readonly||!writable||!!save.busy;}
            mode.children[0].hidden=self.capabilities.lockBand===false||self.capabilities.bandLock===false;
            mode.children[1].hidden=self.capabilities.lockCell===false||self.capabilities.cellLock===false;
            if(mode.value==='band'&&mode.children[0].hidden){mode.value='cell';lastLockType='cell';}
            if(mode.value==='cell'&&mode.children[1].hidden&&!mode.children[0].hidden){mode.value='band';lastLockType='band';}
            mode.children[2].textContent=lastLockType==='band'?'恢复自动频段':'解除小区锁定';
            var signature = JSON.stringify([self.selected,rat.value,presets,writable]);
            if (!force && context === signature) return;
            context = signature; drafts = []; chosen=null;bandRequired.hidden=scsRequired.hidden=selectionInfo.hidden=true;advanced.open=false;paintCells();
            var presetKey = mode.value==='band' && rat.value==='NR' && group.NR_NSA ? (nrMode==='NR_NSA' ? 'NRNSA' : 'NRSA') : rat.value;
            bands = Array.isArray(presets[presetKey]) ? presets[presetKey] : [];
            bandGrid.replaceChildren.apply(bandGrid, bands.map(function(v) {
                var input = E('input', { type: 'checkbox', value: String(v), 'aria-label': (rat.value === 'NR' ? 'n' : 'B') + v });
                input.checked = !!group[bandRat] && group[bandRat].mode === 3 && group[bandRat].entries.some(function(e){return e.band === v;});
                return E('label', { 'class': 'np-band-chip' }, [input, E('span', {}, (rat.value === 'NR' ? 'n' : 'B') + v)]);
            }));
            cellBand.value = '';
            choices = (radio.stale ? [] : (radio.serving || []).concat(radio.neighbors || [])).filter(function(v){return v.rat === rat.value && !v.stale && Number.isInteger(v.channel) && Number.isInteger(v.pci) && (v.rat === 'LTE' || v.channelType !== 'center');});
            cellChoice.replaceChildren.apply(cellChoice, [E('option',{value:''},'选择当前小区 / 邻区')].concat(choices.map(function(v,i){return E('option',{value:String(i)},band(v)+' · PCI '+v.pci+' · '+v.channel);})));
            cellChoice.value = ''; channel.value = pci.value = scs.value = '';
            hint.textContent = bands.length ? (writable ? '按模组支持频段选择；5G 自动对应当前 SA / NSA。保存可能短暂影响连接。' : '仅展示型号资料；此模组尚无可靠的锁定协议。') : '尚未读取到可用频段，不能提交。';
            showMode();
        };
        rat.addEventListener('change', function(){self.refreshLock(true);});
        var result = E('textarea', { 'class': 'np-console', readonly: true, 'aria-label': '锁定状态和结果', placeholder: '主动读取保存配置；不会自动重置供网' });
        var resultMessage=E('p', { 'class':'np-lock-message', role:'status' });
        this.lockResultMessage=resultMessage;
        var resultDetails=E('details', { 'class':'np-lock-result' }, [E('summary',{},'诊断详情'),result]);
        var read = button('读取保存配置', function() {
            var modem = self.selected;
            return task(lockStatusRPC(modem, true)).then(function(value) { if (self.selected !== modem) throw new Error('模组已更换，请重新读取'); if((value.errors||[]).length || !Object.keys(value.groups||{}).length && !Object.keys(value.bandGroups||{}).length)throw new Error('配置仍未读全；请稍后刷新，不要重复提交');self.lockNeedsRefresh=false;self.lockSnapshot.controls = value; if(value.presets)self.lockSnapshot.lockPresets=value.presets; result.value = JSON.stringify(value, null, 2);resultMessage.textContent='配置已读取。选择频段或小区后确认保存。'; self.refreshLock(true); });
        }, 'outline');
        var save = button('检查并保存', function() {
            if(self.lockNeedsRefresh)throw new Error('上次结果未完整确认；请先读取保存配置，再重新选择');
            if (self.readonly || !self.runtime.running || self.runtime.failure || self.snapshot.failure || !self.selected || !(self.capabilities.lockBand||self.capabilities.lockCell)) throw new Error('请先确认服务状态并选择支持锁定协议的模组');
            var selectedRat = rat.value, selectedMode = mode.value, modem = self.selected, items = [];
            if (selectedMode === 'band') items = Array.from(bandGrid.children).filter(function(label){return label.children[0].checked;}).map(function(label){return {band:Number(label.children[0].value)};});
            if (selectedMode === 'cell') {
                items = drafts.slice();
                if(pci.value!==''||channel.value!==''){
                    var entry=draftItem();
                    var same=items.findIndex(function(row){return row.arfcn===entry.arfcn&&row.pci===entry.pci;});
                    if(same>=0)items[same]=entry;
                    else items=self.lockSnapshot?.lockPresets?.maxCells===1?[entry]:items.concat([entry]);
                }
            }
            if (selectedMode !== 'auto' && !items.length) throw new Error('请先选择频段或加入小区');
            var bandGroups=(self.lockSnapshot?.controls || {}).bandGroups;
            var bandTarget=selectedMode==='band' || selectedMode==='auto' && lastLockType==='band';
            if(bandTarget && selectedRat==='NR' && bandGroups?.NR_NSA && (self.lockSnapshot.serving || []).some(function(row){return row.networkMode==='5G NSA';})) selectedRat='NR_NSA';
            var operation = JSON.stringify(Object.assign({ rat: selectedRat, mode: selectedMode, entries: items },bandTarget && bandGroups ? {target:'band'} : {}));
            return task(lockPrepareRPC(modem, operation)).then(function(prepared) {
                return new Promise(function(resolve, reject) {
                    panelModal('确认锁定配置？', [E('p', {}, prepared.warning), E('details', {}, [E('summary', {}, '指令详情'), E('pre', {}, prepared.command)]), E('div', { 'class': 'np-actions right' }, [
                        E('button', { 'class': 'np-btn outline', click: function() { ui.hideModal(); resolve(null); } }, '取消'),
                        E('button', { 'class': 'np-btn danger', click: function() { ui.hideModal(); if (self.selected !== modem || self.readonly || !self.runtime.running) { reject(new Error('模组、服务或权限已改变，请重新读取')); return; } task(lockExecuteRPC(modem, operation, prepared.revision, prepared.confirmation)).then(resolve, function(error){self.lockNeedsRefresh=true;resultMessage.textContent='提交结果未完整确认；先读取保存配置，不要重复提交。';reject(error);}); } }, '理解风险并保存')])]);
                });
            }).then(function(value) { if (value) { result.value = JSON.stringify(value, null, 2);self.lockNeedsRefresh=value.savedVerified!==true;self.lastLockReceipt={modem:modem,operation:JSON.parse(operation),receipt:value};resultMessage.textContent=window.WLMBUIData.lockFeedback([self.lastLockReceipt],self.snapshot.radio,modem); } return self.update(true); });
        }, 'primary');
        save.disabled = read.disabled = this.readonly;
        this.lockSaveButton=save;
        return card('频段与小区', [E('div',{'class':'np-lock-layout'},[
            E('p', { 'class': 'np-desc' }, '选择频段或小区，确认后保存。设备已报告的参数自动填写。'),
            E('div', { 'class': 'np-lock-controls' }, [E('label',{'class':'np-lock-field'},[E('span',{},'网络制式'),rat]),E('label',{'class':'np-lock-field'},[E('span',{},'锁定方式'),mode])]),
            E('div',{'class':'np-lock-content'},[hint,bandGrid,cellForm]),
            E('p', { 'class': 'np-notice np-lock-safety' }, '锁定可能短暂断网，请准备恢复方式。不会主动切飞行模式、拨号或重启。'),
            E('div',{'class':'np-lock-footer'},[E('div', { 'class': 'np-actions' }, [read, save]),resultMessage,resultDetails])
        ])]);
    },
    settingsPage: function(settings) {
        var self = this;
        this.interval = E('input', { 'class': 'np-input np-number', type: 'number', min: 2, max: 30, value: settings.interval || 3, 'aria-label': '后台刷新间隔' });
        this.unsafe = toggle('允许配置型操作', !!settings.allowUnsafeAt, function() { return Promise.resolve(); });
        this.imsToggle=toggle('短信接收（IMS 配置）',false,function(enabled){return self.saveSmsReceiving(enabled);});
        this.imsToggle.input.isBlocked=function(){return !!self.imsBusy||!self.imsKnown||self.smsBlocked('smsReceivingConfigure');};
        this.imsNote=E('p',{'class':'np-hint'},'按模组真实协议读取；此开关不控制短信转发。');
        this.interval.disabled = this.unsafe.input.disabled = this.readonly;
        this.saveButton = button('保存设置', function() {
            var interval = Number(self.interval.value);
            if (!Number.isInteger(interval) || interval < 2 || interval > 30) throw new Error('刷新间隔请输入 2–30 的整数');
            return task(saveRPC(interval, self.unsafe.input.checked)).then(function() { ui.addNotification(null, E('p', {}, '网络面板设置已保存'), 'info'); });
        }, 'primary');
        return E('div', {}, [card('插件设置', [
            row('模组通道',settings.connection&&settings.connection.qmodemDetected?'已检测到 QModem':'未检测到 QModem',E('span',{'class':'np-tag'},settings.connection&&settings.connection.mode==='shared'?'共享模式':settings.connection&&settings.connection.mode==='independent'?'独立模式':'等待识别')),
            row('后台刷新间隔', '2–30 秒；邻区独立缓存', this.interval),
            row('允许配置型操作', '默认关闭；锁定与手动配置 AT 仍需逐次确认', this.unsafe),
            row('短信接收（IMS 配置）','仅对已核验需要此协议的模组开放；不是关闭插件短信功能',this.imsToggle),this.imsNote,
            E('div', { 'class': 'np-actions' }, [this.saveButton, button('脱敏诊断', function() { return diagnosticsRPC().then(unwrap).then(function(value) {
                panelModal('网络面板诊断', [E('pre', { 'class': 'np-console' }, JSON.stringify(value, null, 2)), E('button', { 'class': 'np-btn outline', click: ui.hideModal }, '关闭')]);
            }); }, 'outline')])]),
            card('更多设备功能', [
                E('p', { 'class': 'np-desc' }, '功能按设备实际能力开放，修改前请预留恢复方式。'),
                row('信号、载波与邻区', '按模组协议返回真实数据', E('span', { 'class': 'np-tag' }, '已接入')),
                row('CPU、内存与速率', '共享系统采样，不生成虚假月流量', E('span', { 'class': 'np-tag' }, '已接入')),
                row('WiFi 与已连接设备', '名称、密码、开关、双频合一按实际 AP 能力开放；MLO 不冒充已支持', link('打开系统高级 WiFi', L.url('admin/network/wireless'))),
                row('锁频与锁小区', '按已核验模组协议开放保存接口；不自动切飞行模式', link('查看安全说明', guide('#locks'))),
                row('月限额与统计起始日', '按插件累计；可靠 fw4 环境才开放停网', link('查看功能说明', guide('#features'))),
                row('短信与转发', '内置短信与 PushPlus 按独立 AT 端点能力开放；关闭转发后不轮询', link('查看功能说明', guide('#features')))]),
            E('p', { 'class': 'np-hint' }, '仅保存本插件配置，不自动改拨号、网络或其他插件。小程序使用 LAN 地址与 1117 连接。')]);
    }
});
