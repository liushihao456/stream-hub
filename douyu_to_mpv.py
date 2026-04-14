#!/usr/bin/env python3
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent
EXTRACTOR = ROOT / 'douyu_extract_playurl.js'


def extract_stream(target):
    proc = subprocess.run(
        ['node', str(EXTRACTOR), target],
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(proc.stdout), proc.stdout


def write_playlist(title, urls):
    safe_title = title or 'Douyu Live'
    content = ['#EXTM3U']
    for index, url in enumerate(urls):
        name = safe_title if index == 0 else f'{safe_title} - Backup {index}'
        content.append(f'#EXTINF:-1,{name}')
        content.append(url)

    path = Path(tempfile.gettempdir()) / 'stream-hub-douyu.m3u'
    path.write_text('\n'.join(content) + '\n', encoding='utf-8')
    return path


def build_mpv_cmd(playlist_path, title):
    mpv_bin = os.environ.get('STREAM_HUB_MPV_PATH', '').strip() or 'mpv'
    media_title = (title or 'Douyu Live').replace('"', "''")
    return [
        mpv_bin,
        '--ytdl=no',
        '--stream-lavf-o=reconnect_streamed=yes',
        f'--force-media-title={media_title}',
        str(playlist_path),
    ]


def main():
    if len(sys.argv) < 2:
        print('Usage: python3 douyu_to_mpv.py <room-id-or-url> [--print-only]', file=sys.stderr)
        raise SystemExit(1)

    target = sys.argv[1]
    print_only = '--print-only' in sys.argv[2:]

    data, raw_output = extract_stream(target)

    if data.get('offline'):
        print('Room is offline:', data.get('log', ''), file=sys.stderr)
        raise SystemExit(2)

    urls = data.get('urls') or ([data['url']] if data.get('url') else [])
    if not urls:
        print(raw_output, file=sys.stderr)
        raise SystemExit(3)

    if print_only:
        print(urls[0])
        return

    playlist_path = write_playlist(data.get('title', ''), urls)
    mpv_cmd = build_mpv_cmd(playlist_path, data.get('title', ''))
    print('Running:', ' '.join(mpv_cmd), file=sys.stderr)
    raise SystemExit(subprocess.run(mpv_cmd).returncode)


if __name__ == '__main__':
    main()
