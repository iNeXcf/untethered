// Observe only inbound browser WebRTC tracks; no access to the local oscillator
// or the media keys. Never replace any track, codec, or encoded-frame transform.
(() => {
    const listen = pc => pc.addEventListener("track", async ({track,receiver}) => {
        if (track.kind !== "audio") return;
        window.qaInbound = async () => {
            const reports = Array.from((await receiver.getStats()).values());
            return reports.find(r=>r.type==="inbound-rtp" && r.kind==="audio")?.packetsReceived ?? 0;
        };
        const ctx = new AudioContext(), stream = new MediaStream([track]);
        const src = ctx.createMediaStreamSource(stream), fft = ctx.createAnalyser();
        fft.fftSize = 4096; fft.smoothingTimeConstant = 0;
        const silent = ctx.createGain(); silent.gain.value = 0;
        src.connect(fft); fft.connect(silent); silent.connect(ctx.destination);
        await ctx.resume();
        const bins = new Float32Array(fft.frequencyBinCount);
        let good = 0;
        const timer = setInterval(() => {
            fft.getFloatFrequencyData(bins);
            const peak = bins.reduce((j,v,i) => v > bins[j] ? i : j, 0);
            const hz = peak * ctx.sampleRate / fft.fftSize;
            window.qaSpectra=(window.qaSpectra??0)+1;
            good = Math.abs(hz-480) < 23 && bins[peak] > -60 ? good+1 : 0;
            if (good >= 10) {window.qaReceived480 = true;clearInterval(timer);void ctx.close();}
        }, 100);
    });
    window.RTCPeerConnection = new Proxy(window.RTCPeerConnection, {
        construct(target,args,newTarget) { const pc = Reflect.construct(target,args,newTarget);listen(pc);return pc; }
    });
})();
