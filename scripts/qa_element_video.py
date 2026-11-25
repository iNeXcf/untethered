"""Two actual video callers, independent synthetic camera inputs and rendered remote pixels."""
import io
import re
import subprocess as sp
from urllib.parse import urlsplit

from PIL import Image
from qa_desktop_dm import sfu_participants
from qa_kwin import KWin
from qa_native_paste import child,isolated_env,wait_for
from qa_room_camera import camera_source,grant_camera,start_camera
from qa_room_layout import pattern_box
from qa_voice_room import Desktop

NATIVE=[((31,215,123),(220,40,178)),((244,117,24),(40,83,242))]
BROWSER=[((235,30,30),(26,40,230)),((30,205,210),(230,208,27))]


def y4m(path):
    def yuv(rgb):
        r,g,b=rgb
        return [round(v) for v in (16+.257*r+.504*g+.098*b,128-.148*r-.291*g+.439*b,128+.439*r-.368*g-.071*b)]
    with path.open("wb") as f:
        f.write(b"YUV4MPEG2 W320 H180 F12:1 Ip A1:1 C420jpeg\n")
        for pair in BROWSER:
            a,b=map(yuv,pair)
            planes=b"".join((bytes([a[i]])*w+bytes([b[i]])*w)*h for i,w,h in ((0,160,180),(1,80,90),(2,80,90)))
            for _ in range(20):f.write(b"FRAME\n"+planes)


