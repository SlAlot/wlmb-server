// 网络面板自有前端；仅处理展示和已授权RPC调用，不直接操作模组。
(function(global) {
    'use strict';
    var definitions=[{key:0,name:'网络信息',icon:'home',group:0},{key:1,name:'邻区扫描',icon:'neighbors',group:1},{key:4,name:'WiFi 与设备',icon:'wifi',group:2},{key:5,name:'频段与小区',icon:'bands',group:1},{key:6,name:'短信',icon:'sms',group:3},{key:7,name:'短信转发',icon:'sms',group:3},{key:8,name:'设备与流量',icon:'chart',group:4},{key:2,name:'AT 控制台',icon:'terminal',group:4},{key:3,name:'插件设置',icon:'settings',group:4}];
    var groups=[{name:'概览',icon:'home',keys:[0]},{name:'网络',icon:'neighbors',keys:[1,5]},{name:'WiFi',icon:'wifi',keys:[4]},{name:'短信',icon:'sms',keys:[6,7]},{name:'设置',icon:'settings',keys:[3,8,2]}];
    global.WLMBWebUI={version:'1.0.7',mount:function(target,bridge){
        var D=global.WLMBUIData,I=global.WLMBIcons,doc=target.ownerDocument;
        if(!D||D.version!=='1.0.7'||!I||I.version!=='1.0.7')throw new Error('独立前端资源版本不一致');
        var disposed=false,failed=false,current=bridge.active||0,filter='',search='',refreshing=false,reading=false,copying=false,smsStamp='',overviewStamp='',neighborStamp='';
        var panelRoot=target.closest('#wlmb-panel'),theme='auto',collapsed=false,lastGroup={};
        var motion=typeof global.matchMedia==='function'?global.matchMedia('(prefers-reduced-motion: reduce)'):null,animations=new Set(),paintedPage=-1,carrierDetails=new Set();
        function cancelMotion(){animations.forEach(function(animation){animation.cancel();});animations.clear();}
        function enter(node){
            if(disposed||!node||!node.isConnected||node.hidden||motion?.matches||typeof node.animate!=='function')return;
            var animation=node.animate([{opacity:0,transform:'translateY(5px)'},{opacity:1,transform:'translateY(0)'}],{duration:180,easing:'cubic-bezier(.2,.7,.2,1)'});
            animations.add(animation);animation.finished.catch(function(){}).finally(function(){animations.delete(animation);});
        }
        function motionChanged(){if(motion?.matches)cancelMotion();}
        if(motion?.addEventListener)motion.addEventListener('change',motionChanged);
        function n(tag,attrs,children){
            var node=doc.createElement(tag);
            Object.keys(attrs||{}).forEach(function(key){var value=attrs[key];if(value==null)return;if(key==='class')node.className=value;else if(key==='hidden')node.hidden=!!value;else node.setAttribute(key,String(value));});
            (Array.isArray(children)?children:[children]).forEach(function(child){if(child!=null)node.appendChild(child.nodeType?child:doc.createTextNode(String(child)));});return node;
        }
        function glyph(name){
            var svg=doc.createElementNS('http://www.w3.org/2000/svg','svg'),path=doc.createElementNS('http://www.w3.org/2000/svg','path');
            svg.setAttribute('class','np-ui-icon');svg.setAttribute('viewBox',I.viewBox);svg.setAttribute('aria-hidden','true');svg.setAttribute('fill','currentColor');
            path.setAttribute('d',I.paths[name]||I.paths.info);path.setAttribute('transform',I.transform);svg.appendChild(path);return svg;
        }
        function sidebarIcon(){
            var svg=doc.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('class','np-sidebar-icon');svg.setAttribute('viewBox','0 0 24 24');svg.setAttribute('aria-hidden','true');svg.setAttribute('fill','none');svg.setAttribute('stroke','currentColor');svg.setAttribute('stroke-width','2');svg.setAttribute('stroke-linecap','round');svg.setAttribute('stroke-linejoin','round');
            [['rect',{x:3,y:3,width:18,height:18,rx:2}],['path',{d:'M9 3v18'}],['path',{d:'m16 15-3-3 3-3','class':'np-sidebar-direction'}]].forEach(function(item){var node=doc.createElementNS('http://www.w3.org/2000/svg',item[0]);Object.keys(item[1]).forEach(function(key){node.setAttribute(key,item[1][key]);});svg.appendChild(node);});return svg;
        }
        function val(value,unit){return value==null||value===''||value==='—'?'—':String(value)+(unit||'');}
        function rate(value){if(!D.known(value)||value<0)return '—';var bits=value*8;return bits>=1e9?(bits/1e9).toFixed(2)+' Gbps':bits>=1e6?(bits/1e6).toFixed(2)+' Mbps':bits>=1e3?(bits/1e3).toFixed(2)+' Kbps':Math.round(bits)+' bps';}
        function temperature(value){return D.known(value)?value.toFixed(1)+' °C':'—';}
        function badge(label,kind){return n('span',{'class':'np-badge '+(kind||'neutral')},label);}
        function action(label,handler,kind){var node=n('button',{type:'button','class':'np-btn '+(kind||'outline')},label);node.addEventListener('click',handler);return node;}
        function tutorial(label,hash,extra){
            var node=n('a',{'class':'np-btn outline np-help-link'+(extra?' '+extra:''),href:bridge.guide+(hash||''),target:'_blank',rel:'noopener noreferrer'},label);
            node.addEventListener('click',function(){var url=new global.URL(node.href);url.searchParams.set('appearance',bridge.dark()?'dark':'light');node.href=url.href;});return node;
        }
        function heading(title,icon,tone){return n('div',{'class':'np-section-heading'},[n('span',{'class':'np-heading-icon '+(tone||'blue')},glyph(icon)),n('div',{'class':'np-native-title',role:'heading','aria-level':3},title)]);}
        function card(title,body,icon,tone,extra){return n('section',{'class':'np-native-card'},[n('div',{'class':'np-native-card-head'},[heading(title,icon||'info',tone),extra]),n('div',{'class':'np-native-card-body'},body)]);}
        function field(label,value){return n('div',{'class':'np-info-field'},[n('span',{'class':'np-info-label'},label),n('strong',{},val(value))]);}
        // 刷新只更新现有展示节点，不拆下整页、短信卡或正在展开的载波详情。
        function patchDisplay(before,after){
            if(before.nodeType!==after.nodeType||before.nodeName!==after.nodeName||before.nodeType===1&&['data-carrier','data-metric'].some(function(key){return before.getAttribute(key)!==after.getAttribute(key);})){before.replaceWith(after);return after;}
            if(before.nodeType===3){if(before.nodeValue!==after.nodeValue)before.nodeValue=after.nodeValue;return before;}
            Array.from(before.attributes).forEach(function(attr){if(!after.hasAttribute(attr.name))before.removeAttribute(attr.name);});
            Array.from(after.attributes).forEach(function(attr){if(before.getAttribute(attr.name)!==attr.value)before.setAttribute(attr.name,attr.value);});
            var children=Array.from(after.childNodes);
            children.forEach(function(child,index){if(before.childNodes[index])patchDisplay(before.childNodes[index],child);else before.appendChild(child);});
            while(before.childNodes.length>children.length)before.lastChild.remove();
            return before;
        }
        function metrics(primary,stale){
            var specs=[['rsrp','RSRP','dBm',-120,-75,'blue',D.rsrpQuality],['rsrq','RSRQ','dB',-20,-3,'orange',D.rsrqQuality],['sinr','SINR','dB',-5,30,'green',D.sinrQuality]];
            if(!stale&&D.known(primary.rssi))specs.push(['rssi','RSSI','dBm',-100,-45,'purple',D.rssiQuality]);
            return n('div',{'class':'np-signal-grid'+(specs.length===3?' three':' four')},specs.map(function(s){
                var value=stale?null:primary[s[0]],known=D.known(value),quality=s[6](value),level=known?quality.level:null;
                var percent=known?Math.max(0,Math.min(100,(value-s[3])/(s[4]-s[3])*100)):0;
                var meter=n('div',{'class':'np-signal-meter'+(known?'':' unknown')},known?n('span',{'class':'np-meter-fill',style:'width:'+percent.toFixed(2)+'%'},n('span',{'class':'np-meter-dot'})):null);
                if(known){meter.setAttribute('role','meter');meter.setAttribute('aria-label',s[1]);meter.setAttribute('aria-valuenow',value);meter.setAttribute('aria-valuemin',Math.min(s[3],value));meter.setAttribute('aria-valuemax',Math.max(s[4],value));}
                return n('article',{'class':'np-signal-parameter '+s[5],'data-metric':s[0]},[
                    n('div',{'class':'np-parameter-head'},[n('strong',{},s[1]),badge(known?quality.text:'未读取',level==null?'neutral':'quality-'+level)]),
                    n('div',{'class':'np-parameter-reading'},[n('strong',{},known?String(value).replace('-','−'):'—'),n('span',{},known?s[2]:'')]),meter,
                    n('div',{'class':'np-parameter-range'},[n('span',{},(s[0]==='rsrq'||s[0]==='sinr'?'差 ':'弱 ')+s[3]),n('span',{},s[4]+(s[0]==='rsrq'||s[0]==='sinr'?' 好':' 强'))])]);
            }));
        }
        function signal(snapshot){
            var radio=snapshot.radio||{},network=D.network(snapshot),p=network.primary||{},stale=!!radio.stale||!!p.stale,operator=radio.networkOperator||{};
            var name=!stale&&!operator.stale?operator.name:null;
            var head=n('div',{'class':'np-signal-heading'},[heading('信号看板','neighbors','blue'),n('div',{'class':'np-signal-context'},[badge(stale?'数据已过期':network.networkMode,'neutral'),n('span',{'class':'np-network-registration'},stale?'未确认':p.rat?'已驻留':'等待驻留')])]);
            var secondary=snapshot.radio?.secondaryMeasurement;
            return n('section',{'class':'np-signal-board'},[head,n('p',{'class':'np-hint'},name||(!stale&&p.mcc?val(p.mcc)+'-'+val(p.mnc):'运营商未读取')),metrics(p,stale),!stale&&secondary?.stale===false&&Number.isFinite(secondary.observedAtMs)?n('div',{},[n('p',{'class':'np-hint'},'5G NSA 信号测量 · 未绑定小区，不计入载波或锁定目标'),metrics(secondary,false)]):null]);
        }
        function carriers(snapshot){
            var r=snapshot.radio||{},net=D.network(snapshot),liveKeys=new Set(),nodes=net.carrierInfo.map(function(c){
                var key=c.title+'|'+c.sysMode+'|'+c.dlFcn;liveKeys.add(key);
                var extra=n('div',{'class':'np-info-grid'},[field('上行频点',c.ulFcn),field('下行频率',val(c.dlFreq,' MHz')),field('上行频率',val(c.ulFreq,' MHz'))]);
                var details=n('details',{'class':'np-carrier-details','data-carrier':key},[n('summary',{},'更多参数'),extra]);
                details.open=carrierDetails.has(key);var previous=details.open;
                details.addEventListener('toggle',function(){if(!details.isConnected||disposed||details.open===previous)return;previous=details.open;if(details.open){carrierDetails.add(key);enter(extra);}else carrierDetails.delete(key);});
                return n('article',{'class':'np-carrier-card'},[
                    n('div',{'class':'np-carrier-head'},[n('div',{'class':'np-carrier-identity'},[n('strong',{},c.title),n('span',{'class':'np-carrier-band'},c.bandShortName)]),badge(c.sysMode)]),
                    n('div',{'class':'np-info-grid np-carrier-essential'},[field('下行频点'+(c.bandDesc?' · '+c.bandDesc:''),c.dlFcn),field('带宽 · 下行 / 上行',val(c.dlBandwidth,' MHz')+' / '+val(c.ulBandwidth,' MHz'))]),details]);});
            carrierDetails.forEach(function(key){if(!liveKeys.has(key))carrierDetails.delete(key);});
            var section=card('载波聚合信息',[n('p',{'class':'np-hint'},r.stale?'缓存过期，载波状态未确认':net.carrierCount?net.carrierCount+' 载波 · 下行 '+val(net.dlBandwidth,' MHz')+' / 上行 '+val(net.ulBandwidth,' MHz'):'未读取载波信息'),n('div',{'class':'np-carrier-grid'},nodes)],'bands','purple');section.classList.add('np-carriers');return section;
        }
        function summary(snapshot){
            var r=snapshot.radio||{},s=snapshot.system||{},net=D.network(snapshot),q=r.stale||r.sessionQos?.stale?{}:r.sessionQos||{},sub=r.stale||r.subscription?.stale?{}:r.subscription||{},op=r.stale||r.networkOperator?.stale?{}:r.networkOperator||{},rates=s.stale||s.network?.stale?{}:s.network?.rates||{};
            var connection=card('网络与速率',[
                n('div',{'class':'np-connection-rates'},[field('↓ 实时下载',rate(rates.downloadBytesPerSecond)),field('↑ 实时上传',rate(rates.uploadBytesPerSecond)),field('签约下行',D.known(sub.dlKbps)?val(sub.dlKbps/1000,' Mbps'):null),field('签约上行',D.known(sub.ulKbps)?val(sub.ulKbps/1000,' Mbps'):null)]),
                n('div',{'class':'np-info-grid np-connection-fields'},[field('运营商',op.name),field('APN',sub.apn),field('PCI',net.pci),field('ARFCN',net.channel),field('MCC / MNC',net.mcc==='—'?'—':net.mcc+' / '+net.mnc),field('TAC',net.lac),field('CELL ID',net.cid),field('服务质量',q.qci==null?null:'QCI '+q.qci+' · 上下文 '+q.cid)]),
                n('p',{'class':'np-hint np-interface-hint'},s.network?.interface?'上网接口 '+s.network.interface:'上网接口尚未确认')],'chart','green');connection.classList.add('np-connection-card');
            var device=card('设备状态',n('div',{'class':'np-device-status-grid'},[
                field('设备温度',s.stale?null:temperature(s.temperature?.celsius)),field('5G 模组温度',r.stale||r.temperature?.stale?null:temperature(r.temperature?.celsius)),
                field('CPU 占用',!s.stale&&D.known(s.cpuUsage)?s.cpuUsage.toFixed(1)+'%':null),field('内存占用',!s.stale&&D.known(s.memory?.usedPercent)?s.memory.usedPercent.toFixed(1)+'%':null)]),'info','purple');
            return [signal(snapshot),carriers(snapshot),connection,device,n('div',{'class':'np-inline-notices'},[
                snapshot.failure?n('p',{'class':'np-notice'},snapshot.failure):null,r.refreshDeferred?n('p',{'class':'np-hint',role:'status'},'模组通道暂忙，展示有效期内的上次数据。'):r.pending?n('p',{'class':'np-hint np-loading-status',role:'status'},[n('span',{'class':'np-spinner','aria-hidden':'true'}),'后台正在更新，先展示已有快照。']):null])];
        }
        function showNeighbors(){
            var snapshot=bridge.snapshot||{},rows=D.neighbors(snapshot,filter,search),stamp=JSON.stringify([rows,filter,search,snapshot.radio?.stale,snapshot.capabilities?.lockCell]);
            if(stamp===neighborStamp)return;neighborStamp=stamp;count.textContent='共 '+rows.length+' 个 · 信号强度排序';
            neighborRows.replaceChildren.apply(neighborRows,rows.length?rows.map(function(row){
                var choose=action('锁定',function(){var fresh=D.neighbors(bridge.snapshot||{},filter,search).find(function(item){return item.type===row.type&&item.pci===row.pci&&item.arfcn===row.arfcn;});if(!fresh?.lockable)return;try{bridge.chooseCell(fresh);current=5;draw();scrollTop();}catch(e){bridge.notify(e);}},'outline');choose.disabled=!row.lockable;
                return n('article',{'class':'np-neighbor-card'},[n('div',{'class':'np-neighbor-head'},[n('strong',{},row.type+' '+row.bandLabel),choose]),n('p',{'class':'np-hint'},'PCI: '+row.pci+' · ARFCN: '+row.arfcn),n('dl',{'class':'np-neighbor-metrics'},[['RSRP',row.rsrp,'dBm',D.rsrpQuality],['RSRQ',row.rsrq,'dB',D.rsrqQuality],['SINR',row.sinr,'dB',D.sinrQuality]].map(function(v){var quality=v[3](v[1]);return n('div',{},[n('dt',{},v[0]),n('dd',{},[n('strong',{},val(v[1],v[2])),badge(quality.text,quality.level==null?'neutral':'quality-'+quality.level)])]);}))]);
            }):[n('div',{'class':'np-empty-state'},[glyph('neighbors'),n('strong',{},'暂无邻区数据'),n('p',{'class':'np-hint'},search||filter?'可更改筛选条件':'以模组实际返回的测量为准')])]);
            filterButtons.forEach(function(item){item.node.classList.toggle('selected',filter===item.value);item.node.setAttribute('aria-pressed',String(filter===item.value));});
        }
        function copy(item){if(copying||!item.code||disposed)return;copying=true;Promise.resolve().then(function(){return bridge.copySmsCode(item);}).catch(bridge.notify).finally(function(){copying=false;});}
        var readButton=action('读取短信',function(){if(reading||!bridge.canReadSms()||disposed)return;reading=true;draw();Promise.resolve().then(bridge.readSms).catch(bridge.notify).finally(function(){reading=false;if(!disposed)draw();});});
        var smsBody=n('div',{'class':'np-recent-sms-body'}),smsNote=n('p',{'class':'np-hint np-recent-sms-note'});
        var recentCard=card('最近短信',[smsBody,smsNote],'sms','purple',n('div',{'class':'np-actions'},[readButton,action('查看全部',function(){navigate(6);})]));recentCard.classList.add('np-recent-sms');
        function showSms(){
            var cache=bridge.sms()||{},snapshot=bridge.snapshot||{},supported=!!snapshot.capabilities?.sms,items=D.recentSms(cache),stamp=JSON.stringify([items,cache.loaded,cache.stale,cache.error,supported]);
            readButton.disabled=reading||!bridge.canReadSms();readButton.textContent=reading?'正在读取…':'读取短信';readButton.setAttribute('aria-busy',String(reading));
            if(smsStamp===stamp)return;smsStamp=stamp;
            var nodes=items.length?items.map(function(item){
                var article=n('article',{'class':'np-recent-sms-item'},[n('div',{'class':'np-recent-sms-head'},[n('strong',{},item.phone),badge(item.direction!=='incoming'?'未分类':item.unread?'未读':'已读',item.direction==='incoming'&&item.unread?'blue':'neutral')]),n('div',{'class':'np-hint'},item.timestamp),n('p',{'class':'np-recent-sms-preview'},item.content||'（空内容）')]);
                if(!item.complete)article.appendChild(n('p',{'class':'np-hint'},'长短信接收中，暂不识别验证码'));
                else if(item.code){var code=n('button',{type:'button','class':'np-sms-code','aria-label':'复制验证码 '+item.code},item.code);code.addEventListener('click',function(event){if(event.detail===0)copy(item);});article.appendChild(n('div',{'class':'np-recent-sms-code-line'},[code,n('span',{'class':'np-hint'},'双击复制验证码')]));article.addEventListener('dblclick',function(){copy(item);});}
                return article;
            }):[n('div',{'class':'np-empty-state'},[glyph('sms'),n('strong',{},supported?cache.loaded?'暂无收到的短信':'暂无短信预览':'短信通道暂未就绪'),supported?(cache.loaded?n('p',{'class':'np-hint'},'收到短信并读取后显示在这里'):null):n('p',{'class':'np-hint'},'请检查服务与模组通道')])];
            if(cache.error)nodes.push(n('p',{'class':'np-notice',role:'status'},'读取失败：'+cache.error));smsBody.replaceChildren.apply(smsBody,nodes);
            smsNote.textContent=cache.stale&&cache.loaded?'历史缓存 · 点击读取可更新；验证码有效期以原短信为准。':'';smsNote.hidden=!smsNote.textContent;
        }
        function refresh(){if(refreshing||!bridge.canRefresh()||disposed)return;refreshing=true;draw();Promise.resolve().then(bridge.refresh).catch(bridge.notify).finally(function(){refreshing=false;if(!disposed)draw();});}
        var workspace=n('div',{'class':'np-webui-workspace'}),aside=n('aside',{'class':'np-workspace-navigation'}),nav=n('nav',{'class':'np-workspace-nav','aria-label':'网络面板功能导航'}),content=n('section',{'class':'np-webui-content'});
        var title=n('div',{'class':'np-page-title',role:'heading','aria-level':2}),description=n('p',{'class':'np-page-description'}),themeButton=action('外观：自动',function(){theme=theme==='auto'?'light':theme==='light'?'dark':'auto';applyTheme();try{global.sessionStorage.setItem('wlmb-server.appearance',theme);}catch(_){}}),refreshButton=action('刷新数据',refresh,'primary');
        var body=n('div',{'class':'np-native-presentation'}),management=n('div',{'class':'np-management-status'},[bridge.status,n('div',{'class':'np-help-line'},[n('span',{},'第一次使用？启动插件 → 手机同一局域网 → 小程序连接'),tutorial('查看连接教程','#connect')])]);
        var overview=n('div',{'class':'np-network-overview'}),overviewParts=[],neighborRows=n('div',{'class':'np-neighbor-grid'}),count=n('span',{'class':'np-hint'}),searchInput=n('input',{'class':'np-input','aria-label':'筛选邻区',placeholder:'频段、PCI 或频点',type:'search'}),filterButtons=[];
        var filterRow=n('div',{'class':'np-neighbor-filter'});
        [['','全部'],['NR','5G'],['LTE','4G']].forEach(function(item){var node=action(item[1],function(){filter=item[0];showNeighbors();});filterButtons.push({value:item[0],node:node});filterRow.appendChild(node);});
        searchInput.addEventListener('input',function(){search=searchInput.value;showNeighbors();});filterRow.appendChild(searchInput);filterRow.appendChild(count);
        var neighbors=n('div',{'class':'np-neighbor-presentation'},[filterRow,neighborRows]),subnav=n('nav',{'class':'np-section-tabs','aria-label':'当前功能分组'}),mobile=n('nav',{'class':'np-mobile-navigation','aria-label':'主要功能导航'});
        var brand=n('div',{'class':'np-navigation-brand'},[n('strong',{},'网络面板'),n('span',{},'免费 OpenWrt 插件 · 1117')]),service=n('div',{'class':'np-navigation-status',role:'status'});
        nav.id='np-primary-navigation';
        var collapseButton=action('',function(){collapsed=!collapsed;applyCollapse();try{global.sessionStorage.setItem('wlmb-server.sidebar',collapsed?'collapsed':'expanded');}catch(_){} });collapseButton.appendChild(sidebarIcon());collapseButton.classList.add('np-collapse-toggle');collapseButton.setAttribute('aria-controls',nav.id);
        aside.appendChild(n('div',{'class':'np-navigation-top'},[brand,collapseButton]));aside.appendChild(service);aside.appendChild(nav);aside.appendChild(tutorial('连接小程序','#connect','np-navigation-guide'));
        content.appendChild(n('div',{'class':'np-page-heading'},[n('div',{},[title,description]),n('div',{'class':'np-heading-actions'},[themeButton,refreshButton])]));
        content.appendChild(subnav);content.appendChild(management);content.appendChild(bridge.toolbar);content.appendChild(body);content.appendChild(bridge.pages);
        content.appendChild(n('div',{'class':'np-client-entry'},[n('strong',{},'网络面板助手 · 微信小程序'),tutorial('连接教程','#connect')]));content.appendChild(bridge.footer);
        workspace.appendChild(aside);workspace.appendChild(content);workspace.appendChild(mobile);target.appendChild(workspace);
        var navFrame=0;
        function positionNavigation(){navFrame=0;if(disposed||!workspace.isConnected)return;var rect=workspace.getBoundingClientRect(),padding=parseFloat(global.getComputedStyle(workspace).paddingLeft)||0;workspace.style.setProperty('--np-navigation-left',(rect.left+padding)+'px');}
        function queueNavigation(){if(!disposed&&!navFrame)navFrame=global.requestAnimationFrame(positionNavigation);}
        var navResize=typeof global.ResizeObserver==='function'?new global.ResizeObserver(queueNavigation):null;
        if(navResize)navResize.observe(workspace);
        global.addEventListener('resize',queueNavigation);global.addEventListener('scroll',queueNavigation,true);positionNavigation();
        var originalToolbarButtons=Array.from(bridge.toolbar.children).filter(function(node){return node.tagName==='BUTTON';});originalToolbarButtons.forEach(function(node){node.hidden=true;});
        try{var saved=global.sessionStorage.getItem('wlmb-server.appearance');if(['auto','light','dark'].includes(saved))theme=saved;collapsed=global.sessionStorage.getItem('wlmb-server.sidebar')==='collapsed';}catch(_){}
        function applyCollapse(){workspace.classList.toggle('is-collapsed',collapsed);collapseButton.setAttribute('aria-expanded',String(!collapsed));collapseButton.setAttribute('aria-label',collapsed?'展开导航':'收起导航');collapseButton.title=collapsed?'展开导航':'收起导航';collapseButton.querySelector('.np-sidebar-direction').setAttribute('d',collapsed?'m14 9 3 3-3 3':'m16 15-3-3 3-3');}
        function scrollTop(){if(!workspace.isConnected)return;var host=workspace.closest('.main-right, main')||workspace.closest('#maincontent')||workspace;host.scrollTop=0;if(typeof host.scrollIntoView==='function')host.scrollIntoView({block:'start',behavior:'auto'});}
        function applyTheme(){
            if(panelRoot)panelRoot.setAttribute('data-np-appearance',theme);
            themeButton.textContent='外观：'+({auto:'自动',light:'浅色',dark:'深色'})[theme];var mode=bridge.dark()?'dark':'light';workspace.setAttribute('data-color-mode',mode);content.setAttribute('data-color-mode',mode);
            if(panelRoot)panelRoot.querySelectorAll('a.np-help-link').forEach(function(node){var url=new global.URL(node.href);url.searchParams.set('appearance',mode);node.href=url.href;});
            if(bridge.syncSurface)bridge.syncSurface();
        }
        function menu(){return definitions.filter(function(entry){return !bridge.tabs.children[entry.key].hidden;});}
        function groupMenu(entries){return groups.map(function(group,index){return {group:group,index:index,keys:group.keys.filter(function(key){return entries.some(function(entry){return entry.key===key;});})};}).filter(function(item){return item.keys.length;});}
        function enterGroup(item){navigate(item.keys.includes(lastGroup[item.index])?lastGroup[item.index]:item.keys[0]);}
        var menuStamp='',subnavStamp='';
        function navigate(key){if(disposed||key===current||!menu().some(function(entry){return entry.key===key;}))return;current=key;lastGroup[definitions.find(function(entry){return entry.key===key;}).group]=key;bridge.select(key);draw();scrollTop();}
        nav.addEventListener('keydown',function(event){if(!['ArrowUp','ArrowDown','Home','End'].includes(event.key))return;var entries=groupMenu(menu());if(!entries.length)return;event.preventDefault();var group=definitions.find(function(entry){return entry.key===current;}).group,at=entries.findIndex(function(entry){return entry.index===group;}),next=event.key==='Home'?0:event.key==='End'?entries.length-1:(at+(event.key==='ArrowDown'?1:-1)+entries.length)%entries.length;enterGroup(entries[next]);var node=nav.querySelector('[data-group="'+entries[next].index+'"]');if(node)node.focus();});
        function updateNavigation(entries){
            var stamp=JSON.stringify(entries.map(function(entry){return entry.key;}));
            if(stamp!==menuStamp){menuStamp=stamp;nav.replaceChildren();mobile.replaceChildren();groupMenu(entries).forEach(function(item){
                [nav,mobile].forEach(function(container){var node=n('button',{type:'button','class':container===nav?'np-menu-button':'np-mobile-button','data-group':item.index,'aria-label':item.group.name,title:item.group.name},[glyph(item.group.icon),n('span',{},item.group.name)]);node.addEventListener('click',function(){enterGroup(item);});container.appendChild(node);});});}
            var group=definitions.find(function(entry){return entry.key===current;}).group;
            nav.querySelectorAll('button').forEach(function(node){var active=Number(node.dataset.group)===group;node.classList.toggle('is-active',active);if(active)node.setAttribute('aria-current','page');else node.removeAttribute('aria-current');});
            mobile.querySelectorAll('button').forEach(function(node){var active=Number(node.dataset.group)===group;node.classList.toggle('is-active',active);if(active)node.setAttribute('aria-current','page');else node.removeAttribute('aria-current');});
            var peers=groups[group].keys.filter(function(key){return entries.some(function(entry){return entry.key===key;});});subnav.hidden=peers.length<2;
            var subStamp=JSON.stringify(peers);if(subStamp!==subnavStamp){subnavStamp=subStamp;subnav.replaceChildren();
                peers.forEach(function(key){var node=action(definitions.find(function(entry){return entry.key===key;}).name,function(){navigate(key);});node.dataset.page=String(key);subnav.appendChild(node);});}
            subnav.querySelectorAll('button').forEach(function(node){var active=Number(node.dataset.page)===current;node.classList.toggle('selected',active);node.setAttribute('aria-current',active?'page':'false');});
        }
        function release(){if(disposed)return;disposed=true;cancelMotion();if(navFrame)global.cancelAnimationFrame(navFrame);if(navResize)navResize.disconnect();global.removeEventListener('resize',queueNavigation);global.removeEventListener('scroll',queueNavigation,true);if(motion?.removeEventListener)motion.removeEventListener('change',motionChanged);observer.disconnect();bridge.redraw=null;originalToolbarButtons.forEach(function(node){node.hidden=false;});var select=bridge.toolbar.querySelector('select');if(select)select.hidden=false;}
        function draw(){
            if(disposed||failed)return;
            try{
                var entries=menu(),snapshot=bridge.snapshot||{},runtime=bridge.runtime()||{};
                if(!entries.some(function(entry){return entry.key===current;})){current=0;bridge.select(0);}
                lastGroup[definitions.find(function(entry){return entry.key===current;}).group]=current;
                updateNavigation(entries);applyTheme();applyCollapse();var page=definitions.find(function(entry){return entry.key===current;});title.textContent=page.name;
                description.textContent=(snapshot.system?.board?.model||snapshot.system?.board?.board_name||'OpenWrt 设备')+' · 根据qmodem为参考开发接入';
                service.replaceChildren(n('span',{'class':'np-service-dot'+(runtime.running&&runtime.controlReady?' ready':'')}),doc.createTextNode((runtime.failure?'状态未确认':runtime.running?runtime.controlReady?'运行中':'启动中':'已停止')+' · '+(snapshot.connection?.mode==='shared'?'共享通道':snapshot.connection?.mode==='independent'?'独立通道':'通道待核对')));
                refreshButton.disabled=refreshing||!bridge.canRefresh();refreshButton.textContent=refreshing?'正在刷新…':'刷新数据';refreshButton.setAttribute('aria-busy',String(refreshing));management.hidden=current!==3;
                var selector=bridge.toolbar.querySelector('select');if(selector)selector.hidden=(snapshot.modems||[]).length<=1;
                body.hidden=current!==0&&current!==1;bridge.pages.hidden=current===0||current===1;
                if(current===0){if(overview.parentNode!==body)body.replaceChildren(overview);var stamp=JSON.stringify([snapshot.radio,snapshot.system?.network,snapshot.system?.board,snapshot.system?.temperature,snapshot.system?.cpuUsage,snapshot.system?.memory?.usedPercent,snapshot.system?.stale,snapshot.failure]);if(stamp!==overviewStamp){overviewStamp=stamp;var next=summary(snapshot);if(!overviewParts.length){overviewParts=next;overview.replaceChildren(next[0],next[1],next[2],n('div',{'class':'np-overview-bottom'},[next[3],recentCard]),next[4]);}else overviewParts=overviewParts.map(function(node,index){return patchDisplay(node,next[index]);});}showSms();}
                if(current===1){if(neighbors.parentNode!==body)body.replaceChildren(neighbors);showNeighbors();}
                if(paintedPage!==current){cancelMotion();paintedPage=current;enter(title);enter(subnav);enter(body.hidden?Array.from(bridge.pages.children).find(function(node){return !node.hidden;}):body);}
            }catch(error){failed=true;release();bridge.fail(error);}
        }
        var observer=new global.MutationObserver(function(){if(!disposed)applyTheme();});observer.observe(doc.documentElement,{attributes:true,attributeFilter:['data-theme','data-darkmode','class']});observer.observe(doc.body,{attributes:true,attributeFilter:['data-theme','data-darkmode','class']});if(panelRoot)observer.observe(panelRoot,{attributes:true,attributeFilter:['style']});
        bridge.redraw=draw;draw();if(!failed){bridge.ready();scrollTop();}
        return {unmount:function(){release();workspace.remove();}};
    }};
})(window);
