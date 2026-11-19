#!/usr/bin/env python3
"""Exercise the live overlay protocol inside test-wayland-runtime.sh's session."""
import json
import os
import select
import subprocess
import sys
import time
from pathlib import Path

root, binary, output = Path(sys.argv[1]), sys.argv[2], sys.argv[3]


class Client:
    def __init__(self, image, *options):
        self.log = open(root / (Path(image).stem + '-control.err'), 'wb')
        self.process = subprocess.Popen(
            [binary, '--actor', '--control-stdin', '--image', str(image), '--output', output,
             '--x', '0.5', '--y', '0.5', '--scale', '200', *options],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, bufsize=0,
        )
        self.buffer = b''
        self.event('configured')

    def event(self, name, request_id=None):
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            if b'\n' not in self.buffer:
                remaining = max(0, deadline - time.monotonic())
                ready, _, _ = select.select([self.process.stdout], [], [], remaining)
                if not ready:
                    break
                chunk = os.read(self.process.stdout.fileno(), 4096)
                if not chunk:
                    raise AssertionError(f'runtime exited early: {self.process.poll()}')
                self.buffer += chunk
                continue
            line, self.buffer = self.buffer.split(b'\n', 1)
            event = json.loads(line)
            if event.get('event') == name and (request_id is None or event.get('id') == request_id):
                return event
            if event.get('event') == 'error':
                raise AssertionError(event)
        raise AssertionError(f'timed out waiting for {name} {request_id}')

    def send(self, **message):
        self.process.stdin.write((json.dumps(message) + '\n').encode())

    def close(self):
        if self.process.stdin and not self.process.stdin.closed:
            self.process.stdin.close()
        try:
            code = self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
            raise AssertionError('runtime survived the control pipe closing')
        self.log.close()
        assert code == 0, code

    def abort(self):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait()
        self.log.close()


def sample(name, x, y):
    path = root / f'control-{name}.png'
    subprocess.run(['spectacle', '-b', '-n', '-o', str(path)], timeout=20,
                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
    pixels = subprocess.run(['ffmpeg', '-nostdin', '-v', 'error', '-i', str(path), '-f',
                             'rawvideo', '-pix_fmt', 'rgb24', 'pipe:1'], timeout=20,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout
    assert len(pixels) == 1600 * 600 * 3
    offset = (y * 1600 + x) * 3
    return tuple(pixels[offset:offset + 3])


def green(color):
    assert color[1] > 200 and color[0] < 60 and color[2] < 60, color


def blue(color):
    assert color[2] > 200 and color[0] < 60 and color[1] < 60, color


def red(color):
    assert color[0] > 220 and color[1] < 60 and color[2] < 60, color


client = Client(root / 'actor.png')
try:
    client.send(action='update', id=1, x=0.25, scale=100.0)
    client.event('applied', 1)
    green(sample('moved', 1000, 270))
    red(sample('old-position', 1200, 240))

    client.send(action='update', id=2, x=2.0)
    error = client.event('error', 2)
    assert 'x must be' in error['message'], error
    client.send(action='update', id=3, opacity=0.0)
    client.event('applied', 3)
    red(sample('transparent', 1000, 270))
    client.send(action='update', id=4, opacity=100.0)
    client.event('applied', 4)
    green(sample('restored', 1000, 270))
    client.close()
    red(sample('closed', 1000, 270))
except BaseException:
    client.abort()
    raise

# The 4-second GIF frame lasts 1.33 seconds at 300% speed. Verify that
# pausing freezes playback and resuming advances from the paused position.
client = Client(root / 'actor.gif', '--paused', '--speed', '300')
try:
    time.sleep(1.7)
    green(sample('paused-first', 1200, 240))
    client.send(action='update', id=5, playing=True)
    client.event('applied', 5)
    time.sleep(1.5)
    blue(sample('resumed', 1200, 240))
    client.send(action='update', id=6, playing=False)
    client.event('applied', 6)
    time.sleep(1.6)
    blue(sample('paused-second', 1200, 240))
    client.send(action='stop', id=7)
    client.event('stopped', 7)
    client.close()
except BaseException:
    client.abort()
    raise

print('Live overlay updates, pause/resume and owner shutdown passed.')
