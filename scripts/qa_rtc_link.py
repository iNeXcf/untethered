"""A separate network stack for just the desktop and its gomuks child.

Use only within run_element.sh's disposable outer namespace. The veth is
the desktop's only non-loopback path. Preserve the loopback service URLs in
the seeded Matrix profile, without sharing the service namespace's loopback.
"""
import json
import os
import subprocess as sp
from contextlib import contextmanager


def desktop_network(stack, ports):
    def run(*args, **kw):
        return sp.run(args, check=True, capture_output=True, text=True, timeout=5, **kw)

    interfaces=json.loads(run("ip","-j","addr").stdout)
    assert {i["ifname"] for i in interfaces}=={"lo","rtc-test"}
    assert any(a["local"]=="192.0.2.2" for i in interfaces for a in i["addr_info"])
    ns=sp.Popen(["unshare","-n","sh","-ec","ip link set lo up; echo ready; exec cat"],
                stdin=sp.PIPE, stdout=sp.PIPE, text=True)
    def stop():
        ns.terminate(); ns.wait(timeout=5)
    stack.callback(stop)
    assert ns.stdout.readline().strip()=="ready"
    assert os.stat(f"/proc/{ns.pid}/ns/net").st_ino!=os.stat("/proc/self/ns/net").st_ino
    prefix=["nsenter","-t",str(ns.pid),"-n","--"]
    def inside(*args, **kw):return run(*prefix,*args,**kw)
    run("ip","link","add","qa-server","type","veth","peer","name","qa-desktop","netns",str(ns.pid))
    stack.callback(run,"ip","link","del","qa-server")
    for execute,iface,ip in [(run,"qa-server","198.51.100.1"),(inside,"qa-desktop","198.51.100.2")]:
        execute("ip","addr","add",ip+"/30","dev",iface)
        execute("ip","link","set",iface,"up")
        execute("sysctl","-qw",f"net.ipv4.conf.{iface}.route_localnet=1")
    inside("ip","route","add","192.0.2.2/32","via","198.51.100.1")
    ports=", ".join(str(int(p)) for p in ports)
    run("nft","add","table","ip","qa_rtc")
    stack.callback(run,"nft","delete","table","ip","qa_rtc")
    run("nft","-f","-",input=f"""add chain ip qa_rtc pre {{ type nat hook prerouting priority dstnat; }}
add rule ip qa_rtc pre iifname "qa-server" ip daddr 198.51.100.1 tcp dport {{ {ports} }} dnat to 127.0.0.1
""")
    inside("nft","-f","-",input=f"""add table ip qa_rtc
add chain ip qa_rtc out {{ type nat hook output priority dstnat; }}
add chain ip qa_rtc post {{ type nat hook postrouting priority srcnat; }}
add rule ip qa_rtc out ip daddr 127.0.0.1 tcp dport {{ {ports} }} dnat to 198.51.100.1
add rule ip qa_rtc post oifname "qa-desktop" ip saddr 127.0.0.1 snat to 198.51.100.2
""")

    @contextmanager
    def outage():
        # Remove connectivity in *both* directions (all protocols). Reset the
        # desktop's established SFU TCP socket as on a lost access network;
        # do not wait for the kernel's potentially minutes-long TCP timeout.
        endpoint=["dst","127.0.0.1","dport","=" ,":18820"]
        assert inside("ss","-Htn","state","established",*endpoint).stdout.strip()
        run("ip","link","set","qa-server","down")
        try:
            assert inside("ss","-HKtn","state","established",*endpoint).stdout.strip()
            yield
        finally:
            run("ip","link","set","qa-server","up")

    return prefix,outage
