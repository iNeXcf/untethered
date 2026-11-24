"""Click the desktop and Element call controls; measure PCM at the private audio server."""
import array
import base64
import hashlib
import hmac
import io
import json
import math
import os
import re
import subprocess as sp
import threading
import time
from pathlib import Path
from urllib.parse import urlsplit

from PIL import Image
import requests
from qa_native_paste import child, isolated_env, start_display, wait_for


def sfu_audio(secret):
    def query(method,params,grant):
        def b64(v):
            return base64.urlsafe_b64encode(v).rstrip(b"=")
        claims={"iss":"testkey","exp":int(time.time())+30,"video":grant}
        body=b64(b'{"alg":"HS256","typ":"JWT"}')+b"."+b64(json.dumps(claims).encode())
        token=body+b"."+b64(hmac.digest(secret.encode(),body,hashlib.sha256))
        r=requests.post("http://127.0.0.1:18820/twirp/livekit.RoomService/"+method,
                        headers={"Authorization":"Bearer "+token.decode()},json=params,timeout=5)
        r.raise_for_status()
        return r.json()
    rooms=query("ListRooms",{},{"roomList":True})["rooms"]
    assert len(rooms)==1
    name=rooms[0]["name"]
    peers=query("ListParticipants",{"room":name},{"roomAdmin":True,"room":name})["participants"]
    return {p["identity"]: [track.get("encryption","NONE") for track in p.get("tracks",[]) if track.get("type","AUDIO")=="AUDIO"]
            for p in peers if p.get("tracks")}


