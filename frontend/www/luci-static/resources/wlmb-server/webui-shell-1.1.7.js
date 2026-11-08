// 网络面板自有前端；仅处理展示和已授权RPC调用，不直接操作模组。
(function(global) {
    'use strict';
    var definitions=[{key:0,name:'网络信息',icon:'home',group:0},{key:1,name:'邻区扫描',icon:'neighbors',group:1},{key:4,name:'WiFi 与设备',icon:'wifi',group:2},{key:5,name:'频段与小区',icon:'bands',group:1},{key:6,name:'短信',icon:'sms',group:3},{key:7,name:'短信转发',icon:'sms',group:3},{key:8,name:'设备与流量',icon:'chart',group:4},{key:2,name:'AT 控制台',icon:'terminal',group:4},{key:3,name:'插件设置',icon:'settings',group:4}];
    var groups=[{name:'概览',icon:'home',keys:[0]},{name:'网络',icon:'neighbors',keys:[1,5]},{name:'WiFi',icon:'wifi',keys:[4]},{name:'短信',icon:'sms',keys:[6,7]},{name:'设置',icon:'settings',keys:[3,8,2]}];
    global.WLMBWebUI={version:'1.1.7',mount:function(target,bridge){
        var D=global.WLMBUIData,I=global.WLMBIcons,doc=target.ownerDocument;
        if(!D||D.version!=='1.1.7'||!I||I.version!=='1.1.7')throw new Error('独立前端资源版本不一致');
        var disposed=false,failed=false,current=bridge.active||0,filter='',search='',refreshing=false,reading=false,copying=false,smsStamp='',overviewStamp='',neighborStamp='';
        var panelRoot=target.closest('#wlmb-panel'),theme='auto',lastGroup={};
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
        function signal(snapshot){
            var radio=snapshot.radio||{},network=D.network(snapshot),p=network.primary||{},stale=!!radio.stale||!!p.stale,operator=radio.networkOperator||{};
            var name=!stale&&!operator.stale?operator.name:null,score=D.scoreSignal(radio,p),scored=score.value!==null;
            var radioNote=score.carrierText?'载波 '+score.carrierText:scored?'当前小区信号':p.rat?'等待完整的信号参数':'等待模组采样';
            var specs=[
                ['rsrp','RSRP','dBm','接收强度',-160,-20],
                ['rsrq','RSRQ','dB','信号质量',-60,3],
                ['sinr','SINR','dB','信干噪比',-40,60]
            ];
            var readings=specs.map(function(s){
                var value=p[s[0]],valid=!stale&&D.known(value)&&value>=s[4]&&value<=s[5];
                return n('div',{'class':'np-hero-metric','data-metric':s[0]},[
                    n('span',{'class':'np-hero-label'},s[1]),
                    n('div',{'class':'np-hero-number'},[n('strong',{},valid?String(value):'—'),valid?n('span',{},s[2]):null]),
                    n('span',{'class':'np-hero-detail'},s[3])
                ]);
            });
            var extra=[], secondary=radio.secondaryMeasurement;
            if(!stale&&D.known(p.rssi))extra.push(n('div',{},['RSSI ',n('strong',{},val(p.rssi,' dBm'))]));
            if(!stale&&secondary?.stale===false&&Number.isFinite(secondary.observedAtMs)){
                var measured=[['rsrp','RSRP',' dBm'],['rsrq','RSRQ',' dB'],['sinr','SINR',' dB'],['rssi','RSSI',' dBm']].filter(function(spec){return D.known(secondary[spec[0]]);});
                if(measured.length)extra.push(n('div',{},[n('span',{},'5G NSA 独立测量（未绑定小区）')].concat(measured.map(function(spec){return n('strong',{},spec[1]+' '+val(secondary[spec[0]],spec[2]));}))));
            }
            return n('section',{'class':'np-signal-hero'+(scored?' has-signal':''),'aria-label':'信号看板'},[
                n('div',{'class':'np-hero-header'},[
                    n('div',{'class':'np-hero-network'},[n('h3',{},name||(!stale&&p.mcc?val(p.mcc)+'-'+val(p.mnc):'蜂窝网络')),!stale&&p.rat?n('span',{'class':'np-hero-mode'},network.networkMode):null]),
                    n('span',{'class':'np-hero-state'},stale?'数据已过期':scored?score.label:p.rat?'信号不完整':'信号未读取')
                ]),
                n('div',{'class':'np-hero-score'},[
                    n('span',{},'综合信号评分'),
                    n('div',{'class':'np-hero-score-value'+(scored?'':' is-unavailable')},[n('strong',{},scored?score.value.toFixed(1):'未读取'),scored?n('span',{},'/ 10'):null]),
                    n('span',{'class':'np-hero-note'},radioNote+(score.bandwidth===null?'':' · 下行 '+score.bandwidth+' MHz'))
                ]),
                n('div',{'class':'np-hero-bars','aria-hidden':'true'},[0,1,2,3,4].map(function(i){
                    return n('i',{'class':i<score.level?'is-active':''});
                })),
                n('div',{'class':'np-hero-metrics'},readings),
                n('div',{'class':'np-hero-footer'},[
                    n('span',{'class':'np-network-registration'},[glyph('neighbors'),stale?'驻留状态未确认':p.rat?'已驻留':'等待驻留']),
                    !stale&&D.known(p.pci)?n('span',{},'PCI '+p.pci+(D.known(p.channel)?' · '+(p.rat==='LTE'?'EARFCN ':'ARFCN ')+p.channel:'')):null
                ]),
                extra.length?n('div',{'class':'np-hero-supplement'},extra):null
            ]);
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
            var state=r.carrierState||{},hint=r.stale||state.stale?'缓存过期，载波状态未确认':state.queryStatus==='pending'?'正在更新聚合信息，先展示已有载波':state.queryStatus==='error'?'聚合信息读取未成功，展示已确认的服务载波':state.status==='serving-only'?'已读取当前服务载波，尚未报告额外聚合载波':net.carrierCount?net.carrierCount+' 载波 · 下行 '+val(net.dlBandwidth,' MHz')+' / 上行 '+val(net.ulBandwidth,' MHz'):'未读取载波信息';
            var section=card('载波聚合信息',[n('p',{'class':'np-hint'},hint),n('div',{'class':'np-carrier-grid'},nodes)],'bands','purple');section.classList.add('np-carriers');return section;
        }
        function summary(snapshot){
            var r=snapshot.radio||{},s=snapshot.system||{},net=D.network(snapshot),q=r.stale||r.sessionQos?.stale?{}:r.sessionQos||{},sub=r.stale||r.subscription?.stale?{}:r.subscription||{},op=r.stale||r.networkOperator?.stale?{}:r.networkOperator||{},rates=s.stale||s.network?.stale?{}:s.network?.rates||{};
            var connection=card('网络与速率',[
                n('div',{'class':'np-connection-rates'},[field('↓ 实时下载',rate(rates.downloadBytesPerSecond)),field('↑ 实时上传',rate(rates.uploadBytesPerSecond)),field('签约下行',D.known(sub.dlKbps)?val(sub.dlKbps/1000,' Mbps'):null),field('签约上行',D.known(sub.ulKbps)?val(sub.ulKbps/1000,' Mbps'):null)]),
                n('div',{'class':'np-info-grid np-connection-fields'},[field('运营商',op.name),field('APN',sub.apn),field('PCI',net.pci),field('ARFCN',net.channel),field('MCC / MNC',net.mcc==='—'?'—':net.mcc+' / '+net.mnc),field('TAC',net.lac),field('CELL ID',net.cid),field('服务质量',q.qci==null?null:'QCI '+q.qci+' · 上下文 '+q.cid)]),
                n('p',{'class':'np-hint np-interface-hint'},s.network?.interface?'上网接口 '+s.network.interface:'上网接口尚未确认')],'chart','green');connection.classList.add('np-connection-card');
            var power=D.battery(snapshot);
            var device=card('设备状态',n('div',{'class':'np-device-status-grid'},[
                field('设备温度',s.stale?null:temperature(s.temperature?.celsius)),field('5G 模组温度',r.stale||r.temperature?.stale?null:temperature(r.temperature?.celsius)),
                field('CPU 占用',!s.stale&&D.known(s.cpuUsage)?s.cpuUsage.toFixed(1)+'%':null),field('内存占用',!s.stale&&D.known(s.memory?.usedPercent)?s.memory.usedPercent.toFixed(1)+'%':null),
                power.supported?field('设备电量',power.label):null]),'info','purple');
            return [signal(snapshot),carriers(snapshot),connection,device,n('div',{'class':'np-inline-notices'},[
                snapshot.failure?n('p',{'class':'np-notice'},snapshot.failure):null,r.refreshDeferred?n('p',{'class':'np-hint',role:'status'},'模组通道暂忙，展示有效期内的上次数据。'):r.pending?n('p',{'class':'np-hint np-loading-status',role:'status'},[n('span',{'class':'np-spinner','aria-hidden':'true'}),'后台正在更新，先展示已有快照。']):null])];
        }
        function showNeighbors(){
            var snapshot=bridge.snapshot||{},state=snapshot.radio?.neighborState||{},rows=D.neighbors(snapshot,filter,search),stamp=JSON.stringify([rows,filter,search,snapshot.radio?.stale,snapshot.capabilities?.lockCell,state]);
            if(stamp===neighborStamp)return;neighborStamp=stamp;count.textContent='共 '+rows.length+' 个 · 信号强度排序';
            neighborRows.replaceChildren.apply(neighborRows,rows.length?rows.map(function(row){
                var choose=action('锁定',function(){var fresh=D.neighbors(bridge.snapshot||{},filter,search).find(function(item){return item.type===row.type&&item.pci===row.pci&&item.arfcn===row.arfcn;});if(!fresh?.lockable)return;try{bridge.chooseCell(fresh);current=5;draw();scrollTop();}catch(e){bridge.notify(e);}},'outline');choose.disabled=!row.lockable;
                return n('article',{'class':'np-neighbor-card'},[n('div',{'class':'np-neighbor-head'},[n('strong',{},row.type+' '+row.bandLabel),choose]),n('p',{'class':'np-hint'},'PCI: '+row.pci+' · ARFCN: '+row.arfcn),n('dl',{'class':'np-neighbor-metrics'},[['RSRP',row.rsrp,'dBm',D.rsrpQuality],['RSRQ',row.rsrq,'dB',D.rsrqQuality],['SINR',row.sinr,'dB',D.sinrQuality]].map(function(v){var quality=v[3](v[1]);return n('div',{},[n('dt',{},v[0]),n('dd',{},[n('strong',{},val(v[1],v[2])),badge(quality.text,quality.level==null?'neutral':'quality-'+quality.level)])]);}))]);
            }):[n('div',{'class':'np-empty-state'},[glyph('neighbors'),n('strong',{},state.queryStatus==='pending'?'正在读取邻区':state.queryStatus==='error'?'邻区读取未成功':state.status==='unsupported'?'被动邻区查询暂不可用':'暂无邻区数据'),n('p',{'class':'np-hint'},search||filter?'可更改筛选条件':state.queryStatus==='error'?'请稍后刷新；读取失败不代表附近没有基站':state.status==='unsupported'?'当前通道未提供经核验的被动查询，不会自动搜网':state.status==='empty'?'模组未报告可用邻区；部分固件不返回 5G 邻区测量':'以模组实际返回的测量为准')])]);
            neighborNotice.textContent=state.automaticRetryPaused?'原事务结果未确认，已暂停自动邻区重试；请先确认 modem 服务恢复，刷新页面不会重发。':rows.length&&(state.queryStatus==='error'||state.stale)?'邻区状态尚未确认，展示上次读取的记录。':'';neighborNotice.hidden=!neighborNotice.textContent;
            filterButtons.forEach(function(item){item.node.setAttribute('aria-pressed',String(filter===item.value));});
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
        var workspace=n('div',{'class':'np-webui-workspace'}),nav=n('nav',{'class':'np-primary-tabs','aria-label':'网络面板功能导航'}),content=n('section',{'class':'np-webui-content'});
        var title=n('div',{'class':'np-page-title',role:'heading','aria-level':2}),description=n('p',{'class':'np-page-description'}),themeButton=action('外观：自动',function(){theme=theme==='auto'?'light':theme==='light'?'dark':'auto';applyTheme();try{global.sessionStorage.setItem('wlmb-server.appearance',theme);}catch(_){}}),refreshButton=action('刷新数据',refresh,'primary');
        var body=n('div',{'class':'np-native-presentation'}),management=n('div',{'class':'np-management-status'},[bridge.status,n('div',{'class':'np-help-line'},[n('span',{},'第一次使用？启动插件 → 手机同一局域网 → 小程序连接'),tutorial('查看连接教程','#connect')])]);
        var overview=n('div',{'class':'np-network-overview'}),overviewParts=[],neighborRows=n('div',{'class':'np-neighbor-grid'}),count=n('span',{'class':'np-hint'}),searchInput=n('input',{'class':'np-neighbor-search','aria-label':'筛选邻区',placeholder:'频段、PCI 或频点',type:'search'}),filterButtons=[];
        var filterOptions=n('div',{'class':'np-neighbor-options',role:'group','aria-label':'邻区制式'});
        [['','全部'],['NR','5G'],['LTE','4G']].forEach(function(item){
            var node=n('button',{type:'button','class':'np-neighbor-option','aria-pressed':String(filter===item[0])},item[1]);
            node.addEventListener('click',function(){filter=item[0];showNeighbors();});
            filterButtons.push({value:item[0],node:node});
            filterOptions.appendChild(node);
        });
        searchInput.addEventListener('input',function(){search=searchInput.value;showNeighbors();});
        var filterRow=n('div',{'class':'np-neighbor-filter'},[filterOptions,searchInput]);
        var neighborNotice=n('p',{'class':'np-hint',role:'status'}),subnav=n('nav',{'class':'np-section-tabs','aria-label':'当前功能分组'});
        var neighbors=card('邻区列表',[filterRow,neighborRows,neighborNotice],'neighbors','blue',count);
        neighbors.classList.add('np-neighbor-presentation');
        var serviceDot=n('span',{'class':'np-service-dot','aria-hidden':'true'}),serviceText=doc.createTextNode(''),service=n('div',{'class':'np-service-status',role:'status'},[serviceDot,serviceText]);
        content.appendChild(n('div',{'class':'np-page-heading'},[n('div',{},[title,description,service]),n('div',{'class':'np-heading-actions'},[themeButton,refreshButton])]));
        content.appendChild(nav);content.appendChild(subnav);
        content.appendChild(n('div',{'class':'np-page-region'},[management,bridge.toolbar,body,bridge.pages]));
        content.appendChild(n('div',{'class':'np-client-entry'},[n('strong',{},'网络面板助手 · 微信小程序'),tutorial('连接教程','#connect')]));content.appendChild(bridge.footer);
        workspace.appendChild(content);target.appendChild(workspace);
        function fitWorkspace(){
            var top=target.getBoundingClientRect().top+global.scrollY,tail=0;
            for(var host=target.parentElement;host&&host!==doc.body;host=host.parentElement){
                var style=global.getComputedStyle(host);
                top+=host.scrollTop;
                tail+=(parseFloat(style.paddingBottom)||0)+(parseFloat(style.marginBottom)||0);
            }
            var footer=doc.querySelector('body > footer, .main-right > footer, #maincontent > footer');
            if(footer)tail+=footer.getBoundingClientRect().height;
            var height=doc.documentElement.clientHeight-Math.max(0,top)-tail;
            workspace.style.setProperty('--np-workspace-height',Math.max(0,Math.floor(height))+'px');
        }
        var originalToolbarButtons=Array.from(bridge.toolbar.children).filter(function(node){return node.tagName==='BUTTON';});originalToolbarButtons.forEach(function(node){node.hidden=true;});
        try{var saved=global.sessionStorage.getItem('wlmb-server.appearance');if(['auto','light','dark'].includes(saved))theme=saved;}catch(_){}
        function scrollTop(){
            if(!workspace.isConnected)return;
            for(var host=workspace.parentElement;host&&host!==doc.body;host=host.parentElement)
                if(host.scrollTop)host.scrollTop=0;
            if(global.scrollY)global.scrollTo({top:0,behavior:'instant'});
        }
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
        nav.addEventListener('keydown',function(event){if(!['ArrowLeft','ArrowRight','Home','End'].includes(event.key))return;var entries=groupMenu(menu());if(!entries.length)return;event.preventDefault();var group=definitions.find(function(entry){return entry.key===current;}).group,at=entries.findIndex(function(entry){return entry.index===group;}),next=event.key==='Home'?0:event.key==='End'?entries.length-1:(at+(event.key==='ArrowRight'?1:-1)+entries.length)%entries.length;enterGroup(entries[next]);var node=nav.querySelector('[data-group="'+entries[next].index+'"]');if(node)node.focus();});
        function updateNavigation(entries){
            var stamp=JSON.stringify(entries.map(function(entry){return entry.key;}));
            if(stamp!==menuStamp){menuStamp=stamp;nav.replaceChildren();groupMenu(entries).forEach(function(item){
                var node=n('button',{type:'button','class':'np-primary-tab','data-group':item.index},[glyph(item.group.icon),n('span',{},item.group.name)]);
                node.addEventListener('click',function(){enterGroup(item);});nav.appendChild(node);
            });}
            var group=definitions.find(function(entry){return entry.key===current;}).group;
            nav.querySelectorAll('button').forEach(function(node){var active=Number(node.dataset.group)===group;node.classList.toggle('is-active',active);if(active)node.setAttribute('aria-current','page');else node.removeAttribute('aria-current');});
            var peers=groups[group].keys.filter(function(key){return entries.some(function(entry){return entry.key===key;});});subnav.hidden=peers.length<2;
            var subStamp=JSON.stringify(peers);if(subStamp!==subnavStamp){subnavStamp=subStamp;subnav.replaceChildren();
                peers.forEach(function(key){
                    var node=n('button',{type:'button','class':'np-section-tab','data-page':String(key)},definitions.find(function(entry){return entry.key===key;}).name);
                    node.addEventListener('click',function(){navigate(key);});
                    subnav.appendChild(node);
                });}
            subnav.querySelectorAll('button').forEach(function(node){node.setAttribute('aria-current',Number(node.dataset.page)===current?'page':'false');});
        }
        function release(){if(disposed)return;disposed=true;cancelMotion();global.removeEventListener('resize',fitWorkspace);if(motion?.removeEventListener)motion.removeEventListener('change',motionChanged);observer.disconnect();bridge.redraw=null;originalToolbarButtons.forEach(function(node){node.hidden=false;});var select=bridge.toolbar.querySelector('select');if(select)select.hidden=false;}
        function draw(){
            if(disposed||failed)return;
            try{
                var entries=menu(),snapshot=bridge.snapshot||{},runtime=bridge.runtime()||{};
                if(!entries.some(function(entry){return entry.key===current;})){current=0;bridge.select(0);}
                lastGroup[definitions.find(function(entry){return entry.key===current;}).group]=current;
                updateNavigation(entries);applyTheme();var page=definitions.find(function(entry){return entry.key===current;});title.textContent=page.name;
                description.textContent=(snapshot.system?.board?.model||snapshot.system?.board?.board_name||'OpenWrt 设备')+(current===0?' · 模组信号与载波':'');
                serviceDot.classList.toggle('ready',!!(runtime.running&&runtime.controlReady));
                var serviceLabel=(runtime.failure?'状态未确认':runtime.running?runtime.controlReady?'运行中':'启动中':'已停止')+' · '+(snapshot.connection?.sharedService==='luoye-modem-cache'?'modem 缓存':snapshot.connection?.mode==='shared'?'共享通道':snapshot.connection?.mode==='independent'?'独立通道':'通道待核对');
                if(serviceText.nodeValue!==serviceLabel)serviceText.nodeValue=serviceLabel;
                refreshButton.disabled=refreshing||!bridge.canRefresh();refreshButton.textContent=refreshing?'正在刷新…':'刷新数据';refreshButton.setAttribute('aria-busy',String(refreshing));management.hidden=current!==3;
                var selector=bridge.toolbar.querySelector('select');if(selector)selector.hidden=(snapshot.modems||[]).length<=1;
                body.hidden=current!==0&&current!==1;bridge.pages.hidden=current===0||current===1;
                if(current===0){if(overview.parentNode!==body)body.replaceChildren(overview);var stamp=JSON.stringify([snapshot.radio,snapshot.system?.network,snapshot.system?.board,snapshot.system?.temperature,snapshot.system?.cpuUsage,snapshot.system?.memory?.usedPercent,snapshot.system?.battery,snapshot.system?.stale,snapshot.failure]);if(stamp!==overviewStamp){overviewStamp=stamp;var next=summary(snapshot);if(!overviewParts.length){overviewParts=next;overview.replaceChildren(next[0],next[1],next[2],n('div',{'class':'np-overview-bottom'},[next[3],recentCard]),next[4]);}else overviewParts=overviewParts.map(function(node,index){return patchDisplay(node,next[index]);});}showSms();}
                if(current===1){if(neighbors.parentNode!==body)body.replaceChildren(neighbors);showNeighbors();}
                if(paintedPage!==current){cancelMotion();paintedPage=current;enter(title);enter(subnav);enter(body.hidden?Array.from(bridge.pages.children).find(function(node){return !node.hidden;}):body);}
            }catch(error){failed=true;release();bridge.fail(error);}
        }
        var observer=new global.MutationObserver(function(){if(!disposed)applyTheme();});observer.observe(doc.documentElement,{attributes:true,attributeFilter:['data-theme','data-darkmode','class']});observer.observe(doc.body,{attributes:true,attributeFilter:['data-theme','data-darkmode','class']});if(panelRoot)observer.observe(panelRoot,{attributes:true,attributeFilter:['style']});
        bridge.redraw=draw;draw();
        if(!failed){
            bridge.ready();
            fitWorkspace();
            global.addEventListener('resize',fitWorkspace);
            scrollTop();
        }
        return {unmount:function(){release();workspace.remove();}};
    }};
})(window);
