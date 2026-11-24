"""End-to-end attachment proof using disposable profiles and a loopback Synapse.

Build the web client first. From the checkout:
  uv run --python 3.13 --with matrix-synapse --with requests --with websocket-client \
    --with playwright --with pillow python scripts/test-media-interop.py
CHROMIUM and UNTETHERED_BACKEND can select local executables.
Pass --upload-cancel to run the native and Chromium 16 MiB throttled transfer
test instead (also needs Tesseract and the cryptography Python package).
For the native clipboard acceptance test, pass --native-paste wayland or x11.
It needs a debug client build plus Xvfb, xdotool, ImageMagick, and either
sway/grim/wl-copy (Wayland) or xclip (X11); it never uses your desktop clipboard.
"""
import argparse
import io
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from contextlib import contextmanager
from urllib.parse import quote

import requests
import websocket
from PIL import Image
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[1]
BACKEND = Path(os.environ.get("UNTETHERED_BACKEND", ROOT / ".dev/backends/0.2611.1/gomuks")).resolve()


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@contextmanager
def service(args, cwd, **kw):
    p = subprocess.Popen(args, cwd=cwd, stdout=kw.pop("stdout", subprocess.DEVNULL),
                         stderr=subprocess.DEVNULL, **kw)
    try:
        yield p
    finally:
        p.terminate()
        try:
            p.wait(timeout=12)
        except subprocess.TimeoutExpired:
            p.kill()
            p.wait()


def http(method, url, **kw):
    r = requests.request(method, url, timeout=15, **kw)
    r.raise_for_status()
    return r


