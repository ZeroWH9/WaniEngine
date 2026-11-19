// SPDX-License-Identifier: BSD-3-Clause
// One observer per manager: loading/unloading one script per video can reuse
// KWin script object IDs while another script is still registered.
const p = __WANI_PARAMS__;
let members = [];
let reported = {};
let requesting = false;
let ticks = 0;
let translucent = {};
function restoreOpacity(needed) {
    for (const id of Object.keys(translucent)) {
        if (needed[id]) continue;
        const entry=translucent[id];
        try { if (entry.window.opacity===0.99999) entry.window.opacity=entry.original; } catch(error) {}
        delete translucent[id];
    }
}
const timer = new QTimer();
timer.interval = 50;
function reconcile(removed) {
    const windows = workspace.windowList();
    const needed = {};
    for (let i=0;i<members.length;++i) {
        const pid=Number(members[i][0]);
        const token=String(members[i][1]);
        let renderer=null;
        for(let j=0;j<windows.length;++j) if(windows[j]!==removed && Number(windows[j].pid)===pid) renderer=windows[j];
        let ready=false;
        if(renderer && typeof workspace.constrain === 'function') {
            const r=renderer.frameGeometry;
            for(let j=0;j<windows.length;++j) {
                const w=windows[j];
                if(!w || w===removed || !w.desktopWindow || members.some(function(member){return Number(member[0])===Number(w.pid);})) continue;
                const g=w.frameGeometry;
                if((w.output && renderer.output && w.output===renderer.output)
                    || (Math.abs(g.x-r.x)<1 && Math.abs(g.y-r.y)<1 && Math.abs(g.width-r.width)<1 && Math.abs(g.height-r.height)<1)) {
                    workspace.constrain(renderer,w);
                    const id=String(w.internalId);needed[id]=true;
                    // DesktopView's originally opaque Wayland region can
                    // survive changing its clear colour. Disable opaque-region
                    // culling in KWin, retaining effectively full icon opacity.
                    // Qt's Wayland plugin cannot set QWindow.opacity itself.
                    if(!translucent[id] && w.opacity===1) {
                        translucent[id]={window:w,original:w.opacity};w.opacity=0.99999;
                    }
                    ready=true;
                }
            }
        }
        if(reported[token]!==ready) {reported[token]=ready;callDBus(p.service,'/Bridge','org.waniengine.Bridge','Report',token,ready);}
    }
    restoreOpacity(needed);
    if(members.length===0) callDBus(p.service,'/Bridge','org.waniengine.Bridge','Idle');
}
function refresh() {
    // Keep owner-loss supervision running even when a failed asynchronous
    // Renderers call never invokes its callback.
    if(++ticks%20===0) callDBus('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','NameHasOwner',p.service,function(alive){
        if(!alive){restoreOpacity({});timer.stop();callDBus('org.kde.KWin','/Scripting','org.kde.kwin.Scripting','unloadScript',p.label);}
    });
    if(requesting) return;
    requesting=true;
    callDBus(p.service,'/Bridge','org.waniengine.Bridge','Renderers',function(value){
        requesting=false;try{members=JSON.parse(value);}catch(error){members=[];}
        const keep={};for(let i=0;i<members.length;++i) {const token=String(members[i][1]);if(reported[token]!==undefined)keep[token]=reported[token];}
        reported=keep;reconcile(null);
    });
}
workspace.windowAdded.connect(function(){reconcile(null);});
workspace.windowRemoved.connect(function(window){reconcile(window);});
workspace.screensChanged.connect(function(){reconcile(null);});
timer.timeout.connect(refresh);timer.start();refresh();
