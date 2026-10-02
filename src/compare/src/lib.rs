use geographiclib_rs::InverseGeodesic;
use rayon::prelude::*;
use sim_result::{SimHop, SimResult, SimTrace};
use std::cmp::min;
use std::collections::HashSet;
use std::hash::Hash;
use std::io::Write;
use std::{collections::HashMap, fs::File, net::Ipv4Addr, path::PathBuf};
use topo::AsNumber;
use topo::utils::{decode_latlng, pair_ord};
use topo::{AsRel, Topology, get_router, utils::LatLng};
use traceroutes::{Hop, Trace, read_json_lines};

// return the router in topo closest to hop (no matter whether it is within distance_threshold(in meters) or not)
pub fn find_router<'a>(
    hop: &Hop,
    result: &SimResult,
    topo: &'a Topology,
    distance_threshold: Option<f64>,
) -> Option<(&'a topo::Router, u32)>
where
{
    let asn = hop.asn?;
    let borders = topo.as_borders.get(asn)?;
    let mut covered_borders = Vec::new();
    for u in borders {
        if result.rib.contains_key(u) {
            covered_borders.push(*u);
        }
    }
    let closest_router = covered_borders
        .into_iter()
        .filter_map(|u| {
            let ru = get_router!(topo, u);
            if let Some((lat_hop, lng_hop)) = hop.latlng {
                if let Some((lat_ru, lng_ru)) = ru.latlng {
                    let (lat_ru, lng_ru) = decode_latlng((lat_ru, lng_ru));
                    let g = geographiclib_rs::Geodesic::wgs84();
                    let dist: f64 =
                        g.inverse(lat_hop as f64, lng_hop as f64, lat_ru as f64, lng_ru as f64);
                    if distance_threshold.map_or(true, |threshold: f64| dist < threshold) {
                        return Some((ru, (dist / 1000.0) as u32)); // return distance in km (rounded for comparison)
                    } else {
                        return None;
                    }
                }
            }
            return None;
        })
        .min_by(|a, b| {
            if distance_threshold.is_some() {
                a.0.id.cmp(&b.0.id)
            } else {
                a.1.cmp(&b.1)
            }
        });

    if let Some(ru) = closest_router {
        Some((ru.0, ru.1))
    } else {
        None
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct RouterMap {
    pub id: u32,
    pub group_id: u32,
}

pub struct TopoDiff {
    pub nodes_as: HashMap<Ipv4Addr, u32>,
    pub nodes_geo: HashMap<Ipv4Addr, LatLng>,
    pub links: Vec<(Ipv4Addr, Ipv4Addr)>,
    pub as_rel: HashMap<(u32, u32), AsRel>,
}

impl TopoDiff {
    pub fn from(borders: Vec<(Hop, Hop)>, as_rel: Option<HashMap<(u32, u32), AsRel>>) -> TopoDiff {
        // panic safety: all nodes are required to have both asn and latlng
        let nodes_as = borders
            .iter()
            .flat_map(|(hop1, hop2)| {
                vec![(hop1.ip, hop1.asn.unwrap()), (hop2.ip, hop2.asn.unwrap())]
            })
            .collect::<HashMap<_, _>>();
        let nodes_geo = borders
            .iter()
            .flat_map(|(hop1, hop2)| {
                vec![
                    (hop1.ip, hop1.latlng.unwrap()),
                    (hop2.ip, hop2.latlng.unwrap()),
                ]
            })
            .collect::<HashMap<_, _>>();
        let links = borders
            .iter()
            .map(|(hop1, hop2)| (hop1.ip, hop2.ip))
            .collect::<Vec<_>>();
        let as_rel = if let Some(as_rel) = as_rel {
            as_rel
        } else {
            HashMap::new()
        };
        TopoDiff {
            nodes_as,
            nodes_geo,
            links,
            as_rel,
        }
    }

    pub fn dump(&self, target_dir: PathBuf) -> std::io::Result<()> {
        std::fs::create_dir_all(&target_dir)?;
        let mut nodes_ip = self.nodes_as.keys().collect::<Vec<_>>();
        nodes_ip.sort(); // deterministic order 
        let node_id = nodes_ip
            .iter()
            .enumerate()
            .map(|(i, ip)| (*ip, i as u32 + 1))
            .collect::<HashMap<_, _>>();
        let node_file = File::create(target_dir.join("nodes.in"))?;

        // nodes.in: node <-> IP this is not used in sim
        let mut writer = std::io::BufWriter::new(node_file);
        for (ip, id) in &node_id {
            writeln!(writer, "{} {}", id, ip)?;
        }
        // nodes.as.in: node <-> asn
        let mut writer = std::io::BufWriter::new(File::create(target_dir.join("nodes.as.in"))?);
        for (ip, asn) in &self.nodes_as {
            if let Some(id) = node_id.get(ip) {
                writeln!(writer, "{} {}", id, asn)?;
            }
        }
        // nodes.geo.in: node <-> latlng
        let mut writer = std::io::BufWriter::new(File::create(target_dir.join("nodes.geo.in"))?);
        for (ip, latlng) in &self.nodes_geo {
            if let Some(id) = node_id.get(ip) {
                writeln!(writer, "{} {} {}", id, latlng.0, latlng.1)?;
            }
        }
        // links.in: node <-> node
        let mut writer = std::io::BufWriter::new(File::create(target_dir.join("links.in"))?);
        let links = self
            .links
            .iter()
            .map(|(ip1, ip2)| {
                if let (Some(id1), Some(id2)) = (node_id.get(ip1), node_id.get(ip2)) {
                    (id1, id2)
                } else {
                    panic!("IP not found in node_id: {} or {}", ip1, ip2);
                }
            })
            .collect::<HashSet<_>>();
        for (id1, id2) in links {
            writeln!(writer, "{} {}", id1, id2)?;
        }
        // as-rel.txt: asn|asn|relation|sth
        let mut writer = std::io::BufWriter::new(File::create(target_dir.join("as-rel.txt"))?);
        for ((asu, asv), rel) in &self.as_rel {
            if rel == &AsRel::ProviderCustomer {
                writeln!(writer, "{}|{}|-1|", *asu, *asv)?;
            } else if rel == &AsRel::PeerPeer {
                writeln!(writer, "{}|{}|0|", *asu, *asv)?;
            }
        }
        Ok(())
    }
}

pub fn read_mapping(path: &str) -> HashMap<u32, u32> {
    println!("Reading router mapping from {}", path);
    let result: Vec<RouterMap> = read_json_lines(path).expect("Failed to read router mapping file");

    result
        .into_iter()
        .map(|rm| (rm.id, rm.group_id))
        .collect::<HashMap<_, _>>()
}

pub fn map_trace(
    trace: Trace,
    result: &SimResult,
    topo: &Topology,
    router_mapping: &HashMap<u32, u32>,
) -> Result<SimTrace, String> {
    let trace = Trace::reduce_intermediate(&trace);
    let src_hop: Hop = trace
        .hops
        .first()
        .cloned()
        .ok_or_else(|| "Trace has no hops".to_string())?;
    if src_hop.ttl != 0 {
        return Err("First hop TTL must be 0".to_string());
    }
    // find source router
    let (src_router, _distance) = match find_router(&src_hop, result, topo, None) {
        Some((a, b)) => (Some(a), Some(b)),
        None => (None, None),
    };
    // find destination router
    // println!("src_router: {:?}", src_router.map(|r| r.id));
    if src_router.is_none() {
        return Err("No matching router.".to_string());
    }
    let g = geographiclib_rs::Geodesic::wgs84();
    let src_router = src_router.unwrap();
    let mut hops = trace
        .hops
        .iter()
        .filter_map(|hop| {
            let router = if hop.ttl == 0 || hop.ttl == 1023 {
                let (router, _distance) = match find_router(&hop, result, topo, None) {
                    Some((a, b)) => (Some(a), Some(b)),
                    None => (None, None),
                };
                router
            } else {
                // use the mapping to find the exact router to avoid some corner cases
                let router_id = topo.iface_router.get(&hop.ip)?;
                let group_id = router_mapping.get(router_id)?;
                Some(get_router!(topo, *group_id))
            };

            if let Some(router) = router {
                Some(SimHop {
                    router_id: router.id,
                    asn: router.asn,
                    rtt: 0.0,
                    latlng: Some(decode_latlng(router.latlng.unwrap())),
                })
            } else {
                // temporary, if hop not in topo, use the hop itself as a SimHop
                Some(SimHop {
                    router_id: 0,
                    asn: hop.asn,
                    rtt: 0.0,
                    latlng: hop.latlng,
                })
            }
        })
        .collect::<Vec<_>>();
    let mut acc_dist = 0.0;
    for i in 0..hops.len() - 1 {
        let ru = get_router!(topo, hops[i].router_id);
        let rv = get_router!(topo, hops[i + 1].router_id);
        let (lat_ru, lng_ru) = decode_latlng(ru.latlng.unwrap());
        let (lat_rv, lng_rv) = decode_latlng(rv.latlng.unwrap());
        let dist: f64 = g.inverse(lat_rv as f64, lng_rv as f64, lat_ru as f64, lng_ru as f64);
        acc_dist += dist / 1000.0 / 100.0; // km -> m -> ms
        hops[i + 1].rtt = acc_dist;
    }

    if hops.len() != trace.hops.len() {
        // println!("{}, {}", hops.len(), trace.hops.len());
        // println!("{:?}", trace);
        // println!(
        //     "{:?}",
        //     SimTrace {
        //         src: src_router.id,
        //         hops,
        //     }
        // );
        return Err("Some hops could not be mapped to routers.".to_string());
    }
    Ok(SimTrace {
        src: src_router.id,
        hops,
    })
}

pub struct CompResult {
    pub direct_dist: f64,
    pub trace_dist: f64,
    pub sim_dist: f64,
    pub sim_route: SimTrace,
    pub trace_in_sim: SimTrace,
    pub sim_as_path: Vec<u32>,
    pub trace_as_path: Vec<u32>,
    pub as_triple: Option<(u32, u32, u32)>,
    pub org_as_path_len: u32,
}

pub fn compare(
    trace: &Trace,
    result: &SimResult,
    topo: &Topology,
    router_mapping: &HashMap<u32, u32>,
) -> Result<CompResult, String> {
    // should've return a u32 as dRTT, todo
    // I have no idea yet.
    let src_hop: Hop = trace
        .hops
        .first()
        .cloned()
        .ok_or_else(|| "Trace has no hops".to_string())?;

    let dst_hop = trace
        .hops
        .last()
        .cloned()
        .ok_or_else(|| "Trace has no hops".to_string())?;
    if src_hop.ttl != 0 {
        return Err("First hop TTL must be 0".to_string());
    }

    let (src_router, distance) = match find_router(&src_hop, result, topo, None) {
        Some((a, b)) => (Some(a), Some(b)),
        _ => (None, None),
    };
    // println!("src_router: {:?}", src_router.map(|r| r.id));
    if src_router.is_none() {
        // println!("{:?}", trace);
        return Err("No matching hop. Given AS is not in the topology".to_string());
    }

    let src_router = src_router.unwrap();
    let org_as_path_len = result.rib.get(&src_router.id).map_or(0, |p| p.as_path_len) as u32;
    let distance = distance.unwrap();
    // distance and src_router will either both be Some or both be None
    let direct_dist = src_hop.dist_to(&dst_hop).unwrap(); // src-hop -> dst_hop;
    let sim_route = sim_result::route(src_router.id, result, Some(topo));
    let first_seg_dist = distance as f64; // convert probe->src router to km
    let sim_dist = first_seg_dist + sim_route.route_dist_km(); // prb -> closest border router + br to dst

    let trace_in_sim = map_trace(trace.clone(), result, topo, router_mapping)?;
    let trace_dist = first_seg_dist + trace_in_sim.route_dist_km(); // prb -> closest border router + br to dst

    let mut trace_as_path = Vec::new();
    for hop in &trace_in_sim.hops {
        if let Some(asn) = hop.asn {
            if Some(&asn) != trace_as_path.last() {
                trace_as_path.push(asn);
            }
        }
    }
    let mut sim_as_path = Vec::new();
    for hop in &sim_route.hops {
        if let Some(asn) = hop.asn {
            if Some(&asn) != sim_as_path.last() {
                sim_as_path.push(asn);
            }
        }
    }

    if !valley_free(&get_rel_path(trace.clone(), &topo.as_rel)) {
        return Err("default valley-free condition violated".to_string());
    }
    // let debug_prb_id = HashSet::from([Some(61614), Some(34657), Some(20721)]);
    if trace_dist - sim_dist > 2000.0 {
        // if debug_prb_id.contains(&trace.prb_id) {
        println!("RIPE Trace: {:?}", trace);

        println!("RIPE Trace (as simulated):");
        println!("AS Path: {:?}", trace_as_path);
        for i in 1..trace_as_path.len() {
            println!(
                "{}-{:?}-{}",
                trace_as_path[i - 1],
                topo.as_rel.get(&(trace_as_path[i - 1], trace_as_path[i])),
                trace_as_path[i]
            );
        }
        println!("{:?}", trace_in_sim);
        println!("Sim Hops(org AS Path={}):", org_as_path_len);
        println!("AS Path: {:?}", sim_as_path);
        for i in 1..sim_as_path.len() {
            println!(
                "{}-{:?}-{}",
                sim_as_path[i - 1],
                topo.as_rel.get(&(sim_as_path[i - 1], sim_as_path[i])),
                sim_as_path[i]
            );
        }
        println!("{:?}", sim_route);
        println!(
            "Trace distance: {:.2} km, Inflation factor: {:.2}x, Simulated distance: {:.2} km, Trace Dist - Simulated Dist = {:.2} km\n",
            trace_dist,
            trace_dist / direct_dist,
            sim_dist,
            trace_dist - sim_dist
        );
        // println!("Sim Routes (per hop)");
        // for hop in &trace_in_sim.hops {
        //     let from_hop = sim_result::route(hop.router_id, result, Some(topo));
        //     println!("{}: {} km", hop.router_id, from_hop.route_dist_km());
        //     println!("{:?}", from_hop);
        // }
    }

    for i in 0..min(sim_as_path.len(), trace_as_path.len()) {
        let as_1a = sim_as_path[i];
        let as_2a = trace_as_path[i];
        if as_1a != as_2a {
            // let rel_1 = topo
            //     .as_rel
            //     .get(&(sim_as_path[i - 1], as_1a))
            //     .unwrap_or(&AsRel::None);
            // let rel_2 = topo
            //     .as_rel
            //     .get(&(trace_as_path[i - 1], as_2a))
            //     .unwrap_or(&AsRel::None);
            return Ok(CompResult {
                direct_dist,
                trace_dist,
                sim_dist,
                sim_route,
                trace_in_sim,
                sim_as_path: sim_as_path.clone(),
                trace_as_path: trace_as_path.clone(),
                as_triple: Some((sim_as_path[i - 1], as_1a, as_2a)),
                org_as_path_len,
            });
        }
    }
    return Ok(CompResult {
        direct_dist,
        trace_dist,
        sim_dist,
        sim_route,
        trace_in_sim,
        sim_as_path,
        trace_as_path,
        as_triple: None,
        org_as_path_len,
    });
}

// Function to check if all border links exist in the trace
// F(&Hop, &Hop) -> bool is a condition for two hops with different asn to be considered as border links.

pub fn all_border_link_exist<F>(trace: Trace, border_cond: F) -> Option<Trace>
where
    F: Fn(&Hop, &Hop) -> bool,
{
    if trace.hops.first().unwrap().ttl != 0 {
        return None;
        // temperary workaround for probe geo issue, will be fixed when I download the complete RIPE probe list.
    }
    let new_trace = Trace::reduce_intermediate(&trace);
    for i in 0..new_trace.hops.len() - 1 {
        let hop1 = &new_trace.hops[i];
        let hop2 = &new_trace.hops[i + 1];
        if hop1.asn != hop2.asn && !border_cond(hop1, hop2) {
            return None;
        }
    }
    Some(trace)
}

pub fn infer_as_rel_single_path(
    rel_path: Vec<((u32, u32), Option<AsRel>)>,
) -> HashMap<(u32, u32), AsRel> {
    let mut new_rel = HashMap::new();

    let stop1 = rel_path.iter().rposition(|(_, as_rel)| {
        matches!(
            as_rel,
            Some(AsRel::CustomerProvider) | Some(AsRel::PeerPeer)
        )
    });
    let stop2 = rel_path.iter().position(|(_, as_rel)| {
        matches!(
            as_rel,
            Some(AsRel::ProviderCustomer) | Some(AsRel::PeerPeer)
        )
    });
    let stop1 = if let Some(stop1) = stop1 {
        stop1 + 1
    } else {
        0
    };
    let stop2 = stop2.unwrap_or(rel_path.len());

    // println!("{} {}", stop1, stop2);
    if stop1 != 0 {
        for i in 1..stop1 - 1 {
            let ((asu, asv), rel) = &rel_path[i - 1];
            if rel.is_none() {
                new_rel.insert((*asu, *asv), AsRel::CustomerProvider);
                new_rel.insert((*asv, *asu), AsRel::ProviderCustomer);
            }
        }
    }
    for i in stop2 + 1..rel_path.len() {
        let ((asu, asv), rel) = &rel_path[i];
        if rel.is_none() {
            new_rel.insert((*asu, *asv), AsRel::ProviderCustomer);
            new_rel.insert((*asv, *asu), AsRel::CustomerProvider);
        }
    }

    if stop1 + 1 == stop2 {
        let ((asu, asv), rel) = &rel_path[stop1];
        if rel.is_none() {
            new_rel.insert((*asu, *asv), AsRel::PeerPeer);
            new_rel.insert((*asv, *asu), AsRel::PeerPeer);
        }
    }
    return new_rel;
}

pub fn valley_free(rel_path: &Vec<((u32, u32), Option<AsRel>)>) -> bool {
    if rel_path.len() == 0 {
        return true;
    }
    // return false if any of the relationship is None
    if rel_path.iter().any(|(_, rel)| rel.is_none()) {
        return false;
    }
    for i in 0..rel_path.len() - 1 {
        let rel = rel_path[i].1.clone().unwrap_or(AsRel::None);
        let next_rel = rel_path[i + 1].clone().1.unwrap_or(AsRel::None);
        match rel {
            AsRel::CustomerProvider => {
                continue;
            }
            AsRel::ProviderCustomer | AsRel::PeerPeer => {
                if !matches!(next_rel, AsRel::ProviderCustomer) {
                    return false;
                }
            }
            AsRel::None => {
                return false;
            }
        }
    }
    true
}

pub fn get_rel_path(
    trace: Trace,
    as_rel: &HashMap<(u32, u32), AsRel>,
) -> Vec<((u32, u32), Option<AsRel>)> {
    let mut as_path = Vec::new();
    for hop in &trace.hops {
        if let Some(asn) = hop.asn {
            if Some(&asn) != as_path.last() {
                as_path.push(asn);
            }
        }
    }
    if as_path.len() == 1 {
        return vec![]; // no path to infer
    }
    let mut rel_path = Vec::new();
    for i in 0..as_path.len() - 1 {
        let (asu, asv) = (as_path[i], as_path[i + 1]);
        let rel = as_rel.get(&(asu, asv));
        rel_path.push(((asu, asv), rel.cloned()));
    }
    rel_path
}

pub fn infer_as_rel(
    traces: Vec<Trace>,
    base_rel: HashMap<(u32, u32), AsRel>,
) -> HashMap<(u32, u32), AsRel> {
    let mut new_rel = HashMap::new();
    let mut n_non_valley_free = 0;
    for trace in &traces {
        let rel_path = get_rel_path(trace.clone(), &base_rel);
        let valley_free = valley_free(&rel_path);

        if !valley_free {
            n_non_valley_free += 1;
            // println!("default:");
            // for x in &rel_path {
            //     println!("{:?}", x);
            // }
            // println!("inferred:");
            let new_rel_add = infer_as_rel_single_path(rel_path);
            // for x in &new_rel_add {
            //     println!("{:?}", x);
            // }
            new_rel.extend(new_rel_add);
        }
    }

    println!("Non-valley-free traces(before): {}", n_non_valley_free);

    let mut new_base_rel = base_rel.clone();
    let mut n_non_valley_free = 0;

    new_base_rel.extend(new_rel.clone());
    for trace in &traces {
        let rel_path = get_rel_path(trace.clone(), &new_base_rel);
        let valley_free = valley_free(&rel_path);

        if !valley_free {
            n_non_valley_free += 1;
            // println!("non valley-free trace:");
            // for item in &rel_path {
            //     println!("{:?}", item);
            // }
            let mut new_rel_add = HashMap::new();
            for i in 0..rel_path.len() {
                let ((asu, asv), rel) = &rel_path[i];
                if i != rel_path.len() - 1 {
                    new_rel_add.insert((*asu, *asv), AsRel::CustomerProvider);
                    new_rel_add.insert((*asv, *asu), AsRel::ProviderCustomer);
                } else {
                    new_rel_add.insert((*asu, *asv), AsRel::PeerPeer);
                    new_rel_add.insert((*asv, *asu), AsRel::PeerPeer);
                }
            }

            // println!("new_rel_add: {:?}", new_rel_add);
            new_rel.extend(new_rel_add);
        }
    }

    println!("Non-valley-free traces(after): {}", n_non_valley_free);

    new_base_rel.extend(new_rel.clone());
    let mut n_non_valley_free = 0;
    for trace in &traces {
        let rel_path = get_rel_path(trace.clone(), &new_base_rel);
        let valley_free = valley_free(&rel_path);

        if !valley_free {
            n_non_valley_free += 1;
        }
    }
    println!("Non-valley-free traces(after 2): {}", n_non_valley_free);
    println!("Total traces: {}", traces.len());
    new_rel
}

pub fn process_traces(
    traces: Vec<Trace>,
    base_rel: Option<HashMap<(u32, u32), AsRel>>,
    dst_hop: Option<&Hop>,
) -> (Vec<Trace>, TopoDiff) {
    let traces = traces
        .into_iter()
        .filter(|trace| !trace.hops.is_empty())
        .collect::<Vec<_>>();

    let traces = if let Some(dst_hop) = dst_hop {
        traces
            .into_iter()
            .map(|trace| Trace::refine_last_hop(trace, dst_hop))
            .collect::<Vec<_>>()
    } else {
        traces
    };

    println!("#Total (non-empty) Traces: {}", traces.len());

    // filter out traces with different asn overlapping, asn mapping issues or one orgnazation holding different ASNs;
    let traces = traces
        .into_iter()
        .filter_map(|trace| {
            let hops = trace
                .hops
                .iter()
                .map(|h| {
                    let asn = if h.asn == Some(396982) {
                        Some(15169)
                    } else {
                        h.asn
                    };
                    Hop {
                        ip: h.ip,
                        asn: asn,
                        rtt: h.rtt,
                        ttl: h.ttl,
                        latlng: h.latlng,
                    }
                })
                .collect::<Vec<_>>();
            let trace = Trace {
                hops,
                prb_id: trace.prb_id,
                src: trace.src,
                dst: trace.dst,
            };
            let range = Trace::get_asn_ttl_range(trace.clone());
            for (asn_a, range_a) in range.iter() {
                for (asn_b, range_b) in range.iter() {
                    if asn_a != asn_b && !(range_a.0 > range_b.1 || range_b.0 > range_a.1) {
                        return None;
                    }
                }
            }
            Some(trace)
        })
        .collect::<Vec<_>>();

    println!("#Total Trace(weird asn pattern removed): {}", traces.len());
    // let border_link condition to be one of the following F, keep traces with all border link
    let border_cond_ttl = |hop1: &Hop, hop2: &Hop| hop2.ttl == hop1.ttl + 1;
    let _border_cond_ttl_rtt =
        |hop1: &Hop, hop2: &Hop| hop2.ttl == hop1.ttl + 1 || f64::abs(hop2.rtt - hop1.rtt) < 2.0;

    let _border_cond_ttl_rtt_geo = |hop1: &Hop, hop2: &Hop| {
        let g = geographiclib_rs::Geodesic::wgs84();
        let dist: f64 = g.inverse(
            hop1.latlng.unwrap().0 as f64,
            hop1.latlng.unwrap().1 as f64,
            hop2.latlng.unwrap().0 as f64,
            hop2.latlng.unwrap().1 as f64,
        );
        hop2.ttl == hop1.ttl + 1 || (f64::abs(hop2.rtt - hop1.rtt) < 2.0 && dist < 50.0 * 1000.0)
    };

    let traces = traces
        .into_iter()
        .filter_map(|trace| all_border_link_exist(trace, border_cond_ttl))
        .collect::<Vec<_>>();
    let debug_prb_id = HashSet::from([
        Some(51133),
        Some(1007924),
        Some(60840),
        Some(7268),
        Some(7329),
        Some(54690),
    ]);
    let border_links = traces
        .iter()
        .flat_map(|trace| {
            let mut links = Vec::new();
            let trace = Trace::reduce_intermediate(trace);
            for i in 0..trace.hops.len() - 1 {
                let hop1 = &trace.hops[i];
                let hop2 = &trace.hops[i + 1];
                if hop1.asn != hop2.asn && border_cond_ttl(hop1, hop2) {
                    links.push((hop1.clone(), hop2.clone()));
                }
            }
            links
        })
        .collect::<Vec<_>>();

    let as_rel = if let Some(base_rel) = base_rel {
        let new_rel = infer_as_rel(traces.clone(), base_rel);
        println!("Inferred AS Relations: {}", new_rel.len() / 2);
        Some(new_rel)
    } else {
        None
    };
    println!(
        "#Total Trace(incomplete border link traces removed): {}",
        traces.len()
    );
    (traces, TopoDiff::from(border_links, as_rel))
}

// given processed traces and a topology, return a topodiff that includes the extra links.
// every link in the traces will be included
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    pub fn test_infer_as_rel() {
        let rel_path = vec![
            ((213481, 328748), None),
            ((328748, 37662), Some(AsRel::CustomerProvider)),
            ((37662, 19527), Some(AsRel::ProviderCustomer)),
            ((19527, 15169), None),
        ];
        let inferred_rel = infer_as_rel_single_path(rel_path);
        assert!(inferred_rel.len() == 4);
        assert!(inferred_rel.get(&(213481, 328748)) == Some(&AsRel::CustomerProvider));
        assert!(inferred_rel.get(&(19527, 15169)) == Some(&AsRel::ProviderCustomer));
        let rel_path = vec![
            ((401167, 29953), None),
            ((29953, 11287), None),
            ((11287, 6939), Some(AsRel::CustomerProvider)),
            ((6938, 15169), Some(AsRel::PeerPeer)),
        ];
        let inferred_rel = infer_as_rel_single_path(rel_path);
        assert!(inferred_rel.len() == 4);
        assert!(inferred_rel.get(&(401167, 29953)) == Some(&AsRel::CustomerProvider));
        assert!(inferred_rel.get(&(29953, 11287)) == Some(&AsRel::CustomerProvider));
    }

    #[test]
    pub fn test_dist() {
        let g = geographiclib_rs::Geodesic::wgs84();
        let r = (52.21, 5.9694);
        let a = (52.21, 5.96);
        let b = (52.17, 6.74);
        let d_ra: f64 = g.inverse(r.0, r.1, a.0, a.1);
        let d_rb: f64 = g.inverse(r.0, r.1, b.0, b.1);
        let d_ab: f64 = g.inverse(a.0, a.1, b.0, b.1);
        println!("d_ra: {}, d_rb: {}, d_ab: {}", d_ra, d_rb, d_ab);
        assert!(d_ra < 50_000.0);
        assert!(d_rb < 50_000.0);
    }
}
