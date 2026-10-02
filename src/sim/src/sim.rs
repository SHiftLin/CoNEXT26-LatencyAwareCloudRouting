use core::fmt;
use std::cmp::{self};
use std::collections::{hash_map, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{Read, Write};
use std::net::Ipv4Addr;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use fastmap::FastMap;
use topo::utils::{decode_latlng, geo_distance, LatLng};
use topo::{get_router, AsNumber, AsRel, Router, RouterId, Topology};

use crate::def_field;
use crate::rib::{
    BGPRoute, BGPUpdate, FullRIBEntry, RIBAttrs, RIBTable, RIBUpdate, RIBUpdateEntry,
};
use crate::utils::print_top_count;

pub struct SimOpt {
    pub mode: String,
    pub prefix: Ipv4Addr,
    pub origin: AsNumber,
    pub origin_locs: Option<Vec<LatLng>>,
    pub rounds: usize,
    pub no_bgp_add: bool,
    pub no_geo_next_hop: bool,
    pub no_pref_scope: String,
    pub metric_scope: String,
    pub quant: u32,
}

pub struct SimStats {
    pub round_cnt: usize,
    pub ibgp_cnt: usize,
    pub ebgp_cnt: usize,
    pub init_routers: Vec<RouterId>,
    pub covered_routers: HashSet<RouterId>,
    pub covered_asns: HashSet<AsNumber>,
}

impl SimStats {
    pub fn new() -> Self {
        SimStats {
            round_cnt: 0,
            ibgp_cnt: 0,
            ebgp_cnt: 0,
            init_routers: Vec::new(),
            covered_routers: HashSet::new(),
            covered_asns: HashSet::new(),
        }
    }

    pub fn msg_cnt(&self) -> usize {
        self.ibgp_cnt + self.ebgp_cnt
    }
}

impl fmt::Display for SimStats {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Round {}\n", self.round_cnt)?;
        write!(
            f,
            "covered routers: {}, covered ASNs: {}, messages:{}, ibgp messages: {}, ebgp messages: {}",
            self.covered_routers.len(),
            self.covered_asns.len(),
            self.msg_cnt(),
            self.ibgp_cnt,
            self.ebgp_cnt,
        )?;
        Ok(())
    }
}

pub(crate) trait Simulation {
    type Route: BGPRoute + Clone;
    type Attrs: RIBAttrs + Clone;
    type FullEntry: FullRIBEntry<Route = Self::Route, Attrs = Self::Attrs> + Clone;

    def_field!(opt, SimOpt);
    def_field!(ribs, FastMap<RIBTable<Self::FullEntry>>);

    fn compare_asrel(
        asn: Option<AsNumber>,
        topo: Option<&Topology>,
        new_entry: &Self::FullEntry,
        old_entry: &Self::FullEntry,
    ) -> cmp::Ordering {
        match (asn, topo) {
            (Some(asn), Some(topo)) => {
                // let new_next_hop_as = get_router!(topo, *new_entry.route().next_hop())
                //     .asn
                //     .unwrap();

                // let old_next_hop_as = get_router!(topo, *old_entry.route().next_hop())
                //     .asn
                //     .unwrap();

                let new_next_hop_as = *new_entry
                    .route()
                    .as_path()
                    .iter()
                    .rfind(|&x| *x != asn)
                    .unwrap_or(&0);
                // let old_next_hop_as = *old_entry.route().as_path().last().unwrap();
                let old_next_hop_as = *old_entry
                    .route()
                    .as_path()
                    .iter()
                    .rfind(|&x| *x != asn)
                    .unwrap_or(&0);
                let new_rel = topo
                    .as_rel
                    .get(&(asn, new_next_hop_as))
                    .unwrap_or(&AsRel::None);
                let old_rel = topo
                    .as_rel
                    .get(&(asn, old_next_hop_as))
                    .unwrap_or(&AsRel::None);
                // if new_next_hop_as != old_next_hop_as {
                // println!("asn={}, new_next_hop_as={:?}, old_next_hop_as={:?}, new_rel={:?}, old_rel={:?}", asn, new_next_hop_as, old_next_hop_as,   new_rel, old_rel);
                // }
                // let new_rel = new_entry.attrs().as_rel().as_ref().unwrap();
                // let old_rel = old_entry.attrs().as_rel().as_ref().unwrap();
                new_rel.cmp(old_rel)
            }
            _ => cmp::Ordering::Equal, // Should only be (None, None), but in case something weird happens
        }
    }
    fn compare_entry(
        asn: Option<AsNumber>,
        topo: Option<&Topology>,
        new_entry: &Self::FullEntry,
        old_entry: &Self::FullEntry,
        no_pref_enabled: bool,
    ) -> bool {
        // println!("compare entry");
        // println!("new_entry.as_path: {:?}.", new_entry);
        match (
            no_pref_enabled,
            Self::compare_asrel(asn, topo, new_entry, old_entry),
        ) {
            (false, cmp::Ordering::Less) => true,
            (false, cmp::Ordering::Greater) => false,
            (_, cmp::Ordering::Equal) | (true, _) => {
                if new_entry.route().as_path().len() != old_entry.route().as_path().len() {
                    return new_entry.route().as_path().len() < old_entry.route().as_path().len();
                }
                if *new_entry.attrs().ebgp() != *old_entry.attrs().ebgp() {
                    return *new_entry.attrs().ebgp() && !(*old_entry.attrs().ebgp());
                }
                if new_entry.attrs().metric() != old_entry.attrs().metric() {
                    return new_entry.attrs().metric() < old_entry.attrs().metric();
                }
                if *new_entry.route().router_from() < *old_entry.route().router_from() {
                    return true;
                }
                return false;
            }
        }
    }

