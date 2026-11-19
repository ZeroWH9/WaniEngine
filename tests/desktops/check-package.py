#!/usr/bin/env python3
"""Install an actual candidate bundle into temporary paths, without host integration."""
import argparse, importlib.util, json, subprocess, tempfile
from pathlib import Path

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('bundle',type=Path)
parser.add_argument('--output',type=Path,default=Path('target/package-check/report.json'))
args=parser.parse_args();bundle=args.bundle.resolve()
source=Path(__file__).resolve().parents[2]/'scripts/install-waniengine.py'
spec=importlib.util.spec_from_file_location('installer',source);installer=importlib.util.module_from_spec(spec);spec.loader.exec_module(installer)
manifest=installer.verify_bundle(bundle)
linked=subprocess.check_output(['ldd',str(bundle/manifest['executable'])],text=True,timeout=10)
assert 'not found' not in linked and 'libQt' not in linked and 'libqml' not in linked
for name in ['LICENSE','OFL-Inter.txt','LICENCE-Lucide.txt','LICENSE-Atlas-icons.txt']:
    assert (bundle/'share/licenses/waniengine'/name).is_file(),name
with tempfile.TemporaryDirectory(prefix='wani-package-check-') as folder:
    root=Path(folder)
    paths={key:root/key for key in ['home','prefix','data','config','desktop','library','state','backup']}
    paths['launcher']=root/'bin/waniengine'
    paths['library'].mkdir();scene=paths['library']/'scene.json'
    scene.write_text(json.dumps({'assets':[{'url':'file:///preserved/original.png'}],'settings':{}}))
    first=installer.install(bundle,paths,integration=False)
    state=json.loads(scene.read_text());state['testMarker']='preserved';scene.write_text(json.dumps(state))
    second=installer.install(bundle,paths,integration=False)
    assert first['current']==second['current']
    assert json.loads(scene.read_text())['testMarker']=='preserved'
    assert (paths['prefix']/'current'/manifest['executable']).is_file()
    installer.uninstall(paths,integration=False)
    assert scene.is_file() and json.loads(scene.read_text())['testMarker']=='preserved'
    assert not paths['launcher'].exists()
report={'real_bundle':True,'qt_linked':False,'licenses':True,'install':True,'idempotent_update':True,'uninstall_preserves_library':True,'host_integration':False}
args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
