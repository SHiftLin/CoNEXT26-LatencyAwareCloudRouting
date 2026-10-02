use hashbrown::HashMap;
use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use topo::{AsNumber, AsRel, RouterId};

use crate::def_field;

pub trait RIBAttrs {
    def_field!(ebgp, bool); // Learn from eBGP
                            // def_field!(as_path, Vec<AsNumber>);
                            // def_field!(as_rel, Option<AsRel>);
                            // def_field!(latency, Option<u32>);
    def_field!(metric, u32); // AIGP metric
}

pub trait BGPRoute {
    def_field!(prefix, Ipv4Addr);
    def_field!(as_path, Vec<AsNumber>);
    def_field!(next_hop, RouterId); // Should be ip address in reality
    def_field!(router_from, RouterId); // The router received eBGP in this AS
}

// pub trait PartialRIBEntry {
//     type Route: BGPRoute + Clone; // Clone is for create_ibgp/ebgp_update
//     type Attrs: RIBAttrs + Clone;

//     fn new(
//         prefix: Ipv4Addr,
//         as_path: Vec<AsNumber>,
//         next_hop: RouterId,
//         ebgp: bool,
//         as_rel: Option<AsRel>,
//     ) -> Self;

//     fn new_with_route(route: Self::Route) -> Self;
//     fn new_with_attrs(attrs: Self::Attrs) -> Self;
//     // def_field!(route, Self::Route);
//     def_field!(attrs, Self::Attrs);
// }

pub trait FullRIBEntry {
    type Route: BGPRoute + Clone;
    type Attrs: RIBAttrs + Clone;
    fn new(
        prefix: Ipv4Addr,
        as_path: Vec<AsNumber>,
        next_hop: RouterId,
        ebgp: bool,
        as_rel: Option<AsRel>,
    ) -> Self;
    fn new_with_route(route: Self::Route) -> Self;
    def_field!(route, Self::Route);
    def_field!(attrs, Self::Attrs);
}

#[derive(Default)]
pub struct BGPUpdate<T: BGPRoute> {
    pub table: Vec<T>,
}

impl<T: BGPRoute> BGPUpdate<T> {
    pub fn new() -> Self {
        BGPUpdate { table: Vec::new() }
    }
}

#[derive(Clone)]
pub struct RIBUpdateEntry<T: FullRIBEntry> {
    pub add: bool,
    pub entry: T,
}

pub struct RIBUpdate<T: FullRIBEntry> {
    pub table: Vec<RIBUpdateEntry<T>>,
    // extra or T(entry)
}

impl<T: FullRIBEntry> RIBUpdate<T> {
    pub fn new() -> Self {
        RIBUpdate { table: Vec::new() }
    }
}

#[derive(Clone)] // Clone is for ribs in FastMap
pub struct RIBTable<T: FullRIBEntry + Clone> {
    // Clone is for ibgp/ebgp_import
    pub loc_rib: BTreeMap<Ipv4Addr, (RouterId, RouterId)>,
    // Local-RIB, index to Adj-RIB-In
    // BTreeMap 10^8 * 2 Entries -> ~61.1G
    // HashMap 10^8 * 2 Entries -> ~44.7G
    pub adj_rib_in: Vec<(Ipv4Addr, HashMap<(RouterId, RouterId), T>)>,
    // Adj-RIB-In
    // per prefix, per (next_hop, ru) pair
    // ebgp -> equivalent to (ru, )
    // ibgp -> equivalent to (next_hop, ) real life, ru distinguishs different interface on next_hop router
    // Use Vec since the length is usually small
}

impl<T: FullRIBEntry + Clone> Default for RIBTable<T> {
    // Default is for ribs in FastMap
    fn default() -> Self {
        RIBTable {
            loc_rib: BTreeMap::new(),
            adj_rib_in: Vec::new(),
        }
    }
}

impl<T: FullRIBEntry + Clone> RIBTable<T> {
    pub fn new() -> Self {
        RIBTable {
            loc_rib: BTreeMap::new(),
            adj_rib_in: Vec::new(),
        }
    }
    pub fn get_adj_rib_by_nlri(
        &self,
        prefix: &Ipv4Addr,
        nlri: &(RouterId, RouterId),
    ) -> Option<&T> {
        for (entry, map) in &self.adj_rib_in {
            if entry == prefix {
                return map.get(nlri);
            }
        }
        return None;
    }

    pub fn get_adj_rib_by_nlri_mut(
        &mut self,
        prefix: &Ipv4Addr,
        nlri: &(RouterId, RouterId),
    ) -> Option<&mut T> {
        for (entry, map) in &mut self.adj_rib_in {
            if entry == prefix {
                return map.get_mut(nlri);
            }
        }
        return None;
    }

    pub fn get_adj_rib_entry_any(&self, prefix: &Ipv4Addr) -> Option<&T> {
        for (entry, map) in &self.adj_rib_in {
            if entry == prefix {
                return map.values().next();
            }
        }
        return None;
    }

