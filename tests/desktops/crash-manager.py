#!/usr/bin/env python3
"""Crash only the explicitly started manager in a disposable desktop session."""
import json, os, signal, subprocess, sys, time
from pathlib import Path

root=Path(sys.argv[1]).resolve();pid=int(sys.argv[2])
assert root.name.startswith('wani-backend-check.') and root.parent==Path('/tmp')
assert os.environ['WANIENGINE_ISOLATED_TEST_ROOT']==str(root)
deadline=time.monotonic()+30
while not (root/'crash-ready').exists():
    assert time.monotonic()<deadline,(root/'crash-manager.log').read_text()
    os.kill(pid,0);time.sleep(.05)
assert int((root/'crash-ready').read_text())==pid

def children(parent):
    result=set()
    for task in Path(f'/proc/{parent}/task').glob('*'):
        try:result.update(map(int,(task/'children').read_text().split()))
        except FileNotFoundError:pass
    return result
owned=set();pending=[pid]
while pending:
    current=pending.pop()
    for child in children(current)-owned:owned.add(child);pending.append(child)
def cmd(pid):
    try:return Path(f'/proc/{pid}/cmdline').read_bytes().replace(b'\0',b' ').decode(errors='replace')
    except OSError:return '?'
names={p:cmd(p) for p in owned}
assert len(owned)>=3,'renderer, video decoder and PCM decoder must be present'
os.kill(pid,signal.SIGKILL)
deadline=time.monotonic()+5
def alive(pid):
    try:return Path(f'/proc/{pid}/stat').read_text().rsplit(') ',1)[1][0] not in 'ZX'
    except FileNotFoundError:return False
while any(map(alive,owned)) and time.monotonic()<deadline:time.sleep(.05)
assert not any(map(alive,owned)),f'owned processes survived: {[(p,names[p]) for p in owned if alive(p)]}'
time.sleep(1.5)
path=root/'after-crash.png'
subprocess.run(['spectacle','-f','-b','-n','-o',str(path)],check=True,timeout=20)
pixel=tuple(subprocess.check_output(['ffmpeg','-nostdin','-v','error','-i',str(path),'-vf','crop=1:1:640:360','-frames:v','1','-f','rawvideo','-pix_fmt','rgb24','pipe:1'],timeout=10))
assert len(pixel)==3 and 120<=pixel[0]<=135 and pixel[1]<5 and 120<=pixel[2]<=135,pixel
report={'abrupt_manager_exit':True,'owned_processes_reaped':sorted(owned),'fallback_visible':True}
(root/'crash-report.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report))
