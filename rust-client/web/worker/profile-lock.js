// The upstream SQLite OPFS pool is exclusive to one worker per origin.
// Hold this lock for the worker's lifetime, not just while opening the DB.
export async function withProfileLock(start) {
    const locks = navigator.locks;
    if (!locks) throw new Error("Web Locks API unavailable");
    const name = "untethered:gomuks-opfs-profile";
    const run = async () => {
        self.postMessage({command: "profile-opening"});
        await start();
    };
    const acquired = await locks.request(name, {ifAvailable: true}, async lock => {
        if (!lock) return false;
        await run();
        return true;
    });
    if (!acquired) {
        self.postMessage({command: "profile-waiting"});
        await locks.request(name, run);
    }
}