    fn create_ibgp_update(
        &self,
        _topo: &Topology,
        _ru: &Router,
        _rv: &Router,
        entry: &Self::FullEntry,
    ) -> <Self::FullEntry as FullRIBEntry>::Route {
        entry.route().clone()
    }

    fn ibgp_export(
        &self,
        topo: &Topology,
        ru: &Router,
        rv: &Router,
        table: &RIBUpdate<Self::FullEntry>,
        _metric_enabled: bool, // not used
    ) -> BGPUpdate<<Self::FullEntry as FullRIBEntry>::Route> {
        let mut exported = BGPUpdate::new();
        for update in &table.table {
            let entry = &update.entry;
            // Route reflector will forward the route back to the router where it learned from
            // /*
            if *entry.route().router_from() == rv.id {
                continue;
            }
            // */
            // Should not propagate if entry is from ibgp and u is not reflector and v is reflector
            if *entry.attrs().ebgp() || topo.is_reflector(&ru) {
                exported
                    .table
                    .push(self.create_ibgp_update(topo, ru, rv, entry));
            }
        }
        return exported;
    }
    fn ebgp_loop(as_path: &Vec<AsNumber>, asn: &AsNumber) -> bool {
        // println!("ebgp_loop");
        // println!("as_path: {:?}", as_path);
        let mut n = as_path.len();
        while (n > 0) && (as_path[n - 1] == *asn) {
            n -= 1;
        }
        for i in 0..n {
            if as_path[i] == *asn {
                return true;
            }
        }
        false
    }
    fn update_rib(
        topo: &Topology,
        rv: &Router,
        _ru: &Router,
        rib: &mut RIBTable<<Self as Simulation>::FullEntry>,
        rib_update: &mut RIBUpdate<<Self as Simulation>::FullEntry>,
        new_entry: Self::FullEntry,
        is_ebgp: bool,
        no_pref_enabled: bool,
    ) {
        let debug = false; // rv.id == 3407 && *new_entry.route().router_from() == 403845;
        let asv = rv.asn.unwrap();
        let next_hop = *new_entry.route().next_hop();
        let router_from = *new_entry.route().router_from();
        let prefix = new_entry.route().prefix();

        let nlri = (next_hop, router_from);

        // if rv.id == 103203622 {
        //     println!("install new_entry: (ru.id, router_from) = {:?} route.as_path: {:?} route.as_path_len: {:?} route.next_hop: {:?}, route.router_from: {:?}, attrs.ebgp: {:?}",
        //     nlri,
        //         *new_entry.route().as_path(),
        //         new_entry.route().as_path().len(),
        //         *new_entry.route().next_hop(),
        //         *new_entry.route().router_from(),
        //         *new_entry.attrs().ebgp(),
        //     );

        //     println!(
        //         "RIB@rv={}, current best option={:?}",
        //         rv.id,
        //         rib.get_loc_rib_nlri(prefix)
        //     );
        //     if let Some(all) = rib.get_adj_rib_all(prefix) {
        //         for (nlri, entry) in all {
        //             if Self::ebgp_loop(entry.route().as_path(), &asv) {
        //                 continue;
        //             }
        //             // if nlri.0 == 3407 || nlri.0 == 28539 {
        //             println!("entry: nlri = {:?} route.as_path: {:?} route.as_path_len: {:?} route.next_hop: {:?}, route.router_from: {:?}, attrs.ebgp: {:?}",
        //                 nlri,
        //                 *entry.route().as_path(),
        //                 entry.route().as_path().len(),
        //                 *entry.route().next_hop(),
        //                 *entry.route().router_from(),
        //                 *entry.attrs().ebgp());
        //             //
        //         }
        //     } else {
        //         println!("no entry in adj_rib_all");
        //     }
        //     println!("------");
        // }
        let implicit_withdraw = if let Some(entry) = rib.get_adj_rib_by_nlri_mut(prefix, &nlri) {
            // implicit withdraw
            *entry = new_entry.clone();
            true
        } else {
            rib.insert_adj_rib_in(prefix, &nlri, new_entry.clone());
            false
        };

        if rv.reflector {
            rib_update.table.push(RIBUpdateEntry {
                add: true,
                entry: new_entry.clone(),
            });
            assert!(!is_ebgp);
            return;
        }

        if let Some(loc_nlri) = rib.get_loc_rib_nlri(prefix) {
            if *loc_nlri == nlri {
                if debug {
                    println!(
                        "case 1: reonstruct loc_rib: {:?}, nlri: {:?}",
                        loc_nlri, nlri
                    );
                }
                // reconstruct local rib entry
                let all_entry = rib.get_adj_rib_all(prefix).unwrap(); // unwrap() note: should at least have the new entry in the table
                let mut best_nlri = None;
                let mut best_entry = None;
                // let keys: Vec<_> = all_entry.keys().collect();
                // keys.sort();

                for (key, entry) in all_entry.iter() {
                    if Self::ebgp_loop(entry.route().as_path(), &asv) {
                        continue;
                    }
                    if let Some(ref best) = best_entry {
                        if Self::compare_entry(
                            Some(asv),
                            Some(&topo),
                            &entry,
                            best,
                            no_pref_enabled,
                        ) {
                            best_entry = Some(entry.clone());
                            best_nlri = Some(*key);
                        }
                    } else {
                        best_entry = Some(entry.clone());
                        best_nlri = Some(*key);
                    }
                }
                // if best_

                // if rv.id == 16472 {
                //     println!("reconstruct==>{:?}", best_nlri.clone());
                // }

                if best_nlri != Some(nlri) {
                    assert!(implicit_withdraw);
                    if is_ebgp {
                        rib_update.table.push(RIBUpdateEntry {
                            add: true,
                            entry: new_entry.clone(),
                        });
                    }
                }
                if let Some(best_nlri) = best_nlri {
                    rib.loc_rib.insert(*prefix, best_nlri);
                    rib_update.table.push(RIBUpdateEntry {
                        add: false,
                        entry: best_entry.unwrap(),
                    });
                }
            } else {
                if debug {
                    println!("case 2: loc_nlri: {:?}, nlri: {:?}", loc_nlri, nlri);
                }
                // if rib.get_adj_rib_by_nlri(prefix, loc_nlri).is_none() {
                //     println!("None!")
                // }
                // if let Some(map) = rib.get_adj_rib_all(prefix) {
                //     println!("capacity={}", map.capacity());
                //     for ((next_hop, router_from), entry) in map {
                //         println!(
                //             "entry: next_hop = {}, router_from = {}, route.as_path: {:?} ",
                //             next_hop,
                //             router_from,
                //             *entry.attrs().as_path()
                //         );
                //     }
                // }
                // println!(
                //     "rv.id: {:?}, loc_nlri: {:?}, nlri: {:?}, loc_nlri != nlri",
                //     rv.id, loc_nlri, nlri
                // );
                let entry = rib.get_loc_rib_entry(prefix).unwrap();
                // unwrap() note: loc_opt entry is an index that should not point to non-exist entry
                let better = Self::compare_entry(
                    Some(asv),
                    Some(&topo),
                    &new_entry,
                    &entry,
                    no_pref_enabled,
                );
                let ebgp_loop = Self::ebgp_loop(new_entry.route().as_path(), &asv);
                if !(better && !ebgp_loop) && implicit_withdraw {
                    // if the new entry is no better than the old local best
                    // and the implicit withdraw happens
                    // br need to inform rr about the implicit withdraw
                    rib_update.table.push(RIBUpdateEntry {
                        add: true,
                        entry: new_entry.clone(),
                    });
                }

                if better && !ebgp_loop {
                    rib.loc_rib.insert(*prefix, nlri);
                    rib_update.table.push(RIBUpdateEntry {
                        add: false,
                        entry: new_entry,
                    });
                }
            }
        } else {
            if Self::ebgp_loop(new_entry.route().as_path(), &asv) {
                return;
            }
            // println!(
            //     "rv.id: {:?}, loc_nlri: None, nlri: {:?}, loc_nlri != nlri",
            //     rv.id, nlri
            // );
            rib.loc_rib.insert(*prefix, nlri);
            rib_update.table.push(RIBUpdateEntry {
                add: false,
                entry: new_entry,
            });
        }
        // if rv.id == 103203622 {
        //     if rib_update.table.len() > 0 {
        //         println!("RIB Update for rv.id: {}", rv.id);
        //         for update in &rib_update.table {
        //             let entry = &update.entry;
        //             println!("update: nlri = {:?} route.as_path: {:?} route.as_path_len: {:?} route.next_hop: {:?}, route.router_from: {:?}, attrs.ebgp: {:?}",
        //             nlri,
        //             *entry.route().as_path(),
        //             entry.route().as_path().len(),
        //             *entry.route().next_hop(),
        //             *entry.route().router_from(),
        //             *entry.attrs().ebgp());
        //         }
        //     }
        // }
    }