def seed(tmp, hs):
    user = "media" + secrets.token_hex(4)
    pwd = secrets.token_urlsafe(24)
    auth = http("POST", hs+"/_matrix/client/v3/register", json={
        "auth": {"type": "m.login.dummy"}, "username": user, "password": pwd}).json()
    hdr = {"Authorization": "Bearer "+auth["access_token"]}
    room = http("POST", hs+"/_matrix/client/v3/createRoom", headers=hdr, json={
        "name": "media-proof", "preset": "private_chat", "initial_state": [
            {"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}}]}).json()["room_id"]
    key = secrets.token_hex(32)
    with service([BACKEND, "--desktop"], tmp, stdout=subprocess.PIPE, env={
        **os.environ, "GOMUKS_ROOT": str(tmp/"seed"), "GOMUKS_DESKTOP_KEY": key}) as p:
        addr = json.loads(p.stdout.readline())["address"]
        cookie = http("POST", f"http://{addr}/_gomuks/auth?insecure_cookie=true",
                      auth=("desktop-key", key)).cookies["gomuks_auth"]
        ws = websocket.create_connection(f"ws://{addr}/_gomuks/websocket", suppress_origin=True,
                                         cookie="gomuks_auth="+cookie, timeout=30)
        def rpc(n, cmd, body):
            ws.send(json.dumps(dict(command=cmd, request_id=n, data=body)))
            while True:
                f = json.loads(ws.recv())
                if f["command"] in ("error", "response") and f.get("request_id") == n:
                    assert f["command"] == "response", cmd
                    return f["data"]
        rpc(1, "login", dict(homeserver_url=hs, username=user, password=pwd))
        keys = rpc(2, "generate_recovery_key", {})
        rpc(3, "reset_encryption", {**keys, "account_password": pwd})
        ws.close()
    return dict(server=hs, username=user, password=pwd, key=keys["recovery_key"], room=room), hdr


def check_browser(fixture, web):
    with sync_playwright() as pw:
        browser = pw.chromium.launch(executable_path=os.environ.get("CHROMIUM") or shutil.which("chromium"),
                                     args=["--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"])
        ctx = browser.new_context(viewport={"width": 1180, "height": 760})
        page = ctx.new_page()
        page.goto(web)
        page.wait_for_function("document.title.includes('Preview')", timeout=35000)
        page.mouse.click(1095, 73)
        page.wait_for_function("document.title.includes('Account')")
        for y, field in [(333, "server"), (391, "username"), (447, "password")]:
            page.mouse.click(510, y)
            page.keyboard.type(fixture[field], delay=6)
        page.keyboard.press("Enter")
        secret = page.get_by_label("Recovery key or passphrase")
        secret.wait_for(timeout=35000)
        secret.fill(fixture["key"])
        secret.press("Enter")
        for attempt in range(2):
            if attempt:
                page.reload()
            page.wait_for_function("document.title.includes('media-proof')", timeout=35000)
            end = time.monotonic()+35
            while True:
                im = Image.open(io.BytesIO(page.screenshot())).convert("RGB").crop((280,50,1180,650))
                colors = im.getcolors(im.width * im.height)
                pink = sum(n for n,(r,g,b) in colors if r>205 and g<58 and 120<b<158)
                teal = sum(n for n,(r,g,b) in colors if r<38 and g>195 and 152<b<196)
                if min(pink,teal)>5000:
                    break
                if time.monotonic()>end:
                    raise RuntimeError(f"Encrypted thumbnail not visible after login/reload: {pink}/{teal} matching pixels")
            print(f"Chromium {'reload' if attempt else 'fresh verified device'}: {pink} pink and {teal} teal pixels in timeline.")
        browser.close()


def main():
    args=argparse.ArgumentParser(description=__doc__)
    args.add_argument("--native-paste",choices=("wayland","x11"))
    args.add_argument("--upload-cancel",action="store_true",help="Test native and browser cancellation against a throttled Matrix upload")
    args=args.parse_args()
    assert BACKEND.is_file(), "Run scripts/rust-client.sh once or set UNTETHERED_BACKEND"
    dist = ROOT/"rust-client/web/dist"
    if not args.native_paste:
        assert (dist/"backend/gomuks-worker.js").is_file(), "Run scripts/rust-web.sh first"
    os.umask(0o077)
    with tempfile.TemporaryDirectory(prefix="untethered-media-",dir=os.environ.get("TMPDIR")) as d:
        tmp = Path(d)
        subprocess.run([sys.executable, "-m", "synapse.app.homeserver", "-c", "hs.yaml", "--generate-config",
                        "-H", "test.invalid", "--report-stats", "no"], cwd=tmp, stdout=subprocess.DEVNULL, check=True)
        port = free_port()
        hs = f"http://127.0.0.1:{port}"
        gateway_port=free_port() if args.upload_cancel else None
        public_url=f"http://127.0.0.1:{gateway_port}" if gateway_port else hs
        (tmp/"local.yaml").write_text(f"""listeners:
  - port: {port}
    type: http
    tls: false
    bind_addresses: ["127.0.0.1"]
    resources: [{{names: [client, media]}}]
public_baseurl: {public_url}/
enable_registration: true
enable_registration_without_verification: true
trusted_key_servers: []
federation_domain_whitelist: []
""")
        with service([sys.executable,"-m","synapse.app.homeserver","-c","hs.yaml","-c","local.yaml"],tmp) as proc:
            for _ in range(200):
                if proc.poll() is not None:
                    raise RuntimeError("Disposable homeserver exited")
                try:
                    http("GET",hs+"/_matrix/client/versions")
                    break
                except requests.ConnectionError:
                    time.sleep(.1)
            if args.upload_cancel:
                from qa_upload_cancel import test
                wp=free_port()
                with service([sys.executable,"-m","http.server",str(wp),"--bind","127.0.0.1","--directory",str(dist)],tmp):
                    test(tmp,hs,gateway_port,f"http://127.0.0.1:{wp}",BACKEND,ROOT,seed)
                return
            f, hdr = seed(tmp,hs)
            if args.native_paste:
                from qa_native_paste import check_paste
                check_paste(tmp, ROOT, BACKEND, f, hdr, args.native_paste)
                return
            fixture = tmp/"fixture.json"
            fixture.write_text(json.dumps(f))
            subprocess.run(["cargo","test","--locked","-j3","--manifest-path","rust-client/Cargo.toml",
                            "encrypted_round_trip_and_restart","--","--ignored","--nocapture"],cwd=ROOT, check=True,
                           env={**os.environ,"UNTETHERED_QA_FIXTURE":str(fixture),"UNTETHERED_PROFILE":str(tmp/"native"),
                                "UNTETHERED_BACKEND":str(BACKEND)})
            events = http("GET",f"{hs}/_matrix/client/v3/rooms/{quote(f['room'],safe='')}/messages",
                          headers=hdr,params={"dir":"b","limit":60}).json()["chunk"]
            assert sum(e["type"]=="m.room.encrypted" for e in events)>=3
            assert all(e["type"]!="m.room.message" for e in events)
            wp = free_port()
            with service([sys.executable,"-m","http.server",str(wp),"--bind","127.0.0.1","--directory",str(dist)],tmp):
                check_browser(f,f"http://127.0.0.1:{wp}")
        print("PASS encrypted image interoperability; disposable services stopped.")


if __name__ == "__main__":
    main()
