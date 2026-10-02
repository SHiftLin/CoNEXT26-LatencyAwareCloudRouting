use std::net::Ipv4Addr;

use fastmap::FastMap;
use topo::{AsNumber, AsRel, RouterId};

use crate::impl_field;
use crate::rib::{BGPRoute, FullRIBEntry, RIBAttrs, RIBTable};
use crate::sim::{SimOpt, Simulation};

#[derive(Clone, PartialEq, Debug)] // PartialEq and Debug is for test in rib.rs
pub(crate) struct BGPRouteImpl {
    prefix: Ipv4Addr,
    as_path: Vec<AsNumber>,
    next_hop: RouterId,
    router_from: RouterId,
}

#[derive(Clone)]
pub(crate) struct RIBAttrsImpl {
    // as_path: Vec<AsNumber>,
    ebgp: bool,
    metric: u32,
    // as_rel: Option<AsRel>,
    // latency: Option<u32>,
}

#[derive(Clone)] // Clone is for FastMap
pub(crate) struct FullEntry {
    route: BGPRouteImpl,
    attrs: RIBAttrsImpl,
}

impl BGPRoute for BGPRouteImpl {
    impl_field!(prefix, Ipv4Addr);
    impl_field!(as_path, Vec<AsNumber>);
    impl_field!(next_hop, RouterId);
    impl_field!(router_from, RouterId);
}

impl RIBAttrs for RIBAttrsImpl {
    impl_field!(ebgp, bool);
    impl_field!(metric, u32);
    // impl_field!(as_path, Vec<AsNumber>);
    // impl_field!(as_rel, Option<AsRel>);
    // impl_field!(latency, Option<u32>);
}

impl FullRIBEntry for FullEntry {
    type Route = BGPRouteImpl;
    type Attrs = RIBAttrsImpl;
    fn new(
        prefix: Ipv4Addr,
        as_path: Vec<AsNumber>,
        next_hop: RouterId,
        ebgp: bool,
        _as_rel: Option<AsRel>,
    ) -> Self {
        FullEntry {
            route: BGPRouteImpl {
                prefix,
                as_path: as_path.clone(),
                next_hop,
                router_from: next_hop,
            },
            attrs: RIBAttrsImpl {
                ebgp,
                metric: 0,
                // as_path,
                // as_rel,
                // latency: None,
            },
        }
    }

    fn new_with_route(route: Self::Route) -> Self {
        // let as_path = route.as_path.clone();
        FullEntry {
            route,
            attrs: RIBAttrsImpl {
                ebgp: false,
                metric: 0,
                // as_path,
                // as_rel: None,
                // latency: None,
            },
        }
    }
    impl_field!(route, BGPRouteImpl);
    impl_field!(attrs, RIBAttrsImpl);
}
pub(crate) struct SimVanilla {
    opt: SimOpt,
    ribs: FastMap<RIBTable<FullEntry>>,
}

impl SimVanilla {
    pub fn new(opt: SimOpt, capacity: u32) -> Self {
        SimVanilla {
            opt,
            ribs: FastMap::new(capacity),
        }
    }
}

impl Simulation for SimVanilla {
    type Route = BGPRouteImpl;
    type Attrs = RIBAttrsImpl;
    type FullEntry = FullEntry;

    impl_field!(opt, SimOpt);
    impl_field!(ribs, FastMap<RIBTable<FullEntry>>);
}
