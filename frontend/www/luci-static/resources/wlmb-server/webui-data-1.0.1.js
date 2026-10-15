// 网络面板自有前端；仅处理展示和已授权RPC调用，不直接操作模组。
(function(global) {
    'use strict';
// 小区草稿只补齐唯一且已核验的信息；不猜测 SCS，不发送网络请求。
function resolveCellDraft(input, options = {}) {
  const rat = input.rat, nr = rat === 'NR'
  const empty = value => value === undefined || value === null || value === ''
  const integer = (value, max) => !empty(value) && /^\d+$/.test(String(value)) && Number.isSafeInteger(Number(value)) && Number(value) <= max
  const missing = { missingBand: false, missingScs: false }
  const reply = (cell, error = '') => ({ missingBand: missing.missingBand, missingScs: missing.missingScs, cell: cell || null, error })
  if (!['NR', 'LTE'].includes(rat)) return reply(null, '请选择 5G 或 4G')
  if (!integer(input.pci, nr ? 1007 : 503)) return reply(null, 'PCI 应为 0–' + (nr ? 1007 : 503) + ' 的整数')
  if (!integer(input.arfcn, nr ? 3279165 : 262143)) return reply(null, '请填写有效的 ARFCN / EARFCN')
  const cell = { rat, pci: Number(input.pci), arfcn: Number(input.arfcn) }
  const fields = options.fields || {}, bandRequired = fields.bandRequired !== false, scsRequired = nr && fields.scsRequired !== false
  const rows = (options.choices || []).filter(row => row && row.rat === rat && !row.stale && (!nr || row.channelType !== 'center') && integer(row.pci, nr ? 1007 : 503) && integer(row.arfcn ?? row.channel, nr ? 3279165 : 262143) && Number(row.pci) === cell.pci && Number(row.arfcn ?? row.channel) === cell.arfcn)
  const prefix = nr ? 'n' : 'B', normalize = value => {
    const match = new RegExp('^' + prefix + '([1-9]\\d{0,3})$', 'i').exec(String(value || ''))
    return match ? prefix + Number(match[1]) : ''
  }
  if (bandRequired) {
    if (!empty(input.band) && !normalize(input.band)) { missing.missingBand = true; return reply(cell, '频段格式如 ' + (nr ? 'n78' : 'B3')) }
    let band = normalize(input.band)
    if (!band) {
      const measured = [...new Set(rows.map(row => normalize(row.band)).filter(Boolean))]
      if (measured.length === 1) band = measured[0]
      else if (!measured.length) {
        const ranges = [...new Set((options.ranges || []).filter(row => cell.arfcn >= row.first && cell.arfcn <= row.last).map(row => prefix + row.band))]
        if (ranges.length === 1) band = ranges[0]
      }
    }
    if (band) cell.band = band
    else missing.missingBand = true
  }
  if (scsRequired) {
    if (!empty(input.scsType) && !integer(input.scsType, 4)) { missing.missingScs = true; return reply(cell, '请选择有效的 SCS') }
    let scs = empty(input.scsType) ? null : Number(input.scsType)
    if (scs === null) {
      const measured = [...new Set(rows.filter(row => integer(row.scsType, 4)).map(row => Number(row.scsType)))]
      if (measured.length === 1) scs = measured[0]
    }
    if (scs !== null) cell.scsType = scs
    else missing.missingScs = true
  }
  return reply(cell)
}
    function known(value) { return typeof value === 'number' && Number.isFinite(value); }
    function show(value) { return value == null || value === '' ? '—' : value; }
    function band(row) { return row.band || (row.bandCandidates || []).join(' / ') + ((row.bandCandidates || []).length ? '（候选）' : '') || '未知频段'; }
    function quality(value, limits) {
        if (!known(value)) return {color:'#8c8c8c',text:'未读取'};
        var level=value>=limits[0]?0:value>=limits[1]?1:value>=limits[2]?2:3;
        return {level:level,color:['#52c41a','#1890ff','#faad14','#f5222d'][level],text:['优秀','良好','一般','较差'][level]};
    }
    function total(rows,key) { return rows.length && rows.every(function(row){return known(row[key]);}) ? rows.reduce(function(sum,row){return sum+row[key];},0) : '—'; }
    function frequency(row) {
        if (!known(row.channel)) return '—';
        if(row.rat==='NR') return Math.round((row.channel<600000 ? row.channel*.005 : row.channel<2016667 ? 3000+(row.channel-600000)*.015 : 24250.08+(row.channel-2016667)*.06)*1000)/1000;
        return show(row.dlFrequencyMHz);
    }
    function verificationCode(content) {
        var source=String(content||'').slice(0,4096).normalize('NFKC');
        var keywords=/验证码|校验码|动态码|认证码|安全码|短信码|一次性密码|(?:verification|security|login|authentication|sms)\s+code|one[\s-]*time\s+(?:password|code)|\b(?:otp|passcode)\b/gi;
        var locations=[],match,codes=new Set();
        while((match=keywords.exec(source)))locations.push({start:match.index,end:keywords.lastIndex});
        if(!locations.length)return null;
        var tokens=/(^|[^a-z0-9])([a-z0-9]{4,8})(?![a-z0-9])/gi;
        var glue=/^(?:[\s:：=,，()（）\[\]【】"'「」-]|is|为|是|您的|本次|的)*$/i;
        while((match=tokens.exec(source))){
            var code=match[2],start=match.index+match[1].length,end=start+code.length;
            if(!/\d/.test(code)||/^(?:[年月日时分秒元圆%％]|\.\d)/.test(source.slice(end)))continue;
            if(locations.some(function(keyword){
                return start>=keyword.end&&start-keyword.end<=24&&glue.test(source.slice(keyword.end,start)) ||
                    end<=keyword.start&&keyword.start-end<=20&&glue.test(source.slice(end,keyword.start));
            }))codes.add(code);
        }
        return codes.size===1?Array.from(codes)[0]:null;
    }
    function messageTime(value) {
        var raw=String(value||'').trim();
        if(/^\d{10}$/.test(raw))return Number(raw)*1000;
        if(/^\d{13}$/.test(raw))return Number(raw);
        var at=/^(\d{2})\/(\d{2})\/(\d{2}),\s*(\d{2}):(\d{2}):(\d{2})([+-])(\d{2})$/.exec(raw);
        if(at){
            var date=new Date(Date.UTC(2000+Number(at[1]),Number(at[2])-1,Number(at[3]),Number(at[4]),Number(at[5]),Number(at[6])));
            if(date.getUTCMonth()!==Number(at[2])-1||date.getUTCDate()!==Number(at[3])||Number(at[4])>23||Number(at[5])>59||Number(at[6])>59||Number(at[8])>56)return 0;
            return date.getTime()-(at[7]==='+'?1:-1)*Number(at[8])*15*60000;
        }
        if(!/^\d{4}[-/]\d{1,2}[-/]\d{1,2}(?:[T ,]\d{1,2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2})?)?$/.test(raw))return 0;
        var parsed=Date.parse(raw.replace(/\//g,'-').replace(',','T'));
        return Number.isFinite(parsed)?parsed:0;
    }
    function recentSms(value) {
        return (Array.isArray(value?.messages)?value.messages:[]).filter(function(item){return item.direction!=='outgoing';})
            .map(function(item,index){return {source:item,index:index,time:messageTime(item.timestamp)};})
            .sort(function(a,b){return b.time-a.time||a.index-b.index;}).slice(0,3)
            .map(function(row){var item=row.source;return {id:item.id,phone:String(item.phone||'未知号码'),content:String(item.content||''),timestamp:row.time?new Date(row.time).toLocaleString():String(item.timestamp||'设备未提供时间'),direction:item.direction,unread:item.unread===true,complete:item.complete!==false,code:item.complete===false?null:verificationCode(item.content)};});
    }
function lockFeedback(receipts, radio, modem) {
  if (!receipts || !receipts.length) return ''
  if (receipts.some(item => item.modem !== modem)) return '模组已更换，请重新读取配置。'
  if (receipts.some(item => item.receipt.savedVerified !== true)) return '保存已接受，配置回读未确认；先刷新，不要重复提交。'
  if (receipts.every(item => item.operation.mode === 'auto')) return '恢复自动配置回读一致；当前驻留不用于判断自动选网策略。'
  const uncertain = '配置已保存；等待新的驻留信息确认，不会主动断网重连。'
  if (!radio || radio.stale || !Array.isArray(radio.serving)) return uncertain
  let observed = 0, matched = 0
  for (const item of receipts) {
    const { operation, receipt } = item
    if (operation.mode === 'auto' || !Number.isFinite(receipt.acceptedAtMs)) continue
    const rat = operation.rat === 'NR_NSA' ? 'NR' : operation.rat
    const rows = radio.serving.filter(row => row.rat === rat && row.stale !== true && Number.isFinite(row.observedAtMs) && row.observedAtMs >= receipt.acceptedAtMs)
    if (!rows.length) continue
    const entries = operation.entries || []
    if (operation.mode === 'band') {
      const known = rows.filter(row => /^(?:n|B)\d+$/i.test(String(row.band || '')))
      if (!known.length) continue
      observed++
      if (known.every(row => entries.some(entry => Number(entry.band) === Number(String(row.band).slice(1))))) matched++
    } else if (operation.mode === 'cell') {
      const known = rows.filter(row => Number.isInteger(row.channel) && Number.isInteger(row.pci) && row.channelType !== 'center')
      if (!known.length) continue
      observed++
      if (known.some(row => entries.some(entry => row.channel === Number(entry.arfcn) && row.pci === Number(entry.pci) && (row.band ? Number(String(row.band).replace(/^[nB]/i, '')) === Number(entry.band) : true)))) matched++
    }
  }
  if (observed && matched < observed) return '配置已保存；新的驻留信息暂未符合全部目标，请稍后刷新确认。'
  if (matched === receipts.length) return '配置回读一致，新的驻留信息符合所选条件；不代表已验证重启后保持。'
  return uncertain
}
    global.WLMBUIData={version:'1.0.1',band:band,known:known,lockFeedback:lockFeedback,resolveCellDraft:resolveCellDraft,
        verificationCode:verificationCode,messageTime:messageTime,recentSms:recentSms,
        rsrpQuality:function(value){return quality(value,[-80,-90,-100]);},
        rsrqQuality:function(value){return quality(value,[-10,-15,-20]);},
        sinrQuality:function(value){return quality(value,[20,10,0]);},
        rssiQuality:function(value){return quality(value,[-60,-70,-80]);},
        network:function(snapshot) {
            var radio=snapshot.radio||{},serving=radio.stale?[]:(radio.serving||[]).filter(function(row){return !row.stale;}),carriers=radio.stale?[]:(radio.carriers||[]).filter(function(row){return !row.stale;});
            var primary=serving.find(function(row){return row.rat==='NR';})||serving[0]||{};
            var secondary=0;
            return {primary:primary,networkMode:primary.rat ? String(primary.networkMode||primary.rat) : '等待网络数据',pci:show(primary.pci),channel:show(primary.channel),mcc:show(primary.mcc),mnc:show(primary.mnc),lac:show(primary.tac),cid:show(primary.cellId),carrierCount:carriers.length,dlBandwidth:total(carriers,'dlBandwidthMHz'),ulBandwidth:total(carriers,'ulBandwidthMHz'),carrierInfo:carriers.map(function(row,index){return {
                role:row.role,title:row.role==='pcc'?'主载波':row.role==='scc'?'辅助载波 '+(++secondary):'载波 '+(index+1)+' · 角色未报告',sysMode:row.rat||'—',bandShortName:band(row),bandDesc:row.channelType==='center'?'中心频点':row.channelType==='ssb'?'SSB频点':'设备报告',dlFcn:show(row.channel),ulFcn:show(row.ulChannel),dlFreq:frequency(row),ulFreq:show(row.ulFrequencyMHz),dlBandwidth:show(row.dlBandwidthMHz),ulBandwidth:show(row.ulBandwidthMHz)
            };})};
        },
        neighbors:function(snapshot,filter,search) {
            var radio=snapshot.radio||{},rows=radio.neighbors||[],query=String(search||'').trim().toLowerCase();
            return rows.filter(function(row){return (!filter||row.rat===filter)&&(!query||[band(row),row.pci,row.channel].join(' ').toLowerCase().includes(query));}).slice().sort(function(a,b){
                var delta=(known(b.rsrp)?b.rsrp:-Infinity)-(known(a.rsrp)?a.rsrp:-Infinity);
                return (Number.isFinite(delta)?delta:known(a.rsrp)?-1:known(b.rsrp)?1:0)||String(a.rat).localeCompare(String(b.rat))||(a.channel||0)-(b.channel||0)||(a.pci||0)-(b.pci||0);
            }).map(function(row){return {type:row.rat,bandLabel:band(row),pci:show(row.pci),arfcn:show(row.channel),rsrp:show(radio.stale||row.stale?null:row.rsrp),rsrq:show(radio.stale||row.stale?null:row.rsrq),sinr:show(radio.stale||row.stale?null:row.sinr),source:row,
                lockable:!!snapshot.capabilities?.lockCell&&!radio.stale&&!row.stale&&Number.isInteger(row.channel)&&Number.isInteger(row.pci)&&(row.rat==='LTE'||row.rat==='NR')&&(row.rat==='LTE'||row.channelType!=='center')
            };});
        }
    };
})(window);
