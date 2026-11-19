#!/usr/bin/env python3
"""Run the real egui UI in a private Weston compositor and save its framebuffer."""
from pathlib import Path
import argparse, json, os, subprocess, tempfile, time

ROOT=Path(__file__).resolve().parents[1]
def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--scenarios',default='library,discover,animations,settings,settings-audio,settings-general,settings-library,settings-about,import,playlist,remove,filters,context,empty,no-results,loading,error,sound,unavailable')
    parser.add_argument('--sizes',default='1600x1000,1280x800,960x720')
    parser.add_argument('--output',type=Path,default=ROOT/'target/ui-preview')
    parser.add_argument('--no-build',action='store_true')
    parser.add_argument('--audit-actions',action='store_true')
    args=parser.parse_args()
    if not args.no_build:subprocess.run(['cargo','build','--offline','-p','waniengine-egui'],cwd=ROOT,check=True)
    args.output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='wani-ui-') as temp:
        runtime=Path(temp); runtime.chmod(0o700)
        env=dict(os.environ)
        for key in ['DISPLAY','WAYLAND_DISPLAY','DBUS_SESSION_BUS_ADDRESS','WAYLAND_SOCKET'] :env.pop(key,None)
        env.update({'XDG_RUNTIME_DIR':str(runtime),'XDG_CONFIG_HOME':str(runtime/'config'),'XDG_DATA_HOME':str(runtime/'data'),'XDG_STATE_HOME':str(runtime/'state'),'LIBGL_ALWAYS_SOFTWARE':'1','XDG_SESSION_TYPE':'wayland','WINIT_UNIX_BACKEND':'wayland','XDG_CURRENT_DESKTOP':'KDE','DESKTOP_SESSION':'plasma'})
        log=open(args.output/'weston.log','w')
        weston=subprocess.Popen(['weston','--backend=headless','--renderer=gl','--shell=desktop','--width=1920','--height=1200','--socket=wani-ui','--idle-time=0','--no-config'],env=env,stdout=log,stderr=log)
        try:
            deadline=time.monotonic()+10
            while not (runtime/'wani-ui').exists():
                if weston.poll() is not None:raise RuntimeError((args.output/'weston.log').read_text())
                if time.monotonic()>deadline:raise RuntimeError('Private Weston socket timed out')
                time.sleep(.02)
            env['WAYLAND_DISPLAY']='wani-ui'
            subprocess.run([str(ROOT/'target/debug/waniengine-egui'),'--ui-preview','--ui-output-dir',str(args.output),'--ui-scenarios',args.scenarios,'--ui-sizes',args.sizes]+(['--ui-audit-actions'] if args.audit_actions else []),env=env,cwd=ROOT,check=True,timeout=300)
            if args.audit_actions:
                for name in ['action-audit.json','pointer-audit.json']:
                    checks=json.loads((args.output/name).read_text())
                    failures=[check['action'] for check in checks if not check.get('passed')]
                    if not checks or failures:raise RuntimeError(f'{name}: failed actions {failures}')

        finally:
            weston.terminate()
            try:weston.wait(timeout=5)
            except subprocess.TimeoutExpired:weston.kill();weston.wait()
            log.close()

if __name__=='__main__':main()
