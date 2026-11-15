"""Designed game sound effects as references: index the Sonniss GDC Game Audio bundles and fetch single files.

    gamesfx.py index [urls.txt] [index.json]          read each zip part's directory over HTTP ranges (a few MB each)
    gamesfx.py find [index.json] <regex> [--max 40]   list matching entries
    gamesfx.py fetch [index.json] [out dir] <entry>... fetch only those files (one range request each)

Home: $JUDGE_REFS/gamesfx (default /root/work/haa/refs/gamesfx) holds index.json and the fetched files, which
refs.json names as gamesfx/<file>. gamesfx_urls.txt (next to this script) lists every bundle part: the index is
rebuilt from it, and only the files the references name are fetched.

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

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.join(os.environ.get("JUDGE_REFS", "/root/work/haa/refs"), "gamesfx")
URLS = os.path.join(HERE, "gamesfx_urls.txt")
INDEX = os.path.join(HOME, "index.json")
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
    cmd, args = sys.argv[1], sys.argv[2:]
    if cmd == "index":
        index(args[0] if args else URLS, args[1] if len(args) > 1 else INDEX)
    elif cmd == "find":
        mx = 40
        if "--max" in args:
            k = args.index("--max")
            mx, args = int(args[k + 1]), args[:k] + args[k + 2:]
        find(args[0] if len(args) > 1 else INDEX, args[-1], mx)
    elif cmd == "fetch":
        if args and args[0].endswith(".json"):
            idx, args = args[0], args[1:]
        else:
            idx = INDEX
        out = HOME
        if args and os.path.isdir(args[0]):
            out, args = args[0], args[1:]
        fetch(idx, out, args)
