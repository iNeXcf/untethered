"""Two joined accounts: exercise target-specific membership UI and independent server auth."""
from urllib.parse import quote,unquote,urlsplit
import requests
from qa_native_paste import wait_for


def check_ban(page,api,room,peer_hs,admin,member,http,locate,click,restart,x):
    hs=api.split("/_matrix/")[0]
    moderator=http("GET",hs+"/_matrix/client/v3/account/whoami",headers=admin).json()["user_id"]
    peer=page.evaluate("()=>window.mxMatrixClientPeg.get().getSafeUserId()")
    assert moderator!=peer
    member_url=api+"/state/m.room.member/"+quote(peer,safe="")
    def current():
        return http("GET",member_url,headers=admin).json()
    assert current()["membership"]=="join"
    levels=http("GET",api+"/state/m.room.power_levels",headers=admin).json()
    assert levels.get("users",{}).get(peer,levels.get("users_default",0))<levels.get("ban",50)
    # Public at the time of the failed join: a lack of invitation cannot
    # explain this refusal. Switch back to invite-only before lifting the ban.
    def joins(rule):
        http("PUT",api+"/state/m.room.join_rules",headers=admin,json={"join_rule":rule})
        assert http("GET",api+"/state/m.room.join_rules",headers=admin).json()["join_rule"]==rule
    joins("public")
    http("PUT",(peer_hs or hs)+"/_matrix/client/v3/profile/"+quote(peer,safe="")+"/displayname",headers=member,json={"displayname":"Bruno"})
    wait_for("joined target has room display name",lambda:current().get("displayname")=="Bruno",15)
    def point(ws,word,min_x):return next(((xx,y) for w,xx,y in ws if w==word and xx>min_x),None)
    def tap(word,min_x=910):click(locate(word,lambda ws:point(ws,word,min_x)))
    def selected(kind):
        locate("selected "+kind,lambda ws:any(abs(y-yy)<6 for w,xx,y in ws if xx>910 and w=="membership"
                                               for v,xxx,yy in ws if xxx>xx and v==kind))
    def observed(kind):
        return wait_for("Synapse membership="+kind,lambda:s if (s:=current())["membership"]==kind else None)
    def synced(kind):
        page.wait_for_function("([r,k])=>window.mxMatrixClientPeg.get().getRoom(r)?.getMyMembership()===k",arg=[room,kind],timeout=20000)
    def join(status,code=None):
        def is_join(r):
            path=unquote(urlsplit(r.url).path)
            return r.request.method=="POST" and (path.endswith("/join/"+room) or path.endswith("/rooms/"+room+"/join"))
        with page.expect_response(is_join) as wire:
            result=page.evaluate("""async ([r,via])=>{
              try{return {room:(await window.mxMatrixClientPeg.get().joinRoom(r,{viaServers:[via]})).roomId}}
              catch(e){return {code:e.errcode}}
            }""",[room,moderator.split(":",1)[1]])
        response=wire.value
        assert response.status==status,(response.status,response.json(),result)
        if status==403:
            assert response.json()["errcode"]==result["code"]==code,(response.json(),result)
            return response.json()
        else:
            assert response.json()["room_id"]==result["room"]==room
    def send(body):
        def is_send(r):return r.request.method=="PUT" and f"/rooms/{room}/send/m.room.encrypted/" in unquote(urlsplit(r.url).path)
        with page.expect_response(is_send) as wire:
            eid=page.evaluate("async ([r,b])=>(await window.mxMatrixClientPeg.get().sendTextMessage(r,b)).event_id",[room,body])
        assert wire.value.status==200 and wire.value.json()["event_id"]==eid
        page.locator(f'[data-event-id="{eid}"]').get_by_text(body,exact=True).wait_for()
        a,b=body.lower().split()
        locate("Untethered decrypted "+body,lambda ws:any(abs(y-yy)<6 for w,xx,y in ws if w==a and xx>280
            for v,xxx,yy in ws if v==b and xx<xxx<xx+200))
        # A send is acknowledged by the sender's server before federation
        # necessarily reaches the admin's server; inspect the latter after receipt.
        event=http("GET",api+"/event/"+quote(eid,safe=""),headers=admin).json()
        assert event["sender"]==peer and event["type"]=="m.room.encrypted" and "ciphertext" in event["content"]
        return eid
    baseline=send("Arrival beacon")
    denied=requests.post((peer_hs or hs)+"/_matrix/client/v3/rooms/"+quote(room,safe="")+"/ban",headers=member,json={"user_id":moderator},timeout=10)
    assert denied.status_code==403
    tap("people",280);tap("bruno");selected("joined")
    reason="Room conduct test"
    tap("reason");x("type","--clearmodifiers",reason);tap("ban")
    banned=observed("ban");assert banned.get("reason")==reason
    synced("ban");error=join(403,"M_FORBIDDEN" if peer_hs else "M_BAD_STATE")
    assert "ban" in error["error"].lower(),error
    assert current()["membership"]=="ban"
    print(f"Public join rule confirmed. Synapse refused the banned Element member's rejoin: HTTP 403 {error['errcode']} ({error['error']}).",flush=True)
    restart()
    locate("restored pre-ban message",lambda ws:point(ws,"arrival",280)) # Header has finished loading.
    tap("people",280)
    # Ban state may not include a display name; select the Banned roster row.
    tap("banned");selected("banned")
    joins("invite")
    tap("unban");observed("leave");selected("left")
    join(403,"M_FORBIDDEN") # An unban has not silently created an invitation.
    tap("invite");observed("invite");synced("invite")
    join(200);observed("join");synced("join")
    page.goto(page.url.split("#")[0]+"#/room/"+room)
    selected("joined")
    eid=send("Returned lantern")
    timeline=http("GET",api+"/messages",headers=admin,params={"dir":"b","limit":100}).json()["chunk"]
    assert {e["event_id"] for e in timeline if e["type"] in ("m.room.encrypted","m.room.message")}=={baseline,eid}
    transitions=[e for e in reversed(timeline) if e["type"]=="m.room.member" and e["state_key"]==peer]
    assert [(e["sender"],e["content"]["membership"]) for e in transitions[-4:]]==[
        (moderator,"ban"),(moderator,"leave"),(moderator,"invite"),(peer,"join")]
    print("Cold restart restored the Banned row. Unban → leave (invite-only rejoin still 403); "
          "Iced Invite → invite; same Element account rejoined with HTTP 200.",flush=True)
    print(f"Post-rejoin Megolm send HTTP 200 {eid}; Element and Untethered both displayed 'Returned lantern'.",flush=True)
