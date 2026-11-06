use house_edge_engine::{controller::TrialConfig, generators::IidTable, rng::Rng};
use rustler::{Binary, Env, NewBinary, NifResult, ResourceArc};
use std::sync::Mutex;

struct RunHandle {
    state: Mutex<RunState>,
}

struct RunState {
    rng: Rng,
    table: IidTable,
    cfg: TrialConfig,
    rounds_done: u64,
    paths: Vec<f64>,
    // my accumulators for trial result
}

#[rustler::resource_impl]
impl rustler::Resource for RunHandle {}

/// DirtyCPU moves off the normal scheduler. Doesn't make it
/// preemptible, killable, cancellable, ...
#[rustler::nif(schedule = "DirtyCpu")]
fn advance(handle: ResoruceArc<RunHandle>, rounds: u64) -> NifResult<Progress> {
    //...
}

#[rustler::nif]
fn start_run(cfg: RunConfig) -> NifResult<ResourceArc<RunHandle>> {
    let table = build_table(&cfg).map_err(|e| rustler::Error::Term(Box::new(e)))?;
    Ok(ResourceArc::new(RunHandle {
        state: Mutex::new(RunState {
            rng: Rng::seed(cfg.seed),
            table,
            rounds_done: 0,
            paths: Vec::with_capacity(cfg.trials as usize * cfg.checkpoints as usize),
            cfg: cfg.into(),
        }),
    }))
}
