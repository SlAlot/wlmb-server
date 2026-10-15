// 教程只处理本地外观、章节和地址复制，不读取账户或请求设备接口。
(function(){
    'use strict';
    var root=document.getElementById('wlmb-panel'),button=document.getElementById('guide-theme'),appearance='auto';
    var selected=new URLSearchParams(location.search).get('appearance');
    try{appearance=sessionStorage.getItem('wlmb-server.appearance')||'auto';}catch(_){}
    if(['auto','light','dark'].includes(selected))appearance=selected;
    if(!['auto','light','dark'].includes(appearance))appearance='auto';
    function paint(){root.setAttribute('data-np-appearance',appearance);button.textContent='外观：'+{auto:'自动',light:'浅色',dark:'深色'}[appearance];}
    paint();button.hidden=false;
    button.addEventListener('click',function(){appearance=appearance==='auto'?'light':appearance==='light'?'dark':'auto';paint();try{sessionStorage.setItem('wlmb-server.appearance',appearance);}catch(_){}});
    var address=document.getElementById('guide-address'),copy=document.getElementById('guide-copy'),feedback=document.getElementById('guide-feedback');
    var hostname=location.hostname;
    // 地址取当前页面主机，固定使用插件端口；不把登录页地址或凭据带入教程。
    if(hostname&&location.protocol!=='file:'){
        address.textContent=(hostname.includes(':')&&!hostname.startsWith('[')?'['+hostname+']':hostname)+':1117';copy.hidden=false;
    }
    function fallback(){
        var selection=getSelection(),range=document.createRange();range.selectNodeContents(address);selection.removeAllRanges();selection.addRange(range);
        var ok=false;try{ok=document.execCommand('copy');}catch(_){}
        if(ok)selection.removeAllRanges();return ok;
    }
    copy.addEventListener('click',async function(){
        copy.disabled=true;
        try{
            var ok=false;
            if(navigator.clipboard&&window.isSecureContext){try{await navigator.clipboard.writeText(address.textContent);ok=true;}catch(_){}}
            if(!ok)ok=fallback();
            feedback.textContent=ok?'地址已复制，登录时请确认是你的路由器管理地址。':'未能自动复制，请长按或选中上方地址复制。';
        }catch(_){feedback.textContent='请手动复制上方地址。';}
        finally{copy.disabled=false;}
    });
    function chapter(){
        var id;try{id=decodeURIComponent(location.hash.slice(1));}catch(_){return;}
        var section=document.getElementById(id);if(!section)return;
        if(section.tagName==='DETAILS')section.open=true;
    }
    chapter();window.addEventListener('hashchange',chapter);
})();