    fn ibgp_import(
        &mut self,
        topo: &Topology,
        rv: &Router,
        ru: &Router,
        bgp_update: BGPUpdate<Self::Route>,
        no_pref_enabled: bool,
        _metric_enabled: bool, // not used in vanilla
    ) -> RIBUpdate<Self::FullEntry> {
        let _no_bgp_add = self.opt().no_bgp_add;
        let rib = self.ribs_mut().get_mut_unsafe(rv.id);
        let mut rib_update = RIBUpdate::new();
        for route in bgp_update.table {
            let mut new_entry = Self::FullEntry::new_with_route(route);

            if *new_entry.route().router_from() == rv.id {
                continue;
            }
            *new_entry.attrs_mut().ebgp_mut() = false;
            let rw = get_router!(topo, *new_entry.route().router_from());
            *new_entry.attrs_mut().metric_mut() = topo.get_weight(rw, rv).unwrap_or(0) as u32;
            // *new_entry.attrs_mut().as_rel_mut() = Some(
            //     topo.as_rel
            //         .get(&(rv.asn.unwrap(), next_hop_as))
            //         .unwrap_or(&AsRel::PeerPeer)
            //         .clone(),
            // );

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
        _topo: &Topology,
        ru: &Router,
        rv: &Router,
        entry: &Self::FullEntry,
        _metric_enabled: bool,
    ) -> Self::Route {
        let mut new_route = entry.route().clone();
        new_route.as_path_mut().push(ru.asn.unwrap());
        new_route.as_path_mut().shrink_to_fit();
        *new_route.next_hop_mut() = ru.id;
        *new_route.router_from_mut() = rv.id;
        new_route
    }

    fn ebgp_export(
        &self,
        topo: &Topology,
        ru: &Router,
        rv: &Router,
        table: &RIBUpdate<Self::FullEntry>,
        metric_enabled: bool,
    ) -> BGPUpdate<Self::Route> {
        let asu = ru.asn.unwrap();
        let asv = rv.asn.unwrap();
        let as_rel = topo.as_rel.get(&(asu, asv)).unwrap_or(&AsRel::PeerPeer);

        let mut exported = BGPUpdate::new();
        for update in &table.table {
            let entry = &update.entry;
            if update.add {
                // Add path will only be applicable in iBGP
                continue;
            }
            // if *entry.attrs().ebgp() && ru.ebgp_cnt == 1 && *entry.route().next_hop() != ru.id {
            //     if rv.id == 103203140 {
            //         println!(
            //             "Triggered: ru.id: {}, asu: {}, asv: {}, *entry.route().next_hop(): {}",
            //             ru.id,
            //             asu,
            //             asv,
            //             *entry.route().next_hop()
            //         );
            //     }
            //     // To reproduce the Cisco IOS bug that propagate back
            //     // the update message when ebgp_cnt > 1
            //     continue;
            // }
            let rel_prop = match as_rel {
                AsRel::None => true, // unknown or asu == asv
                AsRel::ProviderCustomer => true,
                _ => {
                    let mut as_from_opt = None;
                    for &asn in entry.route().as_path().iter().rev() {
                        if asn != asu {
                            as_from_opt = Some(asn);
                            break;
                        }
                    }
                    if let Some(as_from) = as_from_opt {
                        match topo.as_rel.get(&(asu, as_from)).unwrap_or(&AsRel::PeerPeer) {
                            AsRel::ProviderCustomer => true,
                            _ => false,
                        }
                    } else {
                        true
                    }
                }
            };
            if rel_prop {
                exported
                    .table
                    .push(self.create_ebgp_update(topo, ru, rv, entry, metric_enabled));
            }
        }
        return exported;

        /*
        if topo.provider_customer.contains(&(asu, asv)) {
            for route in &table.table {
                if *v == route.next_hop {
                    continue;
                }
                exported.table.push(*route);
            }
        } else if topo.provider_customer.contains(&(asv, asu))
            || topo.peer_peer.contains(&(asu, asv))
        {
            for route in &table.table {
                if *v == route.next_hop {
                    continue;
                }
                if asu == route.as_from || topo.provider_customer.contains(&(asu, route.as_from)) {
                    exported.table.push(*route);
                }
            }
        }
        */
    }

    fn ebgp_import(
        &mut self,
        topo: &Topology,
        rv: &Router,
        ru: &Router,
        bgp_update: BGPUpdate<Self::Route>,
        no_pref_enabled: bool,
        _metric_enabled: bool,
    ) -> RIBUpdate<Self::FullEntry> {
        let rib = self.ribs_mut().get_mut_unsafe(rv.id);
        let mut rib_update = RIBUpdate::new();
        for route in bgp_update.table {
            // if route.as_path().contains(&asv) {
            //     continue;
            // }
            let mut new_entry = Self::FullEntry::new_with_route(route);
            *new_entry.attrs_mut().ebgp_mut() = true;
            // *new_entry.attrs_mut().as_rel_mut() = Some(
            //     topo.as_rel
            //         .get(&(rv.asn.unwrap(), ru.asn.unwrap()))
            //         .unwrap_or(&AsRel::PeerPeer)
            //         .clone(),
            // );
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

    fn init_borders(&mut self, topo: &Topology) -> VecDeque<(u32, RIBUpdate<Self::FullEntry>)> {
        let prefix = self.opt().prefix;
        let origin = self.opt().origin;
        let mut origin_locs = self.opt().origin_locs.clone();
        let mut q = VecDeque::new();
        let mut borders = topo.as_borders.get_unsafe(origin).clone();
        borders.sort();
        borders.reverse(); // Fix the iteration order to get the same result in multi-run
        let mut min_dist = f64::MAX;
        for &border in &borders {
            let router = get_router!(topo, border);
            if topo.is_switch(&border) || topo.is_reflector(&router) {
                continue;
            }

            if origin_locs.is_some() {
                let locs = origin_locs.as_mut().unwrap();
                let router_latlng = decode_latlng(router.latlng.unwrap());
                let mut idx = None;
                for (i, &latlng) in locs.iter().enumerate() {
                    let dist = geo_distance(latlng, router_latlng);
                    if dist < min_dist {
                        min_dist = dist;
                    }
                    if dist <= 50.0 * 1000.0 {
                        idx = Some(i);
                        break;
                    }
                }
                if idx.is_none() {
                    continue;
                }
                locs.swap_remove(idx.unwrap());
            }

            let entry = Self::FullEntry::new(prefix, vec![], border, true, None);
            let mut table = RIBTable::new();
            table.loc_rib.insert(prefix, (border, border));
            table.insert_adj_rib_in(&prefix, &(border, border), entry.clone());
            self.ribs_mut().insert(border, table);
            q.push_back((
                border,
                RIBUpdate {
                    table: vec![RIBUpdateEntry { add: false, entry }],
                },
            ));

            if origin_locs.is_none() {
                println!(
                    "Origin lat lng: {:?}",
                    // decode_latlng(router.latlng.unwrap())
                    router.latlng
                );
                break; // Start with one router and will send iBGP to other routers in the AS
            }
        }
        println!("Min Distance: {}", min_dist);
        q
    }

    fn init_asn_scope(&self, scope_path: &str, topo: &Topology) -> HashSet<AsNumber> {
        match scope_path {
            "all" => topo.router_as_list.clone(),
            _ => {
                // read from scope_path
                let mut content = String::new();
                if let Ok(mut file) = File::open(Path::new(scope_path)) {
                    file.read_to_string(&mut content).unwrap();
                    let mut enabled_asns: HashSet<_> = content
                        .trim()
                        .split(',')
                        .filter_map(|s| s.trim().parse::<u32>().ok())
                        .collect();
                    // origin AS will be in the scope in addition to non-empty given scope
                    // Though it is not necessary in no_pref_scope, but it does not make a difference
                    if !enabled_asns.is_empty() {
                        enabled_asns.insert(self.opt().origin);
                    }
                    enabled_asns
                } else {
                    panic!("Cannot open file {}", scope_path);
                }
            }
        }
    }

    fn bgp_neighbors<'a>(
        topo: &Topology,
        ru: &Router,
        uvs: &'a mut HashMap<u32, Vec<RouterId>>,
    ) -> &'a Vec<RouterId> {
        let u = ru.id;
        match uvs.entry(u) {
            hash_map::Entry::Occupied(entry) => entry.into_mut(),
            hash_map::Entry::Vacant(entry) => {
                let mut vs = BTreeSet::new(); // Use BTreeSet to ensure non-duplicate and keep the iteration order
                for &v in &ru.border_links {
                    let rv = get_router!(topo, v);
                    if topo.is_switch(&v) {
                        assert!(rv.asn == Some(topo::HUB_ASN) || rv.asn == Some(topo::IXP_ASN));
                        log::debug!("switch {} {}", u, v);
                        for &w in &rv.border_links {
                            if w != u {
                                // Ensure that w is not a switch
                                if topo.is_switch(&w) {
                                    panic!("w is switch {}", w);
                                }
                                let rw = get_router!(topo, w);
                                if topo.get_weight(&ru, &rw).is_some() {
                                    if ru.asn != rw.asn {
                                        vs.insert(w);
                                    }
                                }
                            }
                        }
                    } else {
                        vs.insert(v);
                    }
                }
                log::debug!("u: {}, vs: {:?}", u, vs);
                let vs: Vec<_> = vs.into_iter().collect();
                // if !ru.reflector {
                //  vs.shuffle(&mut thread_rng());
                // }
                entry.insert(vs)
            }
        }
    }

    fn run(&mut self, topo: &Topology) -> SimStats {
        let mut stats = SimStats::new();
        let prefix = self.opt().prefix;
        let origin = self.opt().origin;
        println!("Annouce {} origin from AS {}", prefix, origin);

        let no_pref_scope = self.init_asn_scope(&self.opt().no_pref_scope, topo);
        let metric_scope = self.init_asn_scope(&self.opt().metric_scope, topo);

        println!(
            "{} no_pref_scope: #AS={:?}",
            self.opt().no_pref_scope,
            no_pref_scope.len(),
        );
        println!(
            "{} metric_scope: #AS={:?}",
            self.opt().metric_scope,
            metric_scope.len(),
        );

        let mut q = self.init_borders(topo);
        for (u, _) in &q {
            stats.init_routers.push(*u);
            stats.covered_routers.insert(*u);
        }
        stats.covered_asns.insert(origin);
        println!("{}", stats);
        println!("{:?}", stats.init_routers);
        let mut uvs = HashMap::new();
        loop {
            let mut newq = VecDeque::new();
            log::debug!(
                "queue: {} {:?}",
                q.len(),
                q.iter().map(|(a, _b)| *a).collect::<Vec<u32>>()
            );

            stats.round_cnt += 1;
            let mut asn_ibgp_cnt: HashMap<u32, usize> = HashMap::new();
            let mut router_ibgp_cnt: HashMap<(u32, u32), usize> = HashMap::new();
            let mut router_ebgp_cnt: HashMap<(u32, u32), usize> = HashMap::new();
            let mut asn_ebgp_cnt: HashMap<(u32, u32), usize> = HashMap::new();
            let mut router_ibgp_enqueued: HashMap<u32, usize> = HashMap::new();
            // let mut v_cnt: HashMap<_, usize> = HashMap::new();

            println!("ibgp round {}", stats.round_cnt);

            let mut start = Instant::now();
            while !q.is_empty() {
                if start.elapsed().as_secs() > 300 {
                    println!(
                        "Time={}",
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs()
                    );
                    println!(
                        "rounds={}, q.len() = {}, nq.len() = {}",
                        stats.round_cnt,
                        q.len(),
                        newq.len()
                    );
                    println!(
                        "covered routers: {}, covered ASNs: {}",
                        stats.covered_routers.len(),
                        stats.covered_asns.len()
                    );

                    print_top_count("ibgp asn cnt:", &asn_ibgp_cnt);
                    print_top_count("ibgp router cnt:", &router_ibgp_cnt);
                    print_top_count("router enqueue cnt:", &router_ibgp_enqueued);

                    // let mut num_entries: usize = 0;
                    let mut total_capacity: usize = 0;
                    // let mut rib_size: usize = 0;
                    let mut distribution = HashMap::new();
                    for rib in self.ribs().values() {
                        for (_prefix, map) in rib.clone().adj_rib_in.into_iter() {
                            total_capacity += map.capacity();
                            distribution
                                .entry(map.capacity())
                                .and_modify(|c| *c += 1)
                                .or_insert(1);
                            // for (_nlri, entry) in map {
                            //     num_entries += 1;
                            //     rib_size += entry.attrs().as_path().len();
                            // }
                        }
                    }
                    // println!("#entries in RIB: {}", num_entries);
                    println!("total capacity: {}", total_capacity);
                    // println!("rib size: {}", rib_size);
                    print_top_count("capacity distribution:", &distribution);
                    start = Instant::now();
                }
                let (u, update) = q.pop_front().unwrap();
                let ru = get_router!(topo, u);
                let asu = ru.asn.unwrap();
                let asu_no_pref_enabled = no_pref_scope.contains(&asu);
                let asu_metric_enabled = metric_scope.contains(&asu);
                let vs = Self::bgp_neighbors(topo, ru, &mut uvs);

                // println!("ru: {}, vs: {:?}", u, vs);
                for &v in vs {
                    let rv = get_router!(topo, v);
                    let asv = rv.asn.unwrap();
                    if asu == asv {
                        let exported = self.ibgp_export(topo, ru, rv, &update, asu_metric_enabled);
                        if exported.table.len() > 0 {
                            // println!("ibgp: {}({}) -> {}({})", u, ru.reflector, v, rv.reflector);
                            // log::debug!("ibgp {} {}", u, v);
                            stats.ibgp_cnt += 1;
                            router_ibgp_cnt
                                .entry((u, v))
                                .and_modify(|c| *c += 1)
                                .or_insert(1);
                            asn_ibgp_cnt.entry(asu).and_modify(|c| *c += 1).or_insert(1);

                            let new_update = self.ibgp_import(
                                topo,
                                rv,
                                ru,
                                exported,
                                asu_no_pref_enabled,
                                asu_metric_enabled,
                            );
                            if new_update.table.len() > 0 {
                                q.push_back((v, new_update));
                                stats.covered_routers.insert(v);
                                router_ibgp_enqueued
                                    .entry(v)
                                    .and_modify(|c| *c += 1)
                                    .or_insert(1);
                                // self.covered_asns.insert(asv);
                            }
                        }
                    }
                }

                if !ru.reflector {
                    newq.push_back((u, update));
                }
            }

            q = newq;
            let mut newq = VecDeque::new();
            println!("ebgp round {}", stats.round_cnt);

            while !q.is_empty() {
                let (u, update) = q.pop_front().unwrap();
                let ru = get_router!(topo, u);
                let asu = ru.asn.unwrap();
                let _asu_no_pref_enabled = no_pref_scope.contains(&asu);
                let asu_metric_enabled = metric_scope.contains(&asu);
                let vs = Self::bgp_neighbors(topo, ru, &mut uvs);

                for &v in vs {
                    let rv = get_router!(topo, v);

                    let asv = rv.asn.unwrap();
                    let asv_no_pref_enabled = no_pref_scope.contains(&asv);
                    let asv_metric_enabled = metric_scope.contains(&asv);
                    if asu != asv {
                        let exported = self.ebgp_export(topo, ru, rv, &update, asu_metric_enabled);

                        if exported.table.len() > 0 {
                            // log::debug!("ebgp {} {}", u, v);
                            // println!("ebgp: {}:{} -> {}:{}", u, asu, v, asv);
                            stats.ebgp_cnt += 1;
                            asn_ebgp_cnt
                                .entry((asu, asv))
                                .and_modify(|c| *c += 1)
                                .or_insert(1);
                            router_ebgp_cnt
                                .entry((u, v))
                                .and_modify(|c| *c += 1)
                                .or_insert(1);

                            let new_update = self.ebgp_import(
                                topo,
                                rv,
                                ru,
                                exported,
                                asv_no_pref_enabled,
                                asv_metric_enabled,
                            );

                            if new_update.table.len() > 0 {
                                // v_cnt.entry(v).and_modify(|c| *c += 1).or_insert(1);
                                newq.push_back((v, new_update));
                                stats.covered_routers.insert(v);
                                stats.covered_asns.insert(asv);
                            }
                        }
                    }
                }
            }
            if stats.round_cnt % 1 == 0 {
                println!("{}", stats);
                print_top_count("ibgp asn cnt:", &asn_ibgp_cnt);
                print_top_count("ebgp asn cnt:", &asn_ebgp_cnt);
                println!("ebgp router links size={:?}", router_ebgp_cnt.len());
                print_top_count("ebgp router cnt:", &router_ebgp_cnt);
                for (k, v) in &asn_ebgp_cnt {
                    if k.1 == 16625 {
                        println!("router_ebgp_cnt: {:?} {:?}", k, v);
                    }
                }
                // print_top_count("v cnt:", &v_cnt);
                // // let mut num_entries: usize = 0;
                // // let mut total_capacity: usize = 0;
                // // let mut rib_size: usize = 0;
                // // let mut distribution = HashMap::new();
                // // for rib in self.ribs().values() {
                // //     for (_prefix, map) in rib.clone().adj_rib_in.into_iter() {
                // //         total_capacity += map.capacity();
                // //         distribution
                // //             .entry(map.capacity())
                // //             .and_modify(|c| *c += 1)
                // //             .or_insert(1);
                // //         for (_nlri, entry) in map {
                // //             num_entries += 1;
                // //             rib_size += entry.attrs().as_path().len();
                // //         }
                // //     }
                // // }
                // // println!("#entries in RIB: {}", num_entries);
                // // println!("total capacity: {}", total_capacity);
                // // println!("rib size: {}", rib_size);
                // print_top_count("capacity distribution:", &distribution);
            }
            if newq.len() == 0 || self.opt().rounds == stats.round_cnt {
                // run until convergence if opt.rounds == 0
                break;
            }
            q = newq;
        }
        stats
    }

    fn calc_latency(&mut self, topo: &Topology, stats: &SimStats) -> Vec<Option<u32>> {
        // Should not calculate the latency during the BGP propagation, which may be error.
        // For example, in SimGeo, Router A received a eBGP update from Router B,
        // but later B receives a new route with smaller latency and it propagates to A.
        // By design of SimGeo, A does not update its RIB with new route since the next hop is still B,
        // Thus, A cannot update the latency from the new route.
        // Besides, the reflector in our simulator is synthesized without location.
        // We consider the delay inside an AS as the direct latency between two border routers
        // without the reflector.
        let start = std::time::Instant::now();
        let prefix = self.opt().prefix;
        let mut stack = Vec::with_capacity(32);
        let max_router_id = *stats.covered_routers.iter().max().unwrap();
        let mut latency = vec![None; max_router_id as usize + 1];

        for u in &stats.init_routers {
            latency[*u as usize] = Some(0);
        }
        let covered_routers = {
            let mut sorted = stats
                .covered_routers
                .clone()
                .into_iter()
                .collect::<Vec<_>>();
            sorted.sort();
            sorted
        };
        for u in covered_routers {
            if get_router!(topo, u).reflector {
                continue;
            }

            stack.clear();
            let mut v = u;
            // println!("router_u={}", u);
            loop {
                let best_entry = self.ribs().get_unsafe(v).get_loc_rib_entry(&prefix);
                let (next_hop, router_from) = if let Some(entry) = best_entry {
                    (*entry.route().next_hop(), *entry.route().router_from())
                } else {
                    break;
                };
                if latency[v as usize].is_some() {
                    break;
                }
                stack.push(v);
                if stack.len() > 32 {
                    println!("router_u={}, router_v={}", u, v);
                    for &p in &stack {
                        println!("router_p={}, asn={:?}", p, get_router!(topo, p).asn);
                        let nexthop = self.ribs().get_unsafe(p).get_loc_rib_ru(&prefix).unwrap();
                        // let entry: &<Self as Simulation>::Entry = self
                        //     .ribs()
                        //     .get_unsafe(p)
                        //     .get_loc_rib_entry(&prefix)
                        //     .unwrap();
                        let all_entries =
                            self.ribs().get_unsafe(p).get_adj_rib_all(&prefix).unwrap();
                        println!("opt (nexthop, ru)={:?}", nexthop);
                        for (nexthop, entry) in all_entries {
                            println!(
                                "entry: (next_hop, router_from): {:?} route.as_path: {:?} route.as_path.len: {:?} route.attrs.ebgp: {:?}",
                                nexthop,
                                *entry.route().as_path(),
                                entry.route().as_path().len(),
                                *entry.attrs().ebgp()
                            );
                        }
                        println!("");
                    }
                    panic!("stack overflow");
                }
                if router_from == v {
                    v = next_hop;
                } else {
                    v = router_from;
                }
            }

            let mut delay = latency[v as usize].unwrap();
            while !stack.is_empty() {
                let p = stack.pop().unwrap();
                delay += topo
                    .get_weight(get_router!(topo, p), get_router!(topo, v))
                    .unwrap();
                // *get_latency_mut_unsafe!(self, p, prefix) = Some(delay);
                latency[p as usize] = Some(delay);
                v = p;
            }
            // println!("router_u={}, delay={}", u, delay);
        }
        println!(
            "Finished calculating latencies in {}s.",
            start.elapsed().as_secs()
        );
        latency
    }

    fn check_latency(
        &mut self,
        topo: &Topology,
        stats: &SimStats,
        filename: impl AsRef<Path>,
    ) -> std::io::Result<()> {
        std::fs::create_dir_all(filename.as_ref().parent().unwrap())?;

        let latency = self.calc_latency(topo, stats);
        let mut latencies = Vec::new();
        for &u in &stats.covered_routers {
            if !get_router!(topo, u).reflector {
                let router_id = get_router!(topo, u).id;
                let latency = latency[u as usize].unwrap();
                latencies.push((router_id, latency));
            }
        }
        latencies.sort();

        let mut file = File::create(filename)?;
        for (router_id, latency) in latencies {
            file.write_fmt(format_args!("{} {}\n", router_id, latency))?;
        }
        Ok(())
    }

    fn check_rib(&self, filename: impl AsRef<Path>) -> std::io::Result<()> {
        std::fs::create_dir_all(filename.as_ref().parent().unwrap())?;
        let mut file = File::create(filename)?;
        for (router, rib) in self.ribs() {
            if rib.adj_rib_in.len() == 0 {
                continue;
            }
            if let Some((next_hop, router_from)) = rib.get_loc_rib_ru(&self.opt().prefix) {
                let entry = rib.get_loc_rib_entry(&self.opt().prefix).unwrap();
                file.write_fmt(format_args!(
                    "{}: {}, {}, {}\n",
                    router,
                    next_hop,
                    router_from,
                    entry.route().as_path().len()
                ))?;
            }
            // if let Some(map) = rib.get_adj_rib_all(&self.opt().prefix) {
            //     let mut v: Vec<(&(u32, u32), &<Self as Simulation>::PartialEntry)> =
            //         map.iter().collect();
            //     v.sort_by_key(|(&next_hop, _)| next_hop);
            //     file.write_fmt(format_args!("{}: {}, ", router, map.len()))?;
            //     for ((next_hop, router_from), entry) in v {
            //         file.write_fmt(format_args!(
            //             "({}, {}, {}), ",
            //             next_hop,
            //             router_from,
            //             entry.attrs().as_path().len()
            //         ))?;
            //     }
            //     file.write("\n".as_bytes())?;
            // }
        }
        Ok(())
    }
}
