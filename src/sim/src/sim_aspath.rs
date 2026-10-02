use std::cmp;
use std::net::Ipv4Addr;

use fastmap::FastMap;
use topo::{get_router, AsNumber, AsRel, Router, RouterId, Topology};

use crate::impl_field;
use crate::rib::{BGPRoute, BGPUpdate, FullRIBEntry, RIBAttrs, RIBTable, RIBUpdate};
use crate::sim::{SimOpt, Simulation};
use crate::utils::quantize_as_path_len;

pub static CAP_PER_PREPEND: u32 = 1024;
pub static CAP_ALL_PREPEND: u32 = 1024;

#[derive(Clone)]
pub(crate) struct BGPRouteImpl {
    // Required fields
    prefix: Ipv4Addr,
    as_path: Vec<AsNumber>,
    next_hop: RouterId,
    router_from: RouterId,
}

#[derive(Clone)]
pub(crate) struct RIBAttrsImpl {
    // Required fields
    ebgp: bool,
    metric: u32,
    // as_path: Vec<AsNumber>,
    // as_rel: Option<AsRel>,
    // latency: Option<u32>,
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

pub(crate) struct SimASPath {
    opt: SimOpt,
    ribs: FastMap<RIBTable<FullEntry>>,
}

impl SimASPath {
    pub fn new(opt: SimOpt, capacity: u32) -> Self {
        SimASPath {
            opt,
            ribs: FastMap::new(capacity),
        }
    }
}

impl Simulation for SimASPath {
    type Route = BGPRouteImpl;
    type Attrs = RIBAttrsImpl;
    type FullEntry = FullEntry;

    impl_field!(opt, SimOpt);
    impl_field!(ribs, FastMap<RIBTable<FullEntry>>);

    fn compare_entry(
        asn: Option<AsNumber>,
        topo: Option<&Topology>,
        new_entry: &Self::FullEntry,
        old_entry: &Self::FullEntry,
        no_pref_enabled: bool,
    ) -> bool {
        // no_pref_enabled is true -> No Preference
        // assert!(!no_pref_enabled);
        match (
            no_pref_enabled,
            Self::compare_asrel(asn, topo, new_entry, old_entry),
        ) {
            (_, cmp::Ordering::Equal) | (true, _) => {
                if new_entry.route().as_path.len() != old_entry.route().as_path.len() {
                    return new_entry.route().as_path.len() < old_entry.route().as_path.len();
                }
                if new_entry.attrs().ebgp() != old_entry.attrs().ebgp() {
                    return *new_entry.attrs().ebgp();
                }
                if new_entry.route().router_from() != old_entry.route().router_from() {
                    return new_entry.route().router_from() < old_entry.route().router_from();
                }
                return false;
            }
            (false, cmp::Ordering::Greater) => false,
            (false, cmp::Ordering::Less) => true,
        }
    }
    fn ibgp_import(
        &mut self,
        topo: &Topology,
        rv: &Router,
        ru: &Router,
        bgp_update: BGPUpdate<Self::Route>,
        no_pref_enabled: bool,
        metric_enabled: bool,
    ) -> RIBUpdate<Self::FullEntry> {
        let _asv = rv.asn.unwrap();
        let _no_bgp_add = self.opt().no_bgp_add;
        let quant = self.opt().quant;
        let rib = self.ribs_mut().get_mut_unsafe(rv.id);
        let mut rib_update = RIBUpdate::new();
        for route in bgp_update.table {
            if *route.router_from() == rv.id {
                continue;
            }
            let mut new_entry = Self::FullEntry::new_with_route(route);
            // let next_hop_as = get_router!(topo, new_entry.route.next_hop).asn.unwrap();
            new_entry.attrs.ebgp = false;
            // new_entry.attrs.as_rel = Some(
            //     topo.as_rel
            //         .get(&(rv.asn.unwrap(), next_hop_as))
            //         .unwrap_or(&AsRel::PeerPeer)
            //         .clone(),
            // );
            let rw = get_router!(topo, new_entry.route.router_from);
            if !rv.reflector && metric_enabled {
                let ilen = quantize_as_path_len(topo.get_weight(rw, rv).unwrap(), quant * 500) - 1; // get_weight returns in one-way, quant factor is Round-Trip
                let ilen = cmp::min(ilen, CAP_PER_PREPEND);
                for _i in 0..ilen {
                    // new_entry.attrs.as_path.push(rv.asn.unwrap());
                    new_entry.route.as_path.push(rv.asn.unwrap());
                }
            }
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

    fn create_ebgp_update(
        &self,
        topo: &Topology,
        ru: &Router,
        rv: &Router,
        entry: &Self::FullEntry,
        metric_enabled: bool,
    ) -> Self::Route {
        let rw = get_router!(topo, entry.route().router_from);
        let mut new_route = entry.route().clone();
        *new_route.next_hop_mut() = ru.id;
        *new_route.router_from_mut() = rv.id;

        if metric_enabled {
            let elen = quantize_as_path_len(
                topo.get_weight(ru, rv).unwrap() + topo.get_weight(ru, rw).unwrap(),
                self.opt().quant * 500,
            );
            // count occurrence of current AS
            let mut new_as_path = vec![];
            for asn in new_route.as_path().iter() {
                if *asn != ru.asn.unwrap() {
                    new_as_path.push(*asn);
                }
            }
            // let elen =
            //     quantize_as_path_len(topo.get_weight(ru, rv).unwrap(), self.opt().quant * 500);

            let total_limit = if new_as_path.len() as u32 >= CAP_ALL_PREPEND {
                0
            } else {
                CAP_ALL_PREPEND - new_as_path.len() as u32
            };
            let elen = cmp::min(elen, total_limit); // limit total prepends to CAP_ALL_PREPEND
                                                    // let elen = if deduct > 0 { elen - 1 } else { elen };
                                                    // get_weight returns in one-way, quant factor is Round-Trip
            let elen = cmp::max(elen, 1); // always prepend at least once if not already present
            let elen = cmp::min(elen, CAP_PER_PREPEND);
            if ru.id == 796389 && rv.id == 25156 {
                println!(
                    "AS{}->AS{} prepend {} times (total AS path len {})",
                    ru.asn.unwrap(),
                    rv.asn.unwrap(),
                    elen,
                    new_as_path.len() + elen as usize
                );
            }
            for _i in 0..elen {
                new_as_path.push(ru.asn.unwrap());
            }
            new_route.as_path_mut().clone_from(&new_as_path);
        } else {
            new_route.as_path_mut().push(ru.asn.unwrap());
        }
        new_route
    }
}