    pub fn get_adj_rib_entry_any_mut(&mut self, prefix: &Ipv4Addr) -> Option<&mut T> {
        for (entry, map) in &mut self.adj_rib_in {
            if entry == prefix {
                return map.values_mut().next();
            }
        }
        return None;
    }

    pub fn get_loc_rib_ru(&self, prefix: &Ipv4Addr) -> Option<&(RouterId, RouterId)> {
        if let Some(nlri) = self.loc_rib.get(prefix) {
            Some(nlri)
        } else {
            None
        }
    }
    pub fn get_loc_rib_nlri(&self, prefix: &Ipv4Addr) -> Option<&(RouterId, RouterId)> {
        if let Some(nlri) = self.loc_rib.get(prefix) {
            Some(nlri)
        } else {
            None
        }
    }

    pub fn get_loc_rib_entry(&self, prefix: &Ipv4Addr) -> Option<&T> {
        if let Some(nlri) = self.loc_rib.get(prefix) {
            self.get_adj_rib_by_nlri(prefix, nlri)
        } else {
            None
        }
    }

    pub fn get_loc_rib_entry_mut(&mut self, prefix: &Ipv4Addr) -> Option<&mut T> {
        if let Some(nlri) = self.loc_rib.get(prefix).cloned() {
            self.get_adj_rib_by_nlri_mut(prefix, &nlri)
        } else {
            None
        }
    }

    // pub fn get_loc_rib_nexthop_entry(&self, prefix: &Ipv4Addr, next_hop: &RouterId) -> Option<&T> {
    //     if let Some(router_id) = self.loc_rib.get(prefix) {
    //         self.get_adj_rib_by_ru(prefix, router_id.get(next_hop)?)
    //     } else {
    //         None
    //     }
    // }

    // pub fn get_loc_rib_nexthop_entry_mut(
    //     &mut self,
    //     prefix: &Ipv4Addr,
    //     next_hop: &RouterId,
    // ) -> Option<&mut T> {
    //     if let Some(router_id) = self.loc_rib.get(prefix).cloned() {
    //         self.get_adj_rib_by_ru_mut(prefix, router_id.get(next_hop)?)
    //     } else {
    //         None
    //     }
    // }

    pub fn get_adj_rib_all(&self, prefix: &Ipv4Addr) -> Option<&HashMap<(RouterId, RouterId), T>> {
        for (entry, map) in &self.adj_rib_in {
            if entry == prefix {
                return Some(map);
            }
        }
        return None;
    }
    pub fn get_adj_rib_all_mut(
        &mut self,
        prefix: &Ipv4Addr,
    ) -> Option<&mut HashMap<(RouterId, RouterId), T>> {
        for (entry, map) in &mut self.adj_rib_in {
            if entry == prefix {
                return Some(map);
            }
        }
        return None;
    }

    // pub fn insert_loc_rib(
    //     &mut self,
    //     prefix: Ipv4Addr,
    //     next_hop: RouterId,
    //     ru_id: RouterId,
    //     replace: bool,
    // ) {
    //     self.loc_rib
    //         .entry(prefix)
    //         .and_modify(|map| {
    //             if replace {
    //                 // always have exact one entry
    //                 map.clear();
    //             }
    //             map.insert(next_hop, ru_id);
    //         })
    //         .or_insert_with(|| {
    //             // not loc rib to this prefix
    //             let mut map = HashMap::new();
    //             map.insert(next_hop, ru_id);
    //             map
    //         });
    // }
    pub fn insert_adj_rib_in(
        &mut self,
        entry_prefix: &Ipv4Addr,
        nlri: &(RouterId, RouterId),
        entry: T,
    ) {
        for (prefix, map) in &mut self.adj_rib_in {
            if entry_prefix == prefix {
                map.insert(*nlri, entry);
                return;
            }
        }
        self.adj_rib_in
            .push((*entry_prefix, HashMap::from([(*nlri, entry)])));
    }
}

pub mod rib_table {
    use hashbrown::{hash_map, HashMap};
    use std::{
        // collections::{btree_map, hash_map, BTreeMap, HashMap},
        net::Ipv4Addr,
    };
    use topo::RouterId;

    pub struct Iter<'a, T: 'a> {
        pub(super) index: usize,
        pub(super) map_iter: Option<hash_map::Iter<'a, (RouterId, RouterId), T>>,
        pub(super) table: &'a Vec<(Ipv4Addr, HashMap<(RouterId, RouterId), T>)>,
    }

    impl<'a, V> Iterator for Iter<'a, V> {
        type Item = &'a V;

        fn next(&mut self) -> Option<Self::Item> {
            while self.index < self.table.len() {
                if self.map_iter.is_none() {
                    self.map_iter = Some(self.table[self.index].1.iter());
                }
                if let Some((_, item)) = self.map_iter.as_mut().unwrap().next() {
                    return Some(item);
                } else {
                    self.index += 1;
                    self.map_iter = None;
                }
            }
            None
        }
    }

