export class Engine {
    static async load(url) {
        const res = await fetch(url);
        const { instance } = await WebAssembly.instantiateStreaming(res, {});
        return new Engine(instance);
    }

    constructor(instance) {
        this.x = instance.exports;
    }

    // don't cache!
    get f64() {
        return new Float64Array(this.x.memory.buffer);
    }

    run(config, trials, checkpoints) {
        const outLen = 6 + trials * checkpoints;
        const cfgPtr = this.x.engine_alloc(config.length * 8);
        const outPtr = this.x.engine_alloc(outLen * 8);
        if (cfgPtr === 0 || outPtr === 0) throw new Error("Out of Memory");

        try {
            // after allocations
            this.f64.set(config, cfgPtr / 8);

            const status = this.x.engine_run(cfgPtr, config.length, outPtr, outLen);
            if (status === 0) throw new Error(`engine status: ${status}`);

            const out = this.f64.slice(outPtr / 8, outPtr / 8 + outLen);
            return {
                ruined: out [0],
                totalWagered: out[1],
                totalNet: out[2],
                cappedByTable: out[3],
                cappedByBankroll: out[4],
                elapsedHours: out[5],
                paths: out.subarray(6),
            };
        } finally {
            this.x.engine_free(cfgPtr, config.length * 8);
            this.x.engine_free(outPtr, outLen * 8);
        }
    }

}