def desktop_call(stack,tmp,page,room,root,backend,alice,bob,secret):
    env=isolated_env(tmp,backend,"x11")
    start_display(stack,tmp,env,"x11",1280,900)
    sock=Path(env["XDG_RUNTIME_DIR"])/"qa-pulse"
    env.update(PULSE_SERVER="unix:"+str(sock),PULSE_SOURCE="QAMic.monitor",PULSE_SINK="QASpeaker")
    def cmd(*args,**kw):
        return sp.run(args,env=env,capture_output=True,timeout=8,**kw)
    pulse=stack.enter_context(child(["pulseaudio","-n","--daemonize=no","--exit-idle-time=-1","--use-pid-file=no",
        "--load=module-native-protocol-unix socket="+str(sock)+" auth-anonymous=1",
        "--load=module-null-sink sink_name=QAMic rate=48000 channels=1",
        "--load=module-null-sink sink_name=QASpeaker rate=48000 channels=1"],env))
    def ready():
        assert pulse.poll() is None,"private audio server exited"
        return sock.is_socket() and "QASpeaker" in cmd("pactl","list","short","sinks",text=True).stdout
    wait_for("private PulseAudio sinks",ready)
    # Play only into the null mic sink; monitor only the separate speaker sink.
    args=["--raw","--rate=48000","--channels=1","--format=s16le","--latency-msec=40"]
    feed=sp.Popen(["pacat","--playback","--device=QAMic",*args],stdin=sp.PIPE,env=env,stderr=sp.DEVNULL)
    monitor=sp.Popen(["pacat","--record","--device=QASpeaker.monitor",*args],stdout=sp.PIPE,env=env,stderr=sp.DEVNULL)
    def end():
        for p in (feed,monitor):
            p.terminate();p.wait(timeout=5)
    stack.callback(end)
    tone=array.array("h",(round(9000*math.sin(i*math.tau*480/48000)) for i in range(4800))).tobytes()
    def inject():
        try:
            while True:feed.stdin.write(tone)
        except (OSError,ValueError):pass
    received=threading.Event()
    def observe():
        good=0
        while data:=monitor.stdout.read(9600):
            pcm=array.array("h");pcm.frombytes(data)
            hz=sum(a<=0<b for a,b in zip(pcm,pcm[1:]))*48000/len(pcm)
            good=good+1 if abs(hz-960)<35 and max(pcm)>500 else 0
            if good>=6:received.set()
    for fun in (inject,observe):threading.Thread(target=fun,daemon=True).start()
    client=stack.enter_context(child([root/"rust-client/target/debug/untethered-iced"],env))
    def window():
        assert client.poll() is None,"desktop exited"
        return cmd("xdotool","search","--onlyvisible","--name","Untethered.*media-proof",text=True).stdout.strip()
    win=wait_for("desktop verified DM",window,40).splitlines()[0]
    cmd("xdotool","windowmove",win,"0","0",check=True)
    cmd("xdotool","windowsize",win,"1180","760",check=True)
    def words():
        im=Image.open(io.BytesIO(cmd("import","-window",win,"png:-",check=True).stdout)).crop((0,0,1180,190)).resize((2360,380))
        buf=io.BytesIO();im.save(buf,"PNG")
        data=sp.run(["tesseract","stdin","stdout","--psm","11","tsv"],input=buf.getvalue(),capture_output=True,check=True).stdout.decode()
        return [(s[11],(int(s[6])+int(s[8])//2)//2,(int(s[7])+int(s[9])//2)//2) for line in data.splitlines()
                if len(s:=line.split("\t"))==12 and s[0]=="5" and s[11].strip()]
    def find(label):
        return next(((x,y) for t,x,y in words() if re.sub(r"^\W+|\W+$","",t).lower()==label.lower()),None)
    def click(label):
        x,y=wait_for("desktop "+label+" control",lambda:find(label),20)
        cmd("xdotool","mousemove","--window",win,str(x),str(y),"click","1",check=True)
    def roster():
        return page.evaluate("""room=>{const c=window.mxMatrixClientPeg.get();
          return c.matrixRTC.getRoomSession(c.getRoom(room)).memberships.map(m=>m.userId)}""",room)
    def streams(kind):
        return [s for s in json.loads(cmd("pactl","-f","json","list",kind,check=True).stdout)
                if s["properties"].get("application.name")=="Untethered"]
    assert roster()==[] and not streams("source-outputs")
    try:
        ring_started=threading.Event()
        page.on("console",lambda m:ring_started.set() if "LegacyCallHandler.play(ringAudio): playing audio successfully" in m.text else None)
        click("Voice")
        toast=page.locator(".mx_IncomingCallToast_content")
        toast.get_by_text("Incoming voice call",exact=True).wait_for(timeout=25000)
        assert alice in toast.inner_text() and roster()==[alice]
        wait_for("Element ringtone playing",lambda:ring_started.is_set() or page.wait_for_timeout(100),8)
        def lease():
            return page.evaluate("""([room,user])=>{const c=window.mxMatrixClientPeg.get();const m=c.matrixRTC
                .getRoomSession(c.getRoom(room)).memberships.find(m=>m.userId===user);
                return m && {event:m.eventId, expiry:m.getAbsoluteExpiry()}}""",[room,alice])
        first=lease()
        assert not streams("source-outputs"),"microphone opened before answer"
        print("Desktop Voice call → Element displayed the incoming DM and started its ringtone; microphone still closed.",flush=True)
        toast.get_by_role("button",name="Join",exact=True).click()
        wait_for("both accepted members",lambda:set(roster())=={alice,bob},25)
        widget=next(f for f in page.frames if urlsplit(f.url).path.startswith("/widgets/element-call/"))
        wait_for("Element decodes desktop mic's 480 Hz",lambda:widget.evaluate("window.qaAudioStats?.().then(s=>s.got480) ?? false"),30)
        assert received.wait(10),"desktop speaker did not render Element's 960 Hz"
        assert len(streams("source-outputs"))==len(streams("sink-inputs"))==1
        tracks=sfu_audio(secret)
        assert len(tracks)==2 and all(v==["GCM"] for v in tracks.values()) and {p.rsplit(":",1)[0] for p in tracks}=={alice,bob}, tracks
        print("Element accepted; Element measured 480 Hz on received WebRTC audio, desktop speaker monitor measured 960 Hz for 600 ms.",flush=True)
        print("LiveKit independently reports GCM on both microphone publications.",flush=True)
        while time.time()*1000 < first["expiry"]+1200:
            assert set(roster())=={alice,bob}, f"lost a member {int((time.time()*1000-first['expiry'])/1000+60)}s after join"
            page.wait_for_timeout(400)
        updated=lease()
        assert updated["event"]!=first["event"] and updated["expiry"]>time.time()*1000
        sample=widget.evaluate("window.qaAudioStats()")
        assert sample["got480"] and sample["spectra"]-sample["last480"]<5
        received.clear()
        assert received.wait(3)
        print("Both tones still present past the original 60-second membership expiry.",flush=True)
        click("Hang")
        wait_for("desktop and Element left DM call",lambda:roster()==[],22)
        wait_for("desktop released both audio devices",lambda:not streams("source-outputs") and not streams("sink-inputs"))
        assert find("Voice")
        print("Desktop Hang up → empty Element call roster, 0 desktop capture/playback streams; call control available again.",flush=True)
    except Exception:
        print("Desktop visible labels:", " ".join(t for t,_,_ in words()),flush=True)
        print("Element roster:",roster(),"incoming toast:",page.locator(".mx_IncomingCallToast_content").all_text_contents(),flush=True)
        print("Membership timeline:",page.evaluate("""room=>window.mxMatrixClientPeg.get().getRoom(room)
            .getLiveTimeline().getEvents().filter(e=>e.getType()==="org.matrix.msc3401.call.member")
            .map(e=>[e.getTs(),e.getSender(),e.getContent().expires??"leave"])""",room),flush=True)
        raise
