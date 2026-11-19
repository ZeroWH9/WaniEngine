#!/usr/bin/env python3
"""Summarize a bounded memory trace; missing GPU counters are never zero VRAM."""
import argparse, json, statistics
from collections import defaultdict
from pathlib import Path

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('trace',type=Path)
parser.add_argument('--warmup',type=float,default=900)
parser.add_argument('--output',type=Path)
args=parser.parse_args()
items=[]
for path in [args.trace.with_suffix(args.trace.suffix+'.1'),args.trace]:
    if path.is_file():
        for line in path.read_text().splitlines():
            try:items.append(json.loads(line))
            except json.JSONDecodeError:pass  # an in-progress final line
samples=sorted((item for item in items if item.get('event')=='sample'),key=lambda item:item['elapsed_s'])
if not samples:raise SystemExit('No memory samples')
series=defaultdict(list);born_audio=set();max_renderers=defaultdict(int)
for sample in samples:
    for output,pids in sample.get('video_pids_by_output',{}).items():
        max_renderers[output]=max(max_renderers[output],len(pids))
    for process in sample.get('processes',[]):
        key=(process['pid'],process['start'])
        if 'f32le' in process.get('command',[]):born_audio.add(key)
        series[key].append((sample['elapsed_s'],process))

def median_window(values,first):
    edge=values[0][0]+300 if first else values[-1][0]-300
    window=[row[1]['ram_kib'].get('VmRSS_kib') for row in values if (row[0]<=edge if first else row[0]>=edge)]
    return statistics.median([v for v in window if v is not None]) if any(v is not None for v in window) else None

result=[]
for (pid,start),all_values in series.items():
    values=[v for v in all_values if v[0]>=args.warmup and v[1].get('ram_kib',{}).get('VmRSS_kib') is not None]
    if len(values)<2:continue
    times=[v[0] for v in values];rss=[v[1]['ram_kib']['VmRSS_kib'] for v in values]
    mean_t=statistics.mean(times);mean_r=statistics.mean(rss)
    denominator=sum((t-mean_t)**2 for t in times)
    slope=sum((t-mean_t)*(r-mean_r) for t,r in zip(times,rss))/denominator if denominator else 0
    fds=[p['fd_count'] for _,p in values if p.get('fd_count') is not None]
    final=values[-1][1]
    result.append(dict(pid=pid,start=start,role=final['role'],comm=final['comm'],
        observed_seconds=round(times[-1]-times[0],3),rss_kib_min=min(rss),rss_kib_max=max(rss),
        rss_kib_first_window=median_window(values,True),rss_kib_last_window=median_window(values,False),
        rss_linear_mib_per_hour=round(slope*3600/1024,4),
        fd_min=min(fds) if fds else None,fd_max=max(fds) if fds else None,
        gpu_counters_available=any(p.get('gpu_memory_available',False) for _,p in values),
        shared_bytes_max=max(sum(v['size_bytes'] for v in p.get('shared_files',[])) for _,p in values)))
report=dict(trace=str(args.trace),complete=any(item.get('event')=='trace-end' for item in items),
    elapsed_seconds=samples[-1]['elapsed_s'],warmup_seconds=args.warmup,sample_count=len(samples),
    max_renderers_by_output=dict(max_renderers),observed_audio_decoder_instances=len(born_audio),
    processes=sorted(result,key=lambda p:(p['role'],p['pid'])),
    hardware_vram_qualified=False,
    note='Slope is a descriptive fit, not an automatic leak verdict. Compare medians and GC/reclaim behaviour; post-stop cleanup is verified separately.')
text=json.dumps(report,indent=2)+'\n'
if args.output:
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(text)
print(text,end='')
