// use topo::Topology;

use std::{
    collections::{HashMap, HashSet},
    io::{self, BufRead, Write},
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    process::Command,
};

use clap::Parser;
use compare::{self, TopoDiff};
use topo::{AsRel, Converter, Topology};
use traceroutes::{Hop, IpInfo, RawScamperHop, RawTrace, RawTraceHop, Trace};

use rayon::prelude::*;

#[derive(Parser, Debug)]
struct Args {
    // if give ripe_log_path, read trace path from it
    // Path to the trace file
    #[arg(
        short,
        long,
        default_value = "/nfs/lsh/traceroutes/all/output_062025_asia-south1_10000.json"
    )]
    trace_path: Vec<String>,
    #[arg(short, long, default_value = "google")]
    cloud: String,
    // #[arg(short, long, default_value = "34.93.241.6")]
    // dst_ip: Ipv4Addr,
    #[arg(short, long, default_value = "1")]
    stage: u32,

    #[arg(short, long)]
    topology_path: Option<String>,
    // #[arg(short, long)]
    // dst_lat: f32,

    // #[arg(short, long)]
    // dst_lng: f32,
}

fn main() {
    let start = std::time::Instant::now();
    let args = Args::parse();

    let traces = args
        .trace_path
        .iter()
        .flat_map(|trace| {
            traceroutes::read_json_lines::<RawTrace<RawScamperHop>>(trace)
                .expect("Failed to read traces")
        })
        .collect::<Vec<_>>();
    println!("Read traces from {:?}", args.trace_path);

    let mut client = database::PostgresClient::new().expect("Failed to create Postgres client");
    let cloud_asns = client
        .query("cloud_asns", "asn", &format!("cloud='{}'", args.cloud))
        .unwrap();
    println!("{:?}", &format!("lower(org) like '%{}%'", args.cloud));
    let cloud_asns = cloud_asns
        .into_iter()
        .map(|row| row.get::<usize, i64>(0) as u32)
        .collect::<std::collections::HashSet<_>>();
    println!("cloud_asns:{:?}", cloud_asns);

    let mut topology = Topology::new(false);
    let input = PathBuf::from("input/caida-0824");
    topology
        .read_as_rel(input.join("20240801.as-rel2.txt"))
        .unwrap();

    let work_dir = "/nfs/lsh/bdrmapit";
    let node_as = format!("{}/node.as", work_dir);
    let reader = std::fs::File::open(node_as).expect("Failed to open node.as file");
    let lines = std::io::BufReader::new(reader).lines();
    let ip_to_as = lines
        .par_bridge()
        .filter_map(|line| {
            if let Ok(line) = line {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() == 4 {
                    let ip = parts[1].trim().to_string();
                    let asn = parts[2].trim().parse::<u32>().ok();
                    if let Some(asn) = asn {
                        return Some((ip, asn));
                    }
                }
            }
            None
        })
        .collect::<HashMap<_, _>>();
    println!(
        "Read AS finished; {:?}; time = {:?}",
        ip_to_as.get("210.166.32.65"),
        start.elapsed()
    );

    if args.stage == 1 {
        // let mut file = std::fs::File::create("temp/paths.txt").expect("Failed to create file");
        // for path in &args.trace_path {
        //     writeln!(file, "{}", path).expect("Failed to write to file");
        // }

        // let status = Command::new(".venv/bin/bdrmapit")
        //     .arg("all")
        //     .arg("-i")
        //     .arg("rib/ip2as.prefixes")
        //     .arg("-b")
        //     .arg("rib/20250701.as-org2info.jsonl")
        //     .arg("-r")
        //     .arg("rib/20250701.as-rel.txt")
        //     .arg("-c")
        //     .arg("rib/20250701.ppdc-ases.txt")
        //     .arg("-P")
        //     .arg("rib/peeringdb_2_dump_2025_07_16.json")
        //     .arg("-k")
        //     .arg("node.as")
        //     .arg("-j")
        //     .arg("../BGPSimulator/temp/paths.txt")
        //     .current_dir(work_dir)
        //     .status()
        //     .expect("Failed to execute bdrmapit");

        let edges = traces
            .par_iter()
            .flat_map(|trace| {
                let mut links = Vec::new();
                if trace.hops.len() < 2 {
                    return links;
                }
                for i in 0..trace.hops.len() - 1 {
                    let asn_u = trace.hops[i]
                        .ip()
                        .and_then(|ip| ip_to_as.get(&ip.to_string()).copied());
                    let asn_v = trace.hops[i + 1]
                        .ip()
                        .and_then(|ip| ip_to_as.get(&ip.to_string()).copied());
                    if let (Some(asn_u), Some(asn_v)) = (asn_u, asn_v) {
                        if trace.hops[i].ttl() + 1 == trace.hops[i + 1].ttl() {
                            if asn_u != asn_v {
                                if cloud_asns.contains(&asn_u) && !cloud_asns.contains(&asn_v) {
                                    links.push((trace.hops[i].clone(), trace.hops[i + 1].clone()));
                                }
                            }
                        }
                    }
                }
                // for hop in trace.hops {
                //     println("{:?}", hop );
                // }
                return links;
            })
            .collect::<Vec<_>>();

        println!("links: {:?}", edges.len());
        let edge_ips = edges
            .par_iter()
            .flat_map(|(u, v)| {
                vec![
                    u.ip().map(|ip| ip.to_string()),
                    v.ip().map(|ip| ip.to_string()),
                ]
            })
            .collect::<HashSet<_>>();
        println!("edge_ips: {:?}", edge_ips.len());

        let asn_links = edges
            .par_iter()
            .filter_map(|(u, v)| {
                let asn_u = u.ip().and_then(|ip| ip_to_as.get(&ip.to_string()).copied());
                let asn_v = v.ip().and_then(|ip| ip_to_as.get(&ip.to_string()).copied());
                // u cloud, v neighbor
                if let (Some(asn_u), Some(asn_v)) = (asn_u, asn_v) {
                    return Some((asn_u, asn_v));
                }
                None
            })
            .collect::<HashSet<_>>();

        let neighbor_rels = asn_links
            .par_iter()
            .filter_map(|(asu, asv)| {
                let as_rel = topology.as_rel.get(&(*asu, *asv)).unwrap_or(&AsRel::None);
                return Some(as_rel);
            })
            .collect::<Vec<&AsRel>>();

        let mut neighbor_rel_counter: HashMap<&AsRel, u32> = HashMap::new();

        for rel in neighbor_rels {
            *neighbor_rel_counter.entry(rel).or_insert(0) += 1;
        }

        for (rel, count) in neighbor_rel_counter {
            println!("Neighbor rel {:?}: {}", rel, count);
        }

        let neighbor_asn = edges
            .par_iter()
            .filter_map(|(u, v)| {
                let asn_v = v.ip().and_then(|ip| ip_to_as.get(&ip.to_string()).copied());
                // u cloud, v neighbor
                if let Some(asn_v) = asn_v {
                    return Some(asn_v);
                }
                None
            })
            .collect::<HashSet<_>>();
        println!("neighbor_asn: {:?}", neighbor_asn.len());

        // write ips to temp/ip.txt
        let mut file = std::fs::File::create("temp/ip.txt").expect("Failed to create file");
        for ip in edge_ips {
            writeln!(file, "{}", ip.unwrap()).expect("Failed to write to file");
        }
    } else {
        let mut client = database::PostgresClient::new().unwrap();
        let ip_info = client
            .query(
                "ip_geo_202509",
                &format!("ip, {}, lat, lng", "asn_bdrmapit"),
                "true",
            )
            .expect("Failed to query IP info");

        let ip_info: HashMap<Ipv4Addr, IpInfo> = ip_info
            .iter()
            .map(|row| {
                // field "ip" is not nullable in the database
                let ip: IpAddr = row.get(0);
                let ip = match ip {
                    IpAddr::V4(ipv4) => ipv4,
                    _ => panic!("Expected IPv4 address"),
                };
                // field asn, lat, lng may be null
                // we convert int4->u32 (asn are always non-negtive), f64->f32 (we don't care about precision)
                let asn: Option<i32> = row.get("asn_bdrmapit");
                let asn = asn.map(|a| a as u32);
                let lat: Option<f64> = row.get("lat");
                let lng: Option<f64> = row.get("lng");
                let latlng = match (lat, lng) {
                    (Some(lat), Some(lng)) => Some((lat as f32, lng as f32)),
                    _ => None,
                };
                (ip, IpInfo { ip, asn, latlng })
            })
            .collect();

        // let converted_traces = traces
        //     .iter()
        //     .map(|trace| {
        //         let hops = trace
        //             .hops
        //             .iter()
        //             .map(|hop| {
        //                 let ip = hop.ip().unwrap();
        //                 let info = ip_info.get(&ip);

        //                 let latlng = if let Some(info) = info {
        //                     info.latlng
        //                 } else {
        //                     None
        //                 };
        //                 let asn = ip_to_as.get(&ip.to_string()).copied();
        //                 Hop {
        //                     ip,
        //                     asn,
        //                     rtt: hop.rtt(),
        //                     ttl: hop.ttl(),
        //                     latlng,
        //                 }
        //             })
        //             .collect::<Vec<_>>();
        //         Trace {
        //             prb_id: None,
        //             src: trace.src.unwrap_or(Ipv4Addr::UNSPECIFIED),
        //             dst: trace.dst.unwrap_or(Ipv4Addr::UNSPECIFIED),
        //             hops,
        //         }
        //     })
        //     .collect::<Vec<_>>();

        // for trace in converted_traces.iter().take(10) {
        //     println!("Trace: {:?}", trace);
        // }
        // let (_, diff) =
        //     compare::process_traces(converted_traces, Some(topology.as_rel.clone()), None);

        let edges = traces
            .par_iter()
            .flat_map(|trace| {
                let mut links = Vec::new();
                if trace.hops.len() < 2 {
                    return links;
                }
                for i in 0..trace.hops.len() - 1 {
                    let info_u = trace.hops[i].ip().and_then(|ip| ip_info.get(&ip).copied());
                    let info_v = trace.hops[i + 1]
                        .ip()
                        .and_then(|ip| ip_info.get(&ip).copied());
                    if let (Some(info_u), Some(info_v)) = (info_u, info_v) {
                        if trace.hops[i].ttl() + 1 == trace.hops[i + 1].ttl() {
                            if let (Some(asn_u), Some(asn_v)) = (info_u.asn, info_v.asn) {
                                // if cloud_asns.contains(&asn_u) && !cloud_asns.contains(&asn_v) {
                                if asn_u != asn_v {
                                    let hop1 = Hop {
                                        ip: trace.hops[i].ip().unwrap(),
                                        asn: Some(asn_u),
                                        rtt: trace.hops[i].rtt(),
                                        ttl: trace.hops[i].ttl(),
                                        latlng: info_u.latlng,
                                    };
                                    let hop2 = Hop {
                                        ip: trace.hops[i + 1].ip().unwrap(),
                                        asn: Some(asn_v),
                                        rtt: trace.hops[i + 1].rtt(),
                                        ttl: trace.hops[i + 1].ttl(),
                                        latlng: info_v.latlng,
                                    };
                                    links.push((hop1, hop2));
                                }
                            }
                        }
                    }
                }
                return links;
            })
            .collect::<Vec<_>>();
        let asn_links = edges
            .par_iter()
            .filter_map(|(u, v)| {
                let asn_u = ip_to_as.get(&u.ip.to_string()).copied();
                let asn_v = ip_to_as.get(&v.ip.to_string()).copied();
                // u cloud, v neighbor
                if let (Some(asn_u), Some(asn_v)) = (asn_u, asn_v) {
                    return Some((asn_u, asn_v));
                }
                None
            })
            .collect::<HashSet<_>>();

        let as_rel = asn_links
            .iter()
            .filter_map(|(asu, asv)| {
                if topology.as_rel.contains_key(&(*asu, *asv)) {
                    None
                } else {
                    Some(((*asu, *asv), AsRel::PeerPeer))
                }
            })
            .collect::<HashMap<_, _>>();

        println!("links: {:?}", edges.len());
        let diff = TopoDiff::from(edges, Some(as_rel));
        if let Some(topology_path) = args.topology_path {
            diff.dump(topology_path.into()).unwrap();
        }
    }
}
