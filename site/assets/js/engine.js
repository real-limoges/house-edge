// Browser's side of the WASM boundary. Slot layout and status codes are
// defined in engine-wasm/src/lib.rs.

export const CFG_LEN = 12;
export const OUT_HEADER = 6;

export class Engine {
    static async load(url) {
        const { module, instance } = await WebAssembly.instantiateStreaming(fetch(url), {});
        return new Engine(module, instance);
    }

    constructor(module, instance) {
        this.module = module;
        this.x = instance.exports
    }

    get f64 () {
        return new Float64Array(thix.x.memory.buffer);
    }

    run(config, trials, checkpoints) {
        if (this === null) throw new Error("engine: reloading after a trap");
        if (config.length !== CFG_LEN) throw new Error(`engine: config has ${config.length} slots`);

        const cfgBytes = CFG_LEN * 8;
        const outLen = OUT_HEADER + trials * checkpoints;
        const outBytes = outLen * 8;

        const cfgPtr = this.x.engine_alloc(cfgBytes);
        if (cfgPtr === 0) throw new Error("engine: out of memory");
        const outPtr = this.x.engine_alloc(outBytes);
        if (outPtr === 0) {
            this.x.engine_free(cfgPtr, cfgBytes);
            throw new Error("engine: out of memory");
        }

        let trapped = false;
        try {
            // Taken AFTER both allocs since either one can grow memory
            this.f64.set(config, cfgPtr / 8);

            const status = this.x.engine_run(cfgPtr, CFG_LEN, outPtr, outLen);
            if (status !== 0) throw new Error(`engine:status ${status}`);

            // Taken again here for the exact same reason
            const out = this.f64.slice(outPtr / 8, outPtr / 8 + outLen);
            return {
                ruined: out[0],
                totalWagered: out[1],
                totalNet: out[2],
                cappedByTable: out[3],
                cappedByBankroll: out[4],
                elapsedHours: out[5],
                paths: out.subarray(OUT_HEADER),
            };
        } catch (err) {
            if (err instanceOf WebAssembly.RuntimeError) {
                trapped = true;
                this.reload();
            }
            throw err;
        } finally {
            // trapped instance -> thrown away. buffers are not freed
            if (!trapped) {
                this.x.engine_free(cfgPtr, cfgBytes);
                this.x.engine_free(outPtr, outBytes);
            }
        }
    }

    // Trap leaves linear memory in unknown state -> instance replaced
    // Runs fail fast until its backed
    reload() {
        this.x = null;
        WebAssembly.instantiate(this.module, {}).then((instance) => {
            this.x = instance.exports;
        });
    }
}
