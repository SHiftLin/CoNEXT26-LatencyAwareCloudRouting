use std::cmp;
use std::net::Ipv4Addr;

use fastmap::FastMap;
use topo::{get_router, AsNumber, AsRel, Router, RouterId, Topology};

use crate::impl_field;
use crate::rib::{BGPRoute, BGPUpdate, FullRIBEntry, RIBAttrs, RIBTable, RIBUpdate};
use crate::sim::{SimOpt, Simulation};
use crate::utils::quantize_aigp;

const INIT_METRIC: u32 = 0;

#[derive(Clone)]
pub(crate) struct BGPRouteImpl {
    // Required fields
    prefix: Ipv4Addr,
    as_path: Vec<AsNumber>,
    next_hop: RouterId,
    router_from: RouterId,
    // Additional fields
    aigp: Option<u32>, // aigp (quantized)
}

#[derive(Clone)]
pub(crate) struct RIBAttrsImpl {
    // Required fields
    ebgp: bool,
    // latency: Option<u32>,
    // Additional fields
    // as_rel: Option<AsRel>,
    // as_path: Vec<AsNumber>,
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
                aigp: Some(quantize_aigp(0)),
            },
            attrs: RIBAttrsImpl {
                ebgp,
                // latency: None,
                // as_path,
                // as_rel,
                metric: INIT_METRIC,
            },
        }
    }
    fn new_with_route(route: Self::Route) -> Self {
        // let as_path = route.as_path.clone();
        FullEntry {
            route: route.clone(),
            attrs: RIBAttrsImpl {
                ebgp: false,
                // as_path,
                // latency: None,
                // as_rel: None,
                metric: route.aigp.unwrap_or(INIT_METRIC),
            },
        }
    }

    impl_field!(route, BGPRouteImpl);
    impl_field!(attrs, RIBAttrsImpl);
}

pub(crate) struct SimAIGP {
    opt: SimOpt,
    ribs: FastMap<RIBTable<FullEntry>>,
}

impl SimAIGP {
    pub fn new(opt: SimOpt, capacity: u32) -> Self {
        SimAIGP {
            opt,
            ribs: FastMap::new(capacity),
        }
    }
}

impl Simulation for SimAIGP {
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
        no_pref_enabled: bool,
    ) -> bool {
        let new_aigp = new_entry.route().aigp;
        let old_aigp = old_entry.route().aigp;
        match (
            no_pref_enabled,
            Self::compare_asrel(asn, topo, new_entry, old_entry),
        ) {
            (_, cmp::Ordering::Equal) | (true, _) => {
                if new_aigp.is_none() && old_aigp.is_some() {
                    return false;
                }
                if new_aigp.is_some() && old_aigp.is_none() {
                    return true;
                }
                if new_entry.attrs().metric != old_entry.attrs().metric {
                    return new_entry.attrs().metric < old_entry.attrs().metric;
                }
                if new_entry.route().as_path().len() != old_entry.route().as_path().len() {
                    return new_entry.route().as_path().len() < old_entry.route().as_path().len();
                }
                if *new_entry.attrs().ebgp() != (*old_entry.attrs().ebgp()) {
                    return *new_entry.attrs().ebgp() && !(*old_entry.attrs().ebgp());
                }
                if *new_entry.route().router_from() != *old_entry.route().router_from() {
                    return *new_entry.route().router_from() < *old_entry.route().router_from();
                }
                return false;
            }
            (false, cmp::Ordering::Less) => true,
            (false, cmp::Ordering::Greater) => false,
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
        // let asv = rv.asn.unwrap();
        // let no_bgp_add = self.opt().no_bgp_add;
        let rib = self.ribs_mut().get_mut_unsafe(rv.id);
        let mut rib_update = RIBUpdate::new();
        for route in bgp_update.table {
            let _prefix = route.prefix;
            if *route.router_from() == rv.id {
                continue;
            }
            let mut new_metric: u32 = route.aigp.unwrap_or(INIT_METRIC);
            if !rv.reflector {
                let rw = get_router!(topo, route.router_from);
                new_metric += quantize_aigp(topo.get_weight(rw, rv).unwrap());
            }
            if !metric_enabled {
                new_metric = INIT_METRIC;
            }

            let mut new_entry = Self::FullEntry::new_with_route(route.clone());
            // let next_hop_as = get_router!(topo, new_entry.route.next_hop).asn.unwrap();
            new_entry.attrs.ebgp = false;
            new_entry.attrs.metric = new_metric;
            new_entry.route.aigp = if metric_enabled {
                if route.aigp.is_some() {
                    Some(new_metric)
                } else {
                    None
                }
            } else {
                None
            };
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
        // Required fields
        let mut new_route = entry.route().clone();
        new_route.as_path_mut().push(ru.asn.unwrap());
        *new_route.next_hop_mut() = ru.id;
        // *new_entry.ebgp_mut() = true;
        *new_route.router_from_mut() = rv.id;

        // Additional fields
        // let rw = get_router!(topo, *entry.route().router_from());
        let cost = topo.get_weight(ru, rv).unwrap();
        // let cost = topo.get_weight(rw, ru).unwrap() + topo.get_weight(ru, rv).unwrap();
        if metric_enabled {
            new_route.aigp = new_route.aigp.map(|aigp| aigp + quantize_aigp(cost));
            // new_route.aigp += quantize_aigp(cost);
        } else {
            new_route.aigp = None
        }
        new_route
    }

    fn ebgp_import(
        &mut self,
        topo: &Topology,
        rv: &Router,
        ru: &Router,
        bgp_update: BGPUpdate<Self::Route>,
        no_pref_enabled: bool,
        metric_enabled: bool,
    ) -> RIBUpdate<Self::FullEntry> {
        // let asv = rv.asn.unwrap();
        let rib = self.ribs_mut().get_mut_unsafe(rv.id);
        let mut rib_update = RIBUpdate::new();
        for route in bgp_update.table {
            // let _prefix = route.prefix;
            let mut new_entry = Self::FullEntry::new_with_route(route.clone());

            // new_entry.attrs.as_rel = Some(
            //     topo.as_rel
            //         .get(&(rv.asn.unwrap(), ru.asn.unwrap()))
            //         .unwrap_or(&AsRel::PeerPeer)
            //         .clone(),
            // );
            new_entry.attrs.ebgp = true;
            new_entry.attrs.metric = if metric_enabled {
                new_entry.route.aigp.unwrap_or(INIT_METRIC) // No need to add igp from rw to rv, since it is 0
            } else {
                INIT_METRIC
            };
            new_entry.route.aigp = if metric_enabled {
                if route.aigp.is_some() {
                    Some(new_entry.attrs.metric)
                } else {
                    None
                }
            } else {
                None
            };
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

            // if let Some(entry) = rib.get_prefix_mut(new_entry.route().prefix()) {
            //     if Self::compare_entry(Some(asv), Some(topo), &new_entry, entry, metric_enabled) {
            //         *entry = new_entry.clone();
            //         rib_update.table.push(new_entry);
            //     }
            // } else {
            //     rib.insert(&ru.id, new_entry.clone());
            //     rib_update.table.push(new_entry);
            // }
        }
        return rib_update;
    }
}
