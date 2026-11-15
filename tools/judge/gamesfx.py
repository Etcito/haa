"""Designed game sound effects as references: index the Sonniss GDC Game Audio bundles and fetch single files.

    gamesfx.py index <urls.txt> <index.json>          read each zip part's directory over HTTP ranges (a few MB each)
    gamesfx.py find <index.json> <regex> [--max 40]   list matching entries
    gamesfx.py fetch <index.json> <out dir> <entry>... fetch only those files (one range request each)

The bundles are royalty-free game audio from professional libraries (licence: media use; no redistribution as
sound effects; no AI/ML training - judging against them is inference, not training). Local only: nothing fetched
is committed or deployed. One connection at a time, niced.
"""
import json
import os
import re
import sys
import time

os.nice(15)
from remotezip import RemoteZip  # noqa: E402

HEADERS = {"User-Agent": "Mozilla/5.0 (X11; Linux x86_64)", "Referer": "https://sonniss.com/gameaudiogdc"}


def index(urls_path, out):
    data = json.load(open(out)) if os.path.exists(out) else {}
    for url in [u.strip() for u in open(urls_path) if u.strip()]:
        if url in data:
            continue
        try:
            with RemoteZip(url, headers=HEADERS) as z:
                data[url] = [[i.filename, i.file_size] for i in z.infolist() if i.filename.lower().endswith((".wav", ".flac", ".aif", ".aiff", ".mp3", ".ogg"))]
        except Exception as e:  # a part that cannot be read is retried next time
            print(f"[gamesfx] {url}: {e}", file=sys.stderr)
            time.sleep(10)
            continue
        json.dump(data, open(out, "w"))
        time.sleep(3)
        print(f"[gamesfx] {len(data[url])} files in {url.rsplit('/', 1)[-1]}", file=sys.stderr, flush=True)


def find(idx, rx, mx=40):
    data = json.load(open(idx))
    r = re.compile(rx, re.I)
    n = 0
    for url, files in data.items():
        for name, size in files:
            if r.search(name):
                print(f"{size // 1024:7d} KB  {name}")
                n += 1
                if n >= mx:
                    return


def fetch(idx, out, entries):
    data = json.load(open(idx))
    where = {name: url for url, files in data.items() for name, _ in files}
    os.makedirs(out, exist_ok=True)
    for e in entries:
        url = where.get(e)
        if not url:
            print(f"[gamesfx] not in the index: {e}", file=sys.stderr)
            continue
        dst = os.path.join(out, os.path.basename(e))
        if os.path.exists(dst):
            continue
        with RemoteZip(url, headers=HEADERS) as z:
            with z.open(e) as src, open(dst, "wb") as f:
                f.write(src.read())
        print(f"[gamesfx] {dst}", file=sys.stderr)


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "index":
        index(sys.argv[2], sys.argv[3])
    elif cmd == "find":
        mx = int(sys.argv[sys.argv.index("--max") + 1]) if "--max" in sys.argv else 40
        find(sys.argv[2], sys.argv[3], mx)
    elif cmd == "fetch":
        fetch(sys.argv[2], sys.argv[3], sys.argv[4:])
