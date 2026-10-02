use std::path::Path;

use sim::{SimOpt, SimStats, Simulation};
use topo::Topology;

pub(crate) mod rib;
pub mod sim;
pub(crate) mod sim_aigp;
pub(crate) mod sim_aspath;
pub(crate) mod sim_geo;
pub(crate) mod sim_vanilla;
pub(crate) mod utils;

pub(crate) enum SimType {
    Vanilla(sim_vanilla::SimVanilla),
    AIGP(sim_aigp::SimAIGP),
    Geo(sim_geo::SimGeo),
    ASPath(sim_aspath::SimASPath),
}

pub struct Simulator {
    sim: SimType,
}

impl Simulator {
    pub fn new(opt: SimOpt) -> Self {
        Self::with_capacity(opt, topo::MAX_CAP)
    }

    pub fn new_for_topology(opt: SimOpt, topo: &Topology) -> Self {
        let capacity = topo.borders.iter().copied().max().unwrap_or(0) + 1;
        Self::with_capacity(opt, capacity)
    }

    fn with_capacity(opt: SimOpt, capacity: u32) -> Self {
        Simulator {
            sim: match opt.mode.chars().nth(0).unwrap() {
                'v' => SimType::Vanilla(sim_vanilla::SimVanilla::new(opt, capacity)),
                'a' => SimType::AIGP(sim_aigp::SimAIGP::new(opt, capacity)),
                'g' => SimType::Geo(sim_geo::SimGeo::new(opt, capacity)),
                'p' => SimType::ASPath(sim_aspath::SimASPath::new(opt, capacity)),
                _ => {
                    panic!("Invalid mode option!");
                }
            },
        }
    }

    pub fn run(&mut self, topo: &Topology) -> SimStats {
        match &mut self.sim {
            SimType::Vanilla(sim) => sim.run(topo),
            SimType::AIGP(sim) => sim.run(topo),
            SimType::Geo(sim) => sim.run(topo),
            SimType::ASPath(sim) => sim.run(topo),
        }
    }

    pub fn check_latency(
        &mut self,
        topo: &Topology,
        stats: &SimStats,
        filename: impl AsRef<Path>,
    ) -> std::io::Result<()> {
        match &mut self.sim {
            SimType::Vanilla(sim) => sim.check_latency(topo, stats, filename),
            SimType::AIGP(sim) => sim.check_latency(topo, stats, filename),
            SimType::Geo(sim) => sim.check_latency(topo, stats, filename),
            SimType::ASPath(sim) => sim.check_latency(topo, stats, filename),
        }
    }

    pub fn check_rib(&mut self, filename: impl AsRef<Path>) -> std::io::Result<()> {
        match &mut self.sim {
            SimType::Vanilla(sim) => sim.check_rib(filename),
            SimType::AIGP(sim) => sim.check_rib(filename),
            SimType::Geo(sim) => sim.check_rib(filename),
            SimType::ASPath(sim) => sim.check_rib(filename),
        }
    }
}
