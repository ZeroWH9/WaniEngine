#!/usr/bin/env python3
"""Create review artifacts. Pixel error is diagnostic, never a visual acceptance claim."""
from pathlib import Path
from PIL import Image, ImageChops, ImageEnhance, ImageStat, ImageDraw, ImageFont
import argparse, json

ROOT=Path(__file__).resolve().parents[1]

def geometry_report(renders):
    document=json.loads((ROOT/'docs/egui-reference/document.json').read_text())
    manifest=json.loads((ROOT/'docs/egui-reference/manifest.json').read_text())
    def walk(node):
        yield node
        for child in node.get('children',[]):yield from walk(child)
    nodes={node['id']:node for page in document['pages'] for node in walk(page)}
    result={}
    for scenario in ['library','discover','animations','settings','import','playlist','remove']:
        file=renders/f'{scenario}-1600x1000-geometry.json'
        if not file.exists():continue
        dialog=scenario in ['import','playlist','remove']
        frame=nodes[manifest['frames']['dialogs-and-states' if dialog else scenario]]
        origin=frame['absoluteBounds']
        expected={node['name']:node for node in walk(frame) if node.get('type')=='FRAME'}
        samples=json.loads(file.read_text());checks={}
        for name,sample in samples.items():
            if name not in expected:continue
            node=expected[name];bounds=node['absoluteBounds']
            target=[bounds['x']-origin['x'],bounds['y']-origin['y'],node['width'],node['height']]
            if dialog:
                target[0]=(1600-target[2])/2;target[1]=(1000-target[3])/2
            delta=[actual-wanted for actual,wanted in zip(sample['bounds'],target)]
            error=max(abs(value) for value in delta)
            checks[name]={'source_node':node['id'],'expected':target,'actual':sample['bounds'],'delta':delta,'max_error':error,'within_1_logical_px':error<=1}
        result[scenario]=checks
    return result

def compare(reference,actual,out,name):
    def flatten(image):
        background=Image.new('RGBA',image.size,(7,6,11,255))
        return Image.alpha_composite(background,image.convert('RGBA')).convert('RGB')
    reference=flatten(reference);actual=flatten(actual)
    if actual.size!=reference.size:raise ValueError(f'{name}: expected {reference.size}, got {actual.size}')
    w,h=reference.size
    side=Image.new('RGB',(w*2,h));side.paste(reference,(0,0));side.paste(actual,(w,0));side.save(out/f'{name}-side-by-side.png')
    Image.blend(reference,actual,.5).save(out/f'{name}-overlay.png')
    diff=ImageChops.difference(reference,actual)
    ImageEnhance.Contrast(diff).enhance(3).save(out/f'{name}-difference.png')
    return {'mean_absolute_channel_error':ImageStat.Stat(diff).mean,'status':'requires visual review','size':[w,h]}

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--renders',type=Path,default=ROOT/'target/ui-preview')
    parser.add_argument('--check',action='store_true',help='Fail on missing or inaccurate principal geometry')
    args=parser.parse_args();out=args.renders/'comparison';out.mkdir(exist_ok=True)
    report={}
    for name in ['library','discover','animations','settings']:
        reference=Image.open(ROOT/f'docs/egui-reference/frames/{name}.png')
        actual=Image.open(args.renders/f'{name}-1600x1000.png')
        report[name]=compare(reference,actual,out,name)
    dialogs=Image.open(ROOT/'docs/egui-reference/frames/dialogs-and-states.png')
    for name,source,actual in [
        ('import',(40,140,760,620),(440,260,1160,740)),
        ('playlist',(800,140,1520,700),(440,220,1160,780)),
        ('remove',(40,650,400,870),(620,390,980,610)),
        ('filters',(420,650,760,950),(938,520,1278,820)),
        ('context',(800,730,1060,970),(1300,156,1560,396)),
    ]:
        rendered=Image.open(args.renders/f'{name}-1600x1000.png')
        report[name]=compare(dialogs.crop(source),rendered.crop(actual),out,name)
    # Same-size contact sheets help spot clipping and missing content across all scenarios.
    for size in ['1600x1000','1280x800','960x720']:
        paths=sorted(args.renders.glob(f'*-{size}.png'))
        thumb_width=480;thumb_height=round(int(size.split('x')[1])*thumb_width/int(size.split('x')[0]))
        sheet=Image.new('RGB',(thumb_width*3,(thumb_height+28)*((len(paths)+2)//3)),(12,10,17));draw=ImageDraw.Draw(sheet)
        for i,path in enumerate(paths):
            x=(i%3)*thumb_width;y=(i//3)*(thumb_height+28)
            draw.text((x+8,y+8),path.stem,fill=(201,195,214))
            sheet.paste(Image.open(path).convert('RGB').resize((thumb_width,thumb_height)),(x,y+28))
        sheet.save(out/f'scenarios-{size}.jpg',quality=90)
    overview=Image.new('RGB',(1680,1126),(7,6,11));draw=ImageDraw.Draw(overview)
    font=ImageFont.truetype(str(ROOT/'crates/waniengine-egui/assets/fonts/Inter-Regular.ttf'),12)
    for i,name in enumerate(['library','discover','animations','settings']):
        x=(i%2)*840;y=(i//2)*563
        draw.text((x+20,y+14),name.title()+' · egui render',font=font,fill=(201,195,214))
        source=Image.open(args.renders/f'{name}-1600x1000.png').convert('RGBA')
        background=Image.new('RGBA',source.size,(7,6,11,255));background.alpha_composite(source)
        overview.paste(background.convert('RGB').resize((840,525),Image.Resampling.LANCZOS),(x,y+38))
    overview.save(out/'egui-overview.jpg',quality=95)
    (out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    geometry=geometry_report(args.renders)
    (out/'geometry-report.json').write_text(json.dumps(geometry,indent=2)+'\n')
    checks=[check for scene in geometry.values() for check in scene.values()]
    passed=sum(check['within_1_logical_px'] for check in checks)
    print(f'Principal geometry: {passed}/{len(checks)} within 1 logical px')
    if args.check and (len(geometry)!=7 or len(checks)<43 or passed!=len(checks)):
        raise SystemExit('Principal geometry failed; inspect geometry-report.json')
    print(out)

if __name__=='__main__':main()
