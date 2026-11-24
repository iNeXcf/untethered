let stage;
export function install_files(onFiles, hover) {
    stage = onFiles;
    const hasFiles = e => e.dataTransfer?.types.includes("Files");
    window.addEventListener("dragover", e => {
        if (!hasFiles(e)) return;
        e.preventDefault(); e.dataTransfer.dropEffect = "copy";
        if (!document.documentElement.hasAttribute("data-file-drag")) {
            document.documentElement.setAttribute("data-file-drag", ""); hover(true);
        }
    });
    const end = () => { document.documentElement.removeAttribute("data-file-drag"); hover(false); };
    window.addEventListener("dragleave", e => { if (!e.relatedTarget) end(); });
    window.addEventListener("drop", e => {
        if (!hasFiles(e)) return;
        e.preventDefault(); end(); onFiles("", e.dataTransfer.files);
    });
    window.addEventListener("paste", e => {
        if (e.target instanceof HTMLInputElement || !e.clipboardData?.files.length) return;
        e.preventDefault(); onFiles("", e.clipboardData.files);
    });
}
export function pick(room) {
    const input = document.createElement("input");
    input.type = "file"; input.multiple = true;
    input.onchange = () => { if (input.files.length) stage(room, input.files); };
    input.click();
}
export async function post_upload(worker, id, file, encrypt) {
    const buffer = await file.arrayBuffer();
    worker.postMessage({command:"wasm-upload",request_id:id,data:"",encrypt,filename:file.name,payload:new Uint8Array(buffer)},[buffer]);
}
export async function save_media(worker, path, encrypted, name) {
    const channel = new MessageChannel();
    const bytes = await new Promise((resolve, reject) => {
        const timer = setTimeout(() => {channel.port1.close(); reject(new Error("Download timed out"));},120000);
        channel.port1.onmessage = ({data}) => {
            clearTimeout(timer); channel.port1.close();
            if (data instanceof Uint8Array) resolve(data); else reject(new Error("Download failed"));
        };
        worker.postMessage({command:"untethered-download",path,encrypted},[channel.port2]);
    });
    const url = URL.createObjectURL(new Blob([bytes],{type:"application/octet-stream"}));
    const link=document.createElement("a");link.href=url;link.download=name || "attachment";
    document.body.append(link);link.click();link.remove();setTimeout(()=>URL.revokeObjectURL(url),30000);
}
