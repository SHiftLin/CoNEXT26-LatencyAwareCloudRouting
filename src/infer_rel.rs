use std::{
    collections::{hash_map, BTreeSet, HashMap, HashSet, VecDeque},
    time::Instant,
};

use rand::{rngs::StdRng, SeedableRng};
use sim::Simulator;
use topo::{get_router, AsRel, Router, RouterId, Topology};

mod utils;

const TPO_DIR: &str = "./input/caida";

fn bgp_neighbors(topo: &Topology, ru: &Router) -> Vec<RouterId> {
    let u = ru.id;
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
    vs
}

fn main() -> std::io::Result<()> {
    let start = Instant::now();

    let mut topo = Topology::new(false);
    topo.read_geo(&(TPO_DIR.to_owned() + "/midar-iff.nodes.geo.in"))?;
    topo.read_links(&(TPO_DIR.to_owned() + "/midar-iff.links.in"))?;
    topo.read_as(&(TPO_DIR.to_owned() + "/midar-iff.nodes.as.in"))?;
    topo.read_as_rel(&(TPO_DIR.to_owned() + "/20221201.as-rel2.txt"))?;
    topo.ixp_detection();
    topo.build_border_graph();

    let origin_as = 15169;

    let mut t1_asns = HashSet::from([
        7018, 3320, 3257, 6830, 3356, 2914, 5511, 3491, 6453, 6762, 1299, 12956, 701, 6461,
    ]);
    let mut q = VecDeque::new();
    let mut visited: HashSet<u32> = HashSet::new();

    let mut inferred_as_rel = topo.as_rel.clone();
    for u in topo.as_borders.get(origin_as).unwrap() {
        visited.insert(*u);
        let ru = get_router!(topo, *u);
        let asu = ru.asn.unwrap();
        let neighbors = bgp_neighbors(&topo, ru);
        for v in neighbors {
            let rv = get_router!(topo, v);
            let asv = rv.asn.unwrap();
            if visited.insert(v) {
                q.push_back(v);
            }
            if asu == asv {
                continue;
            }
            let as_rel = inferred_as_rel.get(&(asu, asv)).unwrap_or(&AsRel::None);
            if as_rel == &AsRel::None {
                if t1_asns.contains(&asv) {
                    // tier1->provider
                    inferred_as_rel.insert((asu, asv), AsRel::CustomerProvider);
                    inferred_as_rel.insert((asv, asu), AsRel::ProviderCustomer);
                } else {
                    // others->peer
                    inferred_as_rel.insert((asu, asv), AsRel::PeerPeer);
                    inferred_as_rel.insert((asv, asu), AsRel::PeerPeer);
                }
            }
        }
    }
    while !q.is_empty() {
        let u = q.pop_front().unwrap();
        let ru = get_router!(topo, u);
        let asu = ru.asn.unwrap();
        let neighbors = bgp_neighbors(&topo, ru);
        for v in neighbors {
            let rv = get_router!(topo, v);
            let asv = rv.asn.unwrap();

            if asu == asv {
                continue;
            }
            if visited.contains(&v) {
                continue;
            }
            let as_rel = topo.as_rel.get(&(asu, asv)).unwrap_or(&AsRel::None);
            if as_rel != &AsRel::None {
                continue;
            }
            if as_rel != &AsRel::ProviderCustomer {
                continue;
            }
            q.push_back(v);
        }
    }
    println!("Total borders: {}", topo.borders.len());
    println!("Total visited: {}", visited.len());
    Ok(())
}