def verify(stack,tmp,root,backend,page,room,alice,bob,secret,incoming):
    env=isolated_env(tmp,backend,"wayland")
    sock=tmp/"video-pulse"
    env.update(PULSE_SERVER="unix:"+str(sock),PULSE_SOURCE="Mic0.monitor",PULSE_SINK="Out0")
    stack.enter_context(child(["pulseaudio","-n","--daemonize=no","--exit-idle-time=-1","--use-pid-file=no",
        "--load=module-native-protocol-unix socket="+str(sock)+" auth-anonymous=1",
        *(f"--load=module-null-sink sink_name={k} rate=48000 channels=1" for k in ("Mic0","Out0"))],env))
    wait_for("private audio",lambda:sock.is_socket() and "Out0" in sp.run(["pactl","list","short","sinks"],env=env,capture_output=True,text=True).stdout)
    kde=KWin(stack,tmp,root,backend,env=env,screencast=True)
    desk=Desktop(stack,root,env,0,kde,room="media-proof")
    show=camera_source(stack,kde,NATIVE)
    def readers():return kde.command("pgrep","-P",str(kde.native.pid),"-x","gst-launch-1.0",check=False).stdout.strip()
    def tracks():
        return {(p["identity"].rsplit(":",1)[0],t["source"]):(t["sid"],t.get("encryption"),t.get("muted",False))
                for p in sfu_participants(secret,allow_empty=True) for t in p.get("tracks",[])}
    def cam(u):return (t:=tracks().get((u,"CAMERA"))) and not t[2]
    def widget():return next((f for f in page.frames if urlsplit(f.url).path.startswith("/widgets/element-call/")),None)
    def roster():return page.evaluate("""r=>{const c=window.mxMatrixClientPeg.get();return c.matrixRTC
        .getRoomSession(c.getRoom(r)).memberships.map(m=>m.userId)}""",room)
    def native_box(pair):return pattern_box(kde.capture().crop((283,82,1179,761)),pair)
    def browser_box(pair):
        # Small overlaid labels/controls can occlude the edges of a tile.
        for v in widget().locator("video").all():
            if not v.is_visible():continue
            im=Image.open(io.BytesIO(v.screenshot(timeout=4000))).convert("RGB")
            if im.height<70:continue
            hit=pattern_box(im.crop((0,im.height//3,im.width,2*im.height//3)),pair)
            if hit:return hit
    def camera_button(w):
        return w.get_by_test_id("incall_videomute")
    def logs():
        print("Native UI:",[w for w,_,_ in desk.words()],"roster:",roster(),"publications:",tracks(),flush=True)
        if widget():print("Widget controls:",widget().get_by_role("button").evaluate_all("bs=>bs.map(b=>b.ariaLabel||b.title||b.innerText)"),flush=True)
    try:
        def run():
            show(0)
            if incoming:
                page.get_by_role("button",name="Video call",exact=True).click()
                def ringing():
                    if w:=widget():
                        join=w.get_by_role("button",name=re.compile(r"^(Join|Join call|Start call|Enter call)$",re.I))
                        if join.count() and join.first.is_visible():join.first.click(timeout=2000)
                    return {"incoming","video","accept"}<={v for v,_,_ in desk.words()}
                wait_for("native incoming video controls",ringing,25)
                assert not readers();desk.click("accept")
            else:
                desk.click("video")
                toast=page.locator(".mx_IncomingCallToast_content")
                toast.get_by_text("Incoming video call",exact=True).wait_for(timeout=25000)
                assert not readers()
                toast.get_by_role("button",name="Join",exact=True).click()
            wait_for("two members",lambda:set(roster())=={alice,bob},25)
            signals=page.evaluate("""r=>window.mxMatrixClientPeg.get().getRoom(r).getLiveTimeline().getEvents()
                .filter(e=>e.getType()==='org.matrix.msc4075.rtc.notification' && e.getContent().notification_type==='ring')
                .map(e=>[e.getSender(),e.getWireType(),e.getContent()['m.call.intent'],e.getId()])""",room)
            assert len(signals)==1 and signals[0][:3]==[bob if incoming else alice,"m.room.encrypted","video"],signals
            w=wait_for("Element Call widget",widget)
            # Depending on the incoming-call settings Element can answer with camera off.
            if incoming:
                wait_for("native connected before camera switch",lambda:"connected" in {s for s,_,_ in desk.words()},20)
                assert not readers() and not cam(alice)
                start_camera(desk,lambda:cam(alice))
            else:grant_camera(kde,lambda:cam(alice))
            camera_button(w).wait_for()
            if camera_button(w).get_attribute("aria-checked")=="false":camera_button(w).click(timeout=7000)
            wait_for("both video sources published",lambda:cam(alice) and cam(bob),15)
            before=tracks()
            assert set(before)=={(u,k) for u in (alice,bob) for k in ("CAMERA","MICROPHONE")}
            assert all(enc=="GCM" and not muted for _,enc,muted in before.values())
            for i in range(2):
                show(i)
                left=wait_for("Element renders native pattern "+str(i),lambda:browser_box(NATIVE[i]),20)
                right=wait_for("Iced renders browser pattern "+str(i),lambda:native_box(BROWSER[i]),20)
                print(f"Pattern {i+1}: Element rendered {left[2]-left[0]} px wide, Untethered {right[2]-right[0]} px wide.",flush=True)
            assert tracks()==before
            def audio():return desk.hears({670}) and w.evaluate("window.qaAudioStats?.().then(s=>s.run410>=10)??false")
            wait_for("fresh two-way decoded audio with cameras on",audio,15)
            camera_button(w).click(timeout=7000)
            wait_for("Element camera off",lambda:not cam(bob),10)
            wait_for("Element's old video cleared in Iced",lambda:all(not native_box(p) for p in BROWSER),12)
            assert readers() and cam(alice)
            def camera_off_ui():
                labels=[word.lower() for word,_,_ in kde.words((283,110,1178,610))]
                return all(word in labels for word in ("no","incoming","video"))
            wait_for("native remote camera-off stage",camera_off_ui,12)
            camera_button(w).click()
            wait_for("Element camera enabled again",lambda:cam(bob))
            wait_for("native decoded resumed camera",lambda:native_box(BROWSER[0]),15)
            resumed=tracks()
            assert resumed[bob,"CAMERA"][1:] == ("GCM",False)
            assert all(resumed[u,"MICROPHONE"]==before[u,"MICROPHONE"] for u in (alice,bob))
            desk.click("stop")
            wait_for("native camera closed",lambda:not readers() and not cam(alice),10)
            spectra=w.evaluate("window.qaAudioStats().then(s=>s.spectra)")
            desk.windows.clear()
            wait_for("fresh audio after stopping camera",lambda:audio() and w.evaluate(
                "start=>window.qaAudioStats().then(s=>s.spectra-start>=10)",spectra))
            if incoming:desk.click("hang")
            else:w.get_by_role("button",name="End call",exact=True).click()
            wait_for("empty call roster and no SFU tracks",lambda:not roster() and not tracks(),25)
            print(("Element" if incoming else "Untethered")+" initiated video: both clients rendered two remote patterns; "
                  "2 GCM cameras + 2 GCM mics, 410/670 Hz delivered. Element camera off/on cleared and restored the remote tile, "
                  "hangup emptied roster and released tracks.",flush=True)
        run()
    except Exception:
        logs();raise
