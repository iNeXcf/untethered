"""Verify the portal-selected window on both remote clients' rendered surfaces."""
import io
import json
import re
import subprocess as sp
from pathlib import Path

from PIL import Image
from qa_desktop_dm import sfu_participants
from qa_native_paste import wait_for


def area(peer,palette):
    raw=sp.check_output(["import","-window",peer.win,"png:-"],env=peer.env,timeout=6)
    im=Image.open(io.BytesIO(raw)).convert("RGB")
    # Two adjacent unique fields at rendered video-tile scale, not an icon.
    boxes=[]
    for color in palette:
        pts=[(x,y) for y in range(90,630,5) for x in range(290,1175,5)
             if max(abs(v-w) for v,w in zip(im.getpixel((x,y)),color))<32]
        if len(pts)<800:return False
        boxes.append((min(x for x,_ in pts),max(x for x,_ in pts),min(y for _,y in pts),max(y for _,y in pts)))
    a,b=boxes
    return 0<b[0]-a[1]<16 and abs(a[2]-b[2])<10 and abs(a[3]-b[3])<10


def window_share(stack,desks,secret,original,publications):
    sender,*peers=desks
    kde=sender.kde
    source=sp.Popen(["/usr/bin/python",str(Path(__file__).with_name("qa_window_pattern.py"))],env=kde.env,stdin=sp.PIPE,stdout=sp.DEVNULL)
    def stop():source.terminate();source.wait(timeout=5)
    stack.callback(stop)
    for title,y in [("Prism",0),("Decoy",340)]:
        wid=wait_for(title,lambda:kde.window("^"+title+"$"))
        kde.command("kdotool","windowmove",wid,"1300",str(y))
    kde.command("kdotool","windowactivate",sender.win)

    def pw_nodes():
        graph=json.loads(kde.command("pw-dump").stdout)
        return {x["id"]: x["info"]["props"].get("node.name","") for x in graph
                if x.get("type")=="PipeWire:Interface:Node" and "Video" in x["info"]["props"].get("media.class","")}
    def video():
        return [(p["identity"],t) for p in sfu_participants(secret) for t in p.get("tracks",[]) if t.get("type")=="VIDEO"]
    def sessions():
        tree=kde.command("busctl","--address="+kde.env["DBUS_SESSION_BUS_ADDRESS"],"--list",
                         "tree","org.freedesktop.portal.Desktop").stdout
        return {p.strip() for p in tree.splitlines() if re.fullmatch(r"/org/freedesktop/portal/desktop/session/[^/]+/[^/]+",p.strip())}
    first=[(235,36,119),(14,204,153)]
    second=[(24,79,234),(245,132,33)]
    before=pw_nodes();before_sessions=sessions()
    sender.click("share")
    def picker():
        return kde.window("^Screen Sharing Requested")
    wid=wait_for("KDE window picker",picker,18)
    print("Portal picker title:",kde.command("kdotool","getwindowname",wid).stdout.strip(),flush=True)
    def dialog_click(label):
        geom=kde.command("kdotool","getwindowgeometry",wid).stdout
        x,y=map(int,re.search(r"Position: (-?\d+),(-?\d+)",geom).groups())
        w,h=map(int,re.search(r"Geometry: (\d+)x(\d+)",geom).groups())
        words=kde.words((x,y+24,x+w,y+h))
        found=next(((x,y) for t,x,y in words if t.strip("|:").lower()==label),None)
        if not found:raise AssertionError(f"Picker label {label!r} not in {words}")
        kde.click(*found)
    kde.command("kdotool","windowactivate",wid)
    kde.keys(("Down","\ue00c"),("Up","\ue00c"))  # Escape in the private chooser.
    wait_for("cancelled picker closed",lambda:not picker() and sessions()==before_sessions)
    wait_for("desktop acknowledged cancellation",lambda:any(w in ("cancelled","denied") for w,_,_ in sender.words()))
    assert not video()
    wait_for("cancelled capture released",lambda:pw_nodes()==before)
    print("Cancelled chooser: no screen publication, portal session or video nodes left.",flush=True)
    sender.click("share")
    wid=wait_for("fresh KDE chooser",picker)
    dialog_click("prism")
    if picker()==wid:dialog_click("share")
    share=wait_for("GCM screen publication",video,25)
    assert len(share)==1 and share[0][1]["source"]=="SCREEN_SHARE" and share[0][1]["encryption"]=="GCM",share
    during=pw_nodes();capture_nodes=set(during)-set(before)
    assert capture_nodes, "expected a new capture source or PipeWire reader"
    active=sessions()-before_sessions
    assert len(active)==1,active
    wait_for("both receivers render Prism",lambda:all(area(p,first) for p in peers),20)
    print("Both peers rendered the selected pink/teal Prism window on one GCM screen track.",flush=True)
    sender.click("media-proof")
    wait_for("sharer is browsing another room",lambda:kde.window("Untethered.*media-proof"))
    desks[0].tone=3;desks[1].tone=4
    fresh=[{830,1030},{530,1030},{530,830}]
    source.stdin.write(b"change\n");source.stdin.flush()
    wait_for("both receivers render new Prism pixels after navigation",lambda:all(area(p,second) for p in peers),18)
    wait_for("new audio tones while sharing from another channel",lambda:all(d.hears(h) for d,h in zip(desks,fresh)))
    current=video()
    assert publications(3)==original and len(current)==1 and current[0][1]["sid"]==share[0][1]["sid"] and current[0][1]["encryption"]=="GCM"
    print("Sharer browsed media-proof; both receivers rendered Prism's new blue/orange pattern. Speakers decoded [830,1030], [530,1030], [530,830] Hz on the original audio tracks.",flush=True)
    sender.click("stop")
    wait_for("screen track unpublished",lambda:not video(),12)
    wait_for("private video graph returned to baseline",lambda:pw_nodes()==before,12)
    wait_for("portal session removed",lambda:sessions()==before_sessions,10)
    wait_for("remote tiles cleared",lambda:all(not area(p,second) and
             not any(w=="shared" for w,_,_ in p.words()) for p in peers),10)
    assert all(d.hears(h) for d,h in zip(desks,fresh)) and publications(3)==original
    desks[0].tone=0;desks[1].tone=1
    print(f"Stop sharing: portal session gone, {len(capture_nodes)} PipeWire video node(s) removed, 0 screen tracks and both remote tiles cleared; original microphones still live.",flush=True)
