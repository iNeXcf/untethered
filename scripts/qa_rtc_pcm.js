// Observe only inbound browser WebRTC tracks; no access to the local oscillator
// or the media keys. Never replace any track, codec, or encoded-frame transform.
(() => {
    let peerConnections = 0;
    const pcs = new Set();
    // Resolve the transport used by each active audio RTP report, not a
    // gathered candidate that the ICE agent might never have selected.
    window.qaIceRoutes = async () => (await Promise.all([...pcs].map(async pc => {
        if (pc.connectionState !== "connected") return [];
        const stats = await pc.getStats();
        return [...stats.values()].filter(r=>r.kind==="audio" &&
            ((r.type==="inbound-rtp" && r.packetsReceived>0) || (r.type==="outbound-rtp" && r.packetsSent>0)))
            .map(r=>{
                const pair = stats.get(stats.get(r.transportId)?.selectedCandidatePairId);
                const c = stats.get(pair?.localCandidateId);
                return {direction:r.type.split("-")[0],local:c?.candidateType,relayProtocol:c?.relayProtocol,
                    state:pair?.state,url:c?.url?.split("?")[0]};
            });
    }))).flat();
    const listen = pc => pc.addEventListener("track", async ({track,receiver}) => {
        if (track.kind !== "audio") return;
        // A new track event, even with the same remote track label, is not
        // continuity. Keep the analysis on this one receiver across both keys.
        const receiverId = crypto.randomUUID();
        let losses = 0;
        const lost = state => ["disconnected","failed","closed"].includes(state);
        pc.addEventListener("connectionstatechange", () => { if (lost(pc.connectionState)) losses++; });
        pc.addEventListener("iceconnectionstatechange", () => { if (lost(pc.iceConnectionState)) losses++; });
        const pcm = {spectra:0, last480:0, got480:false, run410:0, run480:0, run730:0, quiet:0, hits730:0, got730:false, measured730:0};
        window.qaAudioStats = async () => {
            const reports = Array.from((await receiver.getStats()).values());
            const rtp = reports.find(r=>r.type==="inbound-rtp" && r.kind==="audio");
            return {...pcm, receiverId, peerConnections, losses, trackId:track.id, state:pc.connectionState,
                ssrc:rtp?.ssrc, packets:rtp?.packetsReceived ?? 0};
        };
        const ctx = new AudioContext(), stream = new MediaStream([track]);
        const src = ctx.createMediaStreamSource(stream), fft = ctx.createAnalyser();
        fft.fftSize = 4096; fft.smoothingTimeConstant = 0;
        const silent = ctx.createGain(); silent.gain.value = 0;
        src.connect(fft); fft.connect(silent); silent.connect(ctx.destination);
        await ctx.resume();
        const bins = new Float32Array(fft.frequencyBinCount);
        let good480 = 0, good730 = 0, sum730 = 0;
        const timer = setInterval(() => {
            fft.getFloatFrequencyData(bins);
            const peak = bins.reduce((j,v,i) => v > bins[j] ? i : j, 0);
            const hz = peak * ctx.sampleRate / fft.fftSize;
            const heard = target => bins[peak] > -60 && Math.abs(hz-target) < 23;
            pcm.spectra++;
            pcm.run410 = heard(410) ? pcm.run410+1 : 0;
            good480 = heard(480) ? good480+1 : 0;
            pcm.run480 = good480;
            pcm.quiet = bins[peak] < -65 ? pcm.quiet+1 : 0;
            if (good480) pcm.last480 = pcm.spectra;
            if (good480 >= 10) pcm.got480 = true;
            if (heard(730)) {good730++; sum730+=hz; pcm.hits730++;}
            else {good730=0; sum730=0;}
            pcm.run730=good730;
            if (good730 >= 10) {pcm.got730=true; pcm.measured730=sum730/good730;}
        }, 100);
        track.addEventListener("ended",()=>{clearInterval(timer);void ctx.close();},{once:true});
    });
    window.RTCPeerConnection = new Proxy(window.RTCPeerConnection, {
        construct(target,args,newTarget) { const pc = Reflect.construct(target,args,newTarget);peerConnections++;pcs.add(pc);listen(pc);return pc; }
    });
})();
