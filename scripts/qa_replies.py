"""Two device UI test. The duplicate originals have the same author and exact text."""
import io
import json
import os
import re
import shutil
import subprocess as sp
from contextlib import ExitStack
from urllib.parse import quote,unquote

from PIL import Image
from playwright.sync_api import sync_playwright
from qa_kwin_element import element_server,login_element
from qa_native_paste import child,isolated_env,start_display,wait_for
from qa_upload_cancel import slow_gateway


def test_replies(tmp,hs,root,backend,seed,http,port):
    paths=[]
    def observe(method,path):
        if method=="GET" and "/context/" in path:paths.append(unquote(path))
    with ExitStack() as stack:
        proxy,_=stack.enter_context(slow_gateway(hs,port,observe_path=observe))
        native,hdr=seed(tmp,proxy)
        remote=tmp/"peer";remote.mkdir()
        bob,bh=seed(remote,hs)
        room=native["room"];api=hs+"/_matrix/client/v3/rooms/"+quote(room,safe="")
        uid="@"+bob["username"]+":test.invalid"
        http("POST",api+"/invite",headers=hdr,json={"user_id":uid})
        http("POST",api+"/join",headers=bh,json={})
        web=stack.enter_context(element_server(hs));pw=stack.enter_context(sync_playwright())
        browser=pw.chromium.launch(executable_path=os.environ.get("CHROMIUM") or shutil.which("chromium"),
                                   args=["--disable-dev-shm-usage"]);stack.callback(browser.close)
        ctx=browser.new_context(viewport={"width":1280,"height":920},locale="en-US")
        ctx.route(re.compile(r"^https?://(?!127\.0\.0\.1:)"),lambda route:route.abort())
        page=login_element(ctx,web,bob,room)
        def messages(texts):
            return page.evaluate("""async ([room,texts])=>{
                const c=window.mxMatrixClientPeg.get(),ids=[];
                for(const text of texts)ids.push((await c.sendTextMessage(room,text)).event_id);
                return ids;
            }""",[room,texts])
        def tile(eid):return page.locator(f'[data-event-id="{eid}"]')
        def record(body):
            return page.evaluate("""([room,body])=>window.mxMatrixClientPeg.get().getRoom(room)?.getLiveTimeline().getEvents()
                .filter(e=>e.getContent().body?.endsWith(body)&&e.getId()?.startsWith('$'))
                .map(e=>({id:e.getId(),target:e.getContent()['m.relates_to']?.['m.in_reply_to']?.event_id}))""",[room,body])
        env=isolated_env(tmp,backend,"x11")
        start_display(stack,tmp,env,"x11")
        native_life=stack.enter_context(ExitStack())
        def x(*args):return sp.check_output(["xdotool",*args],env=env,text=True,timeout=6).strip()
        def launch():
            p=native_life.enter_context(child([root/"rust-client/target/debug/untethered-iced"],env))
            def visible():
                assert p.poll() is None
                r=sp.run(["xdotool","search","--onlyvisible","--name","Untethered.*media-proof"],
                         env=env,text=True,capture_output=True)
                return r.stdout.strip() if r.returncode==0 else None
            wid=wait_for("native synced window",visible,30)
            x("windowsize",wid,"1180","760","windowfocus",wid)
            return wid
        win=launch()
        def words():
            raw=sp.check_output(["import","-window",win,"png:-"],env=env,timeout=6)
            im=Image.open(io.BytesIO(raw)).convert("RGB");buf=io.BytesIO()
            im.resize((2360,1520)).save(buf,"PNG")
            t=sp.check_output(["tesseract","stdin","stdout","--psm","11","tsv"],input=buf.getvalue(),stderr=sp.DEVNULL)
            return [(s[11].lower().strip(".,:|"),(int(s[6])+int(s[8])//2)//2,(int(s[7])+int(s[9])//2)//2)
                    for line in t.decode().splitlines() if len(s:=line.split("\t"))==12 and s[0]=="5"]
        def locate(what,fn):
            try:return wait_for(what,lambda:fn(words()),15)
            except RuntimeError:
                print("Native visible words:",[w for w,_,_ in words()],flush=True);raise
        def click(pos):x("mousemove","--window",win,*map(str,pos),"click","1")
        def lines(words,label):return [(x,y) for v,x,y in words if v==label and x>280]
        first,_,second,_=messages(["Beacon original","Cobalt waypoint","Beacon original","Amber waypoint"])
        def select_first(ws):
            originals=sorted(y for _,y in lines(ws,"beacon"))
            return len(originals)==2 and next(((x,y) for x,y in lines(ws,"reply") if abs(y-originals[0])<6),None)
        click(locate("Reply on first of two identical messages",select_first))
        locate("native reply draft",lambda ws:lines(ws,"replying"))
        # Element drafts its reply to the other event using its public Reply button.
        tile(second).hover()
        tile(second).get_by_role("button",name="Reply",exact=True).click()
        page.locator(".mx_ReplyPreview").wait_for()
        # Fill the gap using actual encrypted events rather than mock timeline rows.
        gaps=messages([f"History marker {i:03}" for i in range(80)])
        assert len(set(gaps))==80
        nbody="Northern response";ebody="Southern response"
        x("type","--clearmodifiers",nbody);x("key","Return")
        nr=wait_for("Element decoded native reply",lambda:record(nbody))
        assert len(nr)==1 and nr[0]["target"]==first,nr
        editor=page.locator('[contenteditable=true]:visible').last
        editor.fill(ebody);editor.press("Enter")
        er=wait_for("Element confirmed outgoing reply",lambda:record(ebody))
        assert len(er)==1 and er[0]["target"]==second,er
        # Check all 86 user messages, not only events that the clients decrypted.
        timeline=http("GET",api+"/messages",headers=hdr,params={"dir":"b","limit":100}).json()["chunk"]
        wire=[e for e in timeline if e["type"] in ("m.room.encrypted","m.room.message")]
        assert len(wire)==86 and len({e["event_id"] for e in wire})==86
        assert set([first,second,nr[0]["id"],er[0]["id"],*gaps])<={e["event_id"] for e in wire}
        assert all(e["type"]=="m.room.encrypted" and e["content"]["algorithm"]=="m.megolm.v1.aes-sha2" for e in wire)
        assert not any(t in json.dumps(wire) for t in (nbody,ebody,"Beacon original"))
        print(f"Element decrypted Untethered's reply → {first}; Element replied to the look-alike → {second}. "
              "All 86 room messages are distinct Megolm events on Synapse.",flush=True)
        def jump_above(body):
            def find(ws):
                b=lines(ws,body)
                if not b:return None
                links=[(x,y) for x,y in lines(ws,"view") if 0<b[-1][1]-y<45]
                return max(links,key=lambda p:p[1],default=None)
            click(locate("original link for "+body,find))
        def check_marker(marker):
            locate("highlighted original in its actual history",lambda ws:
                   lines(ws,"original") and lines(ws,"history") and lines(ws,"beacon") and
                   any(0<y-lines(ws,"beacon")[0][1]<54 for _,y in lines(ws,marker)))
        def latest():
            click(locate("return to recent timeline",lambda ws:next(iter(lines(ws,"latest")),None)))
            locate("reply timeline restored",lambda ws:lines(ws,"southern") and lines(ws,"northern"))
        paths.clear()
        jump_above("southern");check_marker("amber");latest()
        assert not paths,"loaded original should not require a context request"
        # End the native transport AND UI, then reopen its existing device profile.
        native_life.close()
        page.reload()
        tile(nr[0]["id"]).wait_for()
        win=launch()
        locate("both replies on cold native timeline",lambda ws:lines(ws,"northern") and lines(ws,"southern"))
        live=page.evaluate("r=>window.mxMatrixClientPeg.get().getRoom(r).getLiveTimeline().getEvents().map(e=>e.getId())",room)
        assert first not in live and second not in live,(len(live),live.index(first) if first in live else None)
        assert record(nbody)==nr and record(ebody)==er
        # Click the quoted text inside the native reply as rendered by Element.
        tile(nr[0]["id"]).locator("blockquote").get_by_text("Beacon original",exact=True).click()
        page.wait_for_function("""id=>{const el=document.querySelector('[data-event-id="'+CSS.escape(id)+'"]');
            if(!el || !(el.className.includes('stateHighlighted') || el.className.includes('stateSelected') ||
                decodeURIComponent(location.hash).endsWith('/'+id)))return false;
            const r=el.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight}""",arg=first)
        print(f"Reloaded Element: original absent from {len(live)} live events; jump brought {first} into view.",flush=True)
        for body,target,marker in [("southern",second,"amber"),("northern",first,"cobalt")]:
            paths.clear()
            jump_above(body)
            check_marker(marker)
            assert paths==[f"/_matrix/client/v3/rooms/{room}/context/{target}"],paths
            print(f"Restarted Untethered: fetched exact /context/{target}; original 'Beacon original' "
                  f"followed by '{marker.title()} waypoint' on screen.",flush=True)
            if target==second:
                click(locate("earlier context",lambda ws:next(iter(lines(ws,"preceding")),None)))
                locate("both originals in preceding context",lambda ws:len(lines(ws,"beacon"))==2 and lines(ws,"cobalt"))
            latest()