    pub struct IterMut<'a, T: 'a> {
        pub(super) index: usize,
        pub(super) map_iter: Option<hash_map::IterMut<'a, (RouterId, RouterId), T>>,
        pub(super) table: &'a mut Vec<(Ipv4Addr, HashMap<(RouterId, RouterId), T>)>,
    }

    impl<'a, T> Iterator for IterMut<'a, T> {
        type Item = &'a mut T;

        fn next(&mut self) -> Option<Self::Item> {
            while self.index < self.table.len() {
                if self.map_iter.is_none() {
                    self.map_iter = unsafe {
                        let map = (&mut self.table[self.index].1)
                            as *mut HashMap<(RouterId, RouterId), T>;
                        Some((*map).iter_mut())
                        //  Some(self.table[self.index].1.iter());
                    }
                }
                if let Some((_, item)) = self.map_iter.as_mut().unwrap().next() {
                    return Some(item);
                } else {
                    self.index += 1;
                    self.map_iter = None;
                }
            }
            None
        }
    }
}

impl<'a, T: FullRIBEntry + Clone> IntoIterator for &'a RIBTable<T> {
    type Item = &'a T;
    type IntoIter = rib_table::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        rib_table::Iter {
            index: 0,
            map_iter: None,
            table: &self.adj_rib_in,
        }
    }
}

impl<'a, T: FullRIBEntry + Clone> IntoIterator for &'a mut RIBTable<T> {
    type Item = &'a mut T;
    type IntoIter = rib_table::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        rib_table::IterMut {
            index: 0,
            map_iter: None,
            table: &mut self.adj_rib_in,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::FullRIBEntry;
    use crate::sim_vanilla::FullEntry;
    // use fastmap::FastMap;
    // use hashbrown::HashMap;
    // use rand::Rng;
    #[test]
    fn ribtable_size() {
        let mx_router_id = 10000;
        let mut ribs: Vec<Vec<((u32, u32), FullEntry)>> = Vec::new();
        for _ in 0..mx_router_id {
            ribs.push(Vec::new());
        }
        // let mut n_entries = 0;
        // let mut rng = rand::thread_rng();
        let prefix: Ipv4Addr = "1.1.1.1".parse().unwrap();

        for ru in 1..mx_router_id + 1 {
            if ru % 1000 == 0 {
                println!("{} routers", ru);
                let mut total_capacity = 0;
                for rib in ribs.iter() {
                    // if let Some(map) = rib(&prefix) {
                    // map.shrink_to_fit();
                    total_capacity += rib.capacity();
                    // }
                }
                println!("Total capacity: {}", total_capacity);
            }
            let rib = ribs.get_mut(ru);
            if let Some(rib) = rib {
                let mx_next_hop = 14336;
                // let mx_next_hop = 10;
                for next_hop in 0..mx_next_hop {
                    let entry = FullEntry::new(prefix, vec![1, 2, 3], next_hop, true, None);
                    assert_eq!(std::mem::size_of_val(&entry), 48);
                    // assert!(entry.attrs().as_path().capacity() == 3);
                    rib.push(((next_hop, next_hop), entry));
                }
                rib.shrink_to_fit();
            }
        }
    }
    // #[test]
    // fn ribtable_iter() {
    //     let mut table = RIBTable::new();
    //     let mut entries: Vec<PartialEntry> = vec![
    //         PartialEntry::new("1.1.1.1".parse().unwrap(), vec![], 1, true, None),
    //         PartialEntry::new("1.1.1.1".parse().unwrap(), vec![10], 2, true, None),
    //         PartialEntry::new("2.2.2.2".parse().unwrap(), vec![20, 40], 1, false, None),
    //     ];
    //     table.insert_adj_rib_in("1.1.1.1".parse().unwrap(), &(1, 1), entries[0].clone());
    //     table.insert_adj_rib_in("1.1.1.1".parse().unwrap(), &(2, 2), entries[1].clone());
    //     table.insert_adj_rib_in("2.2.2.2".parse().unwrap(), &(1, 1), entries[2].clone());

    //     let mut i = 0;
    //     for item in table.adj_rib_in.iter_mut().flat_map(|v| v.1.values_mut()) {
    //         entries[i].attrs_mut().as_path_mut().push(10);
    //         item.attrs_mut().as_path_mut().push(10);
    //         i += 1;
    //     }
    //     i = 0;
    //     for item in &mut table {
    //         entries[i].attrs_mut().as_path_mut().push(30);
    //         item.attrs_mut().as_path_mut().push(30);
    //         i += 1;
    //     }
    //     i = 0;
    //     let mut update = BGPUpdate::new();
    //     for item in &table {
    //         update.table.push(item.attrs().clone());
    //         i += 1;
    //     }
    //     update.table.sort_by_key(|x| x.as_path().len());
    //     i = 0;
    //     for item in &update.table {
    //         assert_eq!(entries[i].attrs(), item);
    //         i += 1;
    //     }
    // }
}
