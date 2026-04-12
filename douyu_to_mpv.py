#!/usr/bin/env python3
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
EXTRACTOR = ROOT / 'douyu_extract_playurl.js'


def main():
    if len(sys.argv) < 2:
        print('Usage: python3 douyu_to_mpv.py <room-id-or-url> [--print-only]', file=sys.stderr)
        raise SystemExit(1)

    target = sys.argv[1]
    print_only = '--print-only' in sys.argv[2:]

    proc = subprocess.run(
        ['node', str(EXTRACTOR), target],
        capture_output=True,
        text=True,
        check=True,
    )
    data = json.loads(proc.stdout)

    if data.get('offline'):
        print('Room is offline:', data.get('log', ''), file=sys.stderr)
        raise SystemExit(2)

    url = data.get('url')
    if not url:
        print(proc.stdout, file=sys.stderr)
        raise SystemExit(3)

    if print_only:
        print(url)
        return

    mpv_cmd = ['mpv', url]
    print('Running:', ' '.join(mpv_cmd), file=sys.stderr)
    raise SystemExit(subprocess.run(mpv_cmd).returncode)


if __name__ == '__main__':
    main()
