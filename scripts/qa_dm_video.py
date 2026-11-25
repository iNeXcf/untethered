"""Ring and answer two real clients. Verify camera pixels rather than just track metadata."""
import json
import sqlite3
import subprocess as sp
from contextlib import closing

from qa_desktop_dm import sfu_participants
from qa_kwin import KWin
from qa_native_paste import child,isolated_env,wait_for
from qa_room_camera import camera_source,grant_camera,start_camera
from qa_room_layout import pattern_box
from qa_voice_room import Desktop


def check_video(stack,tmp,root,backend,room,users,secret):
    sock=tmp/"pulse"
    desks=[];show=[]
    palettes=[[((31,215,123),(220,40,178)),((244,117,24),(40,83,242))],
              [((27,212,236),(236,218,26)),((223,43,47),(155,97,229))]]
    for i,path in enumerate([tmp,tmp/"peer"]):
        env=isolated_env(path,backend,"wayland")
        env.update(PULSE_SERVER="unix:"+str(sock),PULSE_SOURCE=f"Mic{i}.monitor",PULSE_SINK=f"Out{i}",
                   RAYON_NUM_THREADS="2",TOKIO_WORKER_THREADS="2",GOMAXPROCS="2")
        if i==0:
            stack.enter_context(child(["pulseaudio","-n","--daemonize=no","--exit-idle-time=-1","--use-pid-file=no",
                f"--load=module-native-protocol-unix socket={sock} auth-anonymous=1",
                *(f"--load=module-null-sink sink_name={k}{j} rate=48000 channels=1" for j in range(2) for k in ("Mic","Out"))],env))
            wait_for("two private audio routes",lambda:sock.is_socket() and "Out1" in sp.run(
                ["pactl","list","short","sinks"],env=env,text=True,capture_output=True).stdout)
        kde=KWin(stack,path,root,backend,env=env,screencast=True)
        desks.append(Desktop(stack,root,env,i,kde,room="media-proof"))
        show.append(camera_source(stack,kde,palettes[i]))
    a,b=desks
    def tracks():
        return {(p["identity"].rsplit(":",1)[0],t["source"]):(t["sid"],t.get("encryption"))
                for p in sfu_participants(secret,allow_empty=True) for t in p.get("tracks",[])}
    def readers(d):
        return d.kde.command("pgrep","-P",str(d.kde.native.pid),"-x","gst-launch-1.0",check=False).stdout.splitlines()
    def box(i,phase,local=False):
        return pattern_box(desks[i].kde.capture().crop((283,82,1179,761)),palettes[i if local else 1-i][phase])
    def audio():return a.hears({670}) and b.hears({410})
    a.click("video")
    try:wait_for("incoming video call accepts camera-off",lambda:{"incoming","video","accept","off"}<={w for w,_,_ in b.words()},15)
    except RuntimeError:print("Caller labels:",a.words(),"callee labels:",b.words(),flush=True);raise
    with closing(sqlite3.connect((tmp/"peer/seed/data/gomuks.db").as_uri()+"?mode=ro",uri=True)) as db:
        [(ring,body)]=db.execute("SELECT event_id,decrypted FROM event WHERE room_id=? AND type='m.room.encrypted' "
            "AND decrypted_type='org.matrix.msc4075.rtc.notification'",(room,))
        assert json.loads(body)["m.call.intent"]=="video"
    assert not tracks() and all(not readers(d) for d in desks)
    b.click("accept")
    grant_camera(a.kde,lambda:(users[0],"CAMERA") in tracks())
    wait_for("callee sees caller's pattern",lambda:box(1,0),20)
    wait_for("audio live",audio)
    assert not readers(b) and (users[1],"CAMERA") not in tracks()
    start_camera(b,lambda:(users[1],"CAMERA") in tracks())
    before=tracks()
    assert set(before)=={(u,k) for u in users for k in ("CAMERA","MICROPHONE")} and all(e=="GCM" for _,e in before.values())
    for i in range(2):
        wait_for("peer pattern on "+users[i],lambda:box(i,0),18)
        show[1-i](1)
        bounds=wait_for("changing decoded video on "+users[i],lambda:box(i,1),18)
        print(f"{users[i]} rendered its peer's changing camera at {bounds[2]-bounds[0]}×{bounds[3]-bounds[1]}.",flush=True)
    small=wait_for("local preview visible beside peer",lambda:box(0,1,True))
    assert small[2]-small[0]<170 and box(0,1)[2]-box(0,1)[0]>450
    def select(rect):
        a.kde.command("kdotool","windowactivate",a.win)
        x,y=283+(rect[0]+rect[2])//2,82+(rect[1]+rect[3])//2
        a.kde.actions("pointer",[{"type":"pointerMove","x":x,"y":y},{"type":"pause","duration":250},
                                 {"type":"pointerDown","button":0},{"type":"pause","duration":120},{"type":"pointerUp","button":0}])
    select(small)
    wait_for("local preview focused",lambda:(r:=box(0,1,True)) and r[2]-r[0]>450)
    select(box(0,1))
    wait_for("peer restored to stage",lambda:(r:=box(0,1)) and r[2]-r[0]>450)
    print("Live self-preview and remote tile swapped focus and back without replacing any publication.",flush=True)
    assert tracks()==before and audio()
    b.click("stop")
    wait_for("only callee's camera stops",lambda:not readers(b) and (users[1],"CAMERA") not in tracks() and not box(0,1),15)
    assert box(1,1) and audio()
    a.click("hang")
    wait_for("hangup releases both cameras and all SFU tracks",lambda:not tracks() and not readers(a) and not readers(b),20)
    print(f"Encrypted video invite {ring}; no camera before acceptance; receiver opted in separately. "
          "Both camera and mic pairs GCM; camera-off retained two-way audio and the other feed; hangup released capture.",flush=True)
