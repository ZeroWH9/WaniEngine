#!/usr/bin/env python3
"""Bake the cached FigCraft SVGs into a tintable 4x atlas. Development only."""
from pathlib import Path
from PIL import Image
import io, json, math, subprocess, xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
REFERENCE = ROOT / 'docs/egui-reference'
OUTPUT = ROOT / 'crates/waniengine-egui/assets'
SCALE, PAD, WIDTH = 4, 2.0, 1024

def main():
    atlas = Image.new('RGBA', (WIDTH, 2048))
    entries, x, y, row = {}, 0, 0, 0
    for path in sorted((REFERENCE / 'vectors').glob('*.svg'), key=lambda p: int(p.stem)):
        svg = ET.fromstring(path.read_text())
        _, _, w, h = map(float, svg.attrib['viewBox'].split())
        w, h = max(w, .1), max(h, .1)
        pw, ph = math.ceil((w+2*PAD)*SCALE), math.ceil((h+2*PAD)*SCALE)
        svg.set('viewBox', f'{-PAD} {-PAD} {pw/SCALE} {ph/SCALE}')
        svg.set('width', str(pw)); svg.set('height', str(ph))
        for node in svg.iter():
            for attr in ('stroke', 'fill'):
                if node.get(attr) not in (None, 'none', 'transparent'):
                    node.set(attr, '#FFFFFF')
        rendered = subprocess.run(['rsvg-convert','--width',str(pw),'--height',str(ph)], input=ET.tostring(svg), capture_output=True, check=True).stdout
        tile = Image.open(io.BytesIO(rendered)).convert('RGBA')
        if x+pw > WIDTH: x, y, row = 0, y+row+4, 0
        atlas.paste(tile,(x,y))
        entries[path.stem] = {'rect':[x,y,pw,ph], 'size':[w,h], 'pad':PAD}
        x += pw+4; row=max(row,ph)
    height=y+row
    atlas=atlas.crop((0,0,WIDTH,height))
    atlas.save(OUTPUT/'figcraft-icons.png')
    (OUTPUT/'figcraft-icons.json').write_text(json.dumps({'width':WIDTH,'height':height,'scale':SCALE,'icons':entries}, separators=(',',':'))+'\n')
    print(f'{len(entries)} SVGs -> {WIDTH}x{height} atlas')

if __name__ == '__main__': main()
