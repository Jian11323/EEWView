# -*- coding: utf-8 -*-
"""把本地 update/manifest.json 推到 Gitee（发版后同步清单）。

用法：
  python tools/publish_gitee_manifest.py

令牌：仓库根 `.gitee_token`（一行私钥；已 gitignore / 打包排除）。
也可设环境变量 GITEE_ACCESS_TOKEN。
"""
from __future__ import annotations

import base64
import json
import os
import ssl
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OWNER = "jian0786"
REPO = "starquake"
BRANCH = "master"
REL = "update/manifest.json"
CTX = ssl.create_default_context()


def load_token() -> str:
    env = (os.environ.get("GITEE_ACCESS_TOKEN") or "").strip()
    if env:
        return env
    path = ROOT / ".gitee_token"
    if path.is_file():
        tok = path.read_text(encoding="utf-8").strip()
        if tok:
            return tok
    raise SystemExit("缺少令牌：请写仓库根 .gitee_token，或设 GITEE_ACCESS_TOKEN")


def api(method: str, path: str, token: str, data: dict | None = None) -> tuple[int, object]:
    q = urllib.parse.urlencode({"access_token": token})
    url = f"https://gitee.com/api/v5{path}"
    url = url + ("&" if "?" in url else "?") + q
    body = None
    headers = {"User-Agent": "StarQuake-publish-gitee-manifest"}
    if data is not None:
        payload = dict(data)
        payload["access_token"] = token
        body = urllib.parse.urlencode(payload).encode("utf-8")
        headers["Content-Type"] = "application/x-www-form-urlencoded"
    req = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, context=CTX, timeout=60) as resp:
            raw = resp.read()
            code = resp.status
    except urllib.error.HTTPError as e:
        raw = e.read()
        code = e.code
    if not raw:
        return code, None
    try:
        return code, json.loads(raw.decode("utf-8"))
    except Exception:
        return code, raw.decode("utf-8", "replace")


def main() -> int:
    token = load_token()
    local = ROOT / REL
    if not local.is_file():
        print(f"missing {local}", file=sys.stderr)
        return 1
    content = local.read_bytes()
    b64 = base64.b64encode(content).decode("ascii")

    code, info = api("GET", f"/repos/{OWNER}/{REPO}/contents/{REL}?ref={BRANCH}", token)
    sha = info.get("sha") if code == 200 and isinstance(info, dict) else None
    data = {
        "content": b64,
        "message": "chore: sync update/manifest.json",
        "branch": BRANCH,
    }
    if sha:
        data["sha"] = sha
        method = "PUT"
    else:
        method = "POST"
    code2, out = api(method, f"/repos/{OWNER}/{REPO}/contents/{REL}", token, data)
    if code2 not in (200, 201):
        print(f"fail HTTP {code2}: {out}", file=sys.stderr)
        return 1

    raw_url = f"https://gitee.com/{OWNER}/{REPO}/raw/{BRANCH}/{REL}"
    req = urllib.request.Request(raw_url, headers={"User-Agent": "StarQuake-publish-gitee-manifest"})
    with urllib.request.urlopen(req, context=CTX, timeout=30) as resp:
        body = resp.read()
    m = json.loads(body.decode("utf-8"))
    print(f"OK {method} {REL}@{BRANCH}")
    print(f"raw {raw_url}")
    print(f"latest_version {m.get('latest_version')}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
