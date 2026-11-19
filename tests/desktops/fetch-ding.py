#!/usr/bin/env python3
"""Fetch pinned official DING sources solely for the disposable desktop matrix."""
import argparse, hashlib, io, tarfile, urllib.request
from pathlib import Path

URL='https://gitlab.com/rastersoft/desktop-icons-ng/-/archive/51.1.1/desktop-icons-ng-51.1.1.tar.gz'
SHA256='0d4dea8e97eea5a4391f3019a0d8cc5b4e3b2db59b12b262d89e0f52b61162af'
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--archive',type=Path,help='use an already downloaded official archive')
args=parser.parse_args()
data=args.archive.read_bytes() if args.archive else urllib.request.urlopen(URL,timeout=30).read(4*1024*1024)
if hashlib.sha256(data).hexdigest()!=SHA256: raise SystemExit('DING archive checksum differs from the reviewed version')
destination=Path(__file__).resolve().parents[2]/'target/desktop-matrix/vendor'
destination.mkdir(parents=True,exist_ok=True)
with tarfile.open(fileobj=io.BytesIO(data)) as archive: archive.extractall(destination,filter='data')
print(destination/'desktop-icons-ng-51.1.1')
