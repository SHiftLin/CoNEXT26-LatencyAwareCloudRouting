use std::cmp;
use std::net::Ipv4Addr;

use fastmap::FastMap;
use topo::{get_router, AsNumber, AsRel, Router, RouterId, Topology};

use crate::impl_field;
use crate::rib::{BGPRoute, BGPUpdate, FullRIBEntry, RIBAttrs, RIBTable, RIBUpdate};
use crate::sim::{SimOpt, Simulation};

const INIT_METRIC: u32 = 0;

#[derive(Clone)]
pub(crate) struct BGPRouteImpl {
    // Required fields
    prefix: Ipv4Addr,
    as_path: Vec<AsNumber>,
    next_hop: RouterId,
    router_from: RouterId,
    // Additional fields
    origin_geo: RouterId, // Should be (f32, f32) in reality to represent lat, lng
}

#[derive(Clone)]
pub(crate) struct RIBAttrsImpl {
    // Required fields
    ebgp: bool,
    // latency: Option<u32>,
    // Additional fields
    // as_path: Vec<AsNumber>,
    // as_rel: Option<AsRel>,
    metric: u32,
}

#[derive(Clone)]
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
                origin_geo: next_hop,
            },
            attrs: RIBAttrsImpl {
                ebgp,
                // as_path,
                // as_rel,
                // latency: None,
                metric: 0,
            },
        }
    }

    fn new_with_route(route: Self::Route) -> Self {
        // let as_path = route.as_path.clone();
        FullEntry {
            route,
            attrs: RIBAttrsImpl {
                ebgp: false,
                // as_path: as_path,
                // as_rel: None,
                // latency: None,
                metric: 0,
            },
        }
    }

    impl_field!(route, BGPRouteImpl);
    impl_field!(attrs, RIBAttrsImpl);
}

pub(crate) struct SimGeo {
    opt: SimOpt,
    ribs: FastMap<RIBTable<FullEntry>>,
}

impl SimGeo {
    pub fn new(opt: SimOpt, capacity: u32) -> Self {
        SimGeo {
            opt,
            ribs: FastMap::new(capacity),
        }
    }
}

impl Simulation for SimGeo {
    type Attrs = RIBAttrsImpl;
    type Route = BGPRouteImpl;
    type FullEntry = FullEntry;

    impl_field!(opt, SimOpt);
    impl_field!(ribs, FastMap<RIBTable<FullEntry>>);

    fn compare_entry(
        asn: Option<AsNumber>,
        topo: Option<&Topology>,
        new_entry: &Self::FullEntry,
        old_entry: &Self::FullEntry,
        _no_pref_enabled: bool,
    ) -> bool {
        match Self::compare_asrel(asn, topo, new_entry, old_entry) {
            cmp::Ordering::Less => true,
            cmp::Ordering::Greater => false,
            cmp::Ordering::Equal => {
                if new_entry.route().as_path().len() != old_entry.route().as_path().len() {
                    return new_entry.route().as_path().len() < old_entry.route().as_path().len();
                }
                if new_entry.attrs().metric != old_entry.attrs().metric {
                    return new_entry.attrs().metric < old_entry.attrs().metric;
                }
                if *new_entry.attrs().ebgp() != *old_entry.attrs().ebgp() {
                    return *new_entry.attrs().ebgp() && !(*old_entry.attrs().ebgp());
                }
                if *new_entry.route().router_from() != *old_entry.route().router_from() {
                    return *new_entry.route().router_from() < *old_entry.route().router_from();
                }
                return false;
            }
        }

        // return new_entry.attrs().metric < old_entry.attrs().metric
        //     || (new_entry.attrs().metric == old_entry.attrs().metric
        //         && new_entry.route().as_path().len() < old_entry.route().as_path().len())
        //     || (new_entry.attrs().metric == old_entry.attrs().metric
        //         && new_entry.route().as_path().len() == old_entry.route().as_path().len()
        //         && *new_entry.attrs().ebgp()
        //         && !(*old_entry.attrs().ebgp()));
    }

    // Reflector may select sub-optimal path when AS1's border router peers with
    // two AS2's border routers and use the same next hop in the BGP updates to AS2 routers,
    // even though BGP Additional path extension is enabled.
    fn ibgp_import(
        &mut self,
        topo: &Topology,
        rv: &Router,
        ru: &Router,
        bgp_update: BGPUpdate<Self::Route>,
        no_pref_enabled: bool,
        metric_enabled: bool,
    ) -> RIBUpdate<Self::FullEntry> {
        let _no_bgp_add = self.opt().no_bgp_add;
        let no_geo_next_hop = self.opt().no_geo_next_hop;
        let rib = self.ribs_mut().get_mut_unsafe(rv.id);
        let mut rib_update = RIBUpdate::new();
        for route in bgp_update.table {
            let rw = get_router!(topo, route.router_from);
            let ro = get_router!(topo, route.origin_geo);

            let mut new_metric = if no_geo_next_hop {
                topo.get_weight(ro, rw).unwrap()
            } else {
                let rn = get_router!(topo, route.next_hop);
                topo.get_weight(ro, rn).unwrap() + topo.get_weight(rn, rw).unwrap()
            };
            if !rv.reflector {
                new_metric += topo.get_weight(rw, rv).unwrap();
            }
            if !metric_enabled {
                new_metric = INIT_METRIC;
            }

            let mut new_entry = Self::FullEntry::new_with_route(route);
            new_entry.attrs.ebgp = false;
            new_entry.attrs.metric = new_metric;
            Self::update_rib(
                topo,
                rv,
                ru,
                rib,
                &mut rib_update,
                new_entry,
                false,
                no_pref_enabled,
            );
        }
        return rib_update;
    }

    // No need to update geo and metric, use default create_ebgp_update()

    fn ebgp_import(
        &mut self,
        topo: &Topology,
        rv: &Router,
        ru: &Router,
        bgp_update: BGPUpdate<Self::Route>,
        no_pref_enabled: bool,
        metric_enabled: bool,
    ) -> RIBUpdate<Self::FullEntry> {
        let no_geo_next_hop = self.opt().no_geo_next_hop;
        let asv = rv.asn.unwrap();
        let rib = self.ribs_mut().get_mut_unsafe(rv.id);
        let mut rib_update = RIBUpdate::new();
        for route in bgp_update.table {
            if route.as_path().contains(&asv) {
                continue;
            }
            let ro = get_router!(topo, route.origin_geo);
            let rw = get_router!(topo, route.router_from);

            let mut new_metric = if no_geo_next_hop {
                topo.get_weight(ro, rw).unwrap()
            } else {
                let rn = get_router!(topo, route.next_hop);
                topo.get_weight(ro, rn).unwrap() + topo.get_weight(rn, rw).unwrap()
            };
            if !metric_enabled {
                new_metric = INIT_METRIC;
            }

            let mut new_entry = Self::FullEntry::new_with_route(route);
            new_entry.attrs.ebgp = true;
            new_entry.attrs.metric = new_metric;

            Self::update_rib(
                topo,
                rv,
                ru,
                rib,
                &mut rib_update,
                new_entry,
                true,
                no_pref_enabled,
            );
        }
        return rib_update;
    }
}
