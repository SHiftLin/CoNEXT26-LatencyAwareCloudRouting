use clap::Parser;
use geographiclib_rs::InverseGeodesic;
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::net::Ipv4Addr;
use traceroutes::Hop;
#[derive(Parser, Debug)]
struct Args {
    #[arg(
    short,
    long,
    // default_value = "../ripe/log_google/TR_google-asia-south1_34.93.241.6.txt"   
    )]
    ripe_log_path: Option<String>,
    // Path to the trace file
    #[arg(short, long, default_value = "input/ripe/msm_results/117777501.json")]
    trace_path: Vec<String>,

    /// Path to the ripe probe file
    #[arg(short, long, default_value = "input/ripe/20250713.json")]
    ripe_probe_path: String,

    /// IP table to use
    #[arg(short, long, default_value = "ip_geo_202506")]
    ip_table: String,

    #[arg(short, long, default_value = "34.93.241.6")]
    dst_ip: Ipv4Addr,

    #[arg(short, long)]
    dst_lat: f32,

    #[arg(short, long)]
    dst_lng: f32,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    println!("{:?}", args);

    let trace_path = if let Some(ripe_log_path) = args.ripe_log_path {
        let reader = std::fs::File::open(ripe_log_path).expect("Failed to open ripe log file");
        let lines = std::io::BufReader::new(reader).lines();
        let mut trace_path = Vec::new();
        for line in lines {
            if let Ok(line) = line {
                let parts: Vec<&str> = line.split(|c| c == '[' || c == ']').collect();
                trace_path.push(format!(
                    "input/ripe/msm_results/{}.json",
                    parts[parts.len() - 2].to_string()
                ));
            }
        }
        trace_path
    } else {
        args.trace_path
    };

    println!("{:?}", trace_path);

    let all_traces =
        traceroutes::get_traces(trace_path, &args.ripe_probe_path, &args.ip_table).unwrap();
    println!("read in {}", all_traces.len());

    let dst_hop = Hop {
        ip: args.dst_ip,
        asn: Some(396982),
        rtt: 1000.0,
        ttl: 1023,
        latlng: Some((args.dst_lat, args.dst_lng)),
    };

    let (processed_traces, topo_diff) =
        compare::process_traces(all_traces.clone(), None, Some(&dst_hop));
    let retained_probes = processed_traces
        .iter()
        .filter_map(|trace| trace.prb_id)
        .collect::<Vec<u32>>();
    assert!(topo_diff.nodes_as.len() == topo_diff.nodes_geo.len());
    println!(
        "retained {} traces; discoverd {} border links, {} nodes",
        processed_traces.len(),
        topo_diff.links.len(),
        topo_diff.nodes_as.len()
    );
    println!("{:?}", all_traces[0]);
    let prb_dist1 = processed_traces
        .iter()
        .map(|trace| {
            // println!("{}/{}", trace.route_dist_km(), dist);
            (trace.prb_id.unwrap(), trace.route_dist_km()) // Convert to km
        })
        .collect::<HashMap<u32, f64>>();
    let mut inflation = prb_dist1
        .iter()
        .map(|(_prb_id, route_dist)| {
            let g = geographiclib_rs::Geodesic::wgs84();
            let src_hop = all_traces
                .iter()
                .find(|trace| trace.prb_id.unwrap() == *_prb_id)
                .and_then(|trace| trace.hops.first())
                .expect("Trace has no hops");

            let dist: f64 = g.inverse(
                src_hop.latlng.unwrap().0 as f64,
                src_hop.latlng.unwrap().1 as f64,
                dst_hop.latlng.unwrap().0 as f64,
                dst_hop.latlng.unwrap().1 as f64,
            );
            *route_dist / (dist / 1000.0) // Convert to km
        })
        .collect::<Vec<f64>>();

    inflation.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = inflation[inflation.len() / 2];
    let p90 = inflation[inflation.len() * 9 / 10];
    let p95 = inflation[inflation.len() * 19 / 20];
    println!("median inflation: {:.2} ", median);
    println!("90% inflation: {:.2}", p90);
    println!("95% inflation: {:.2}", p95);

    let prb_dist2 = all_traces
        .iter()
        .filter_map(|trace| {
            if !retained_probes.contains(&trace.prb_id.unwrap()) {
                None // Skip traces with prb_id not in retained_probes
            } else {
                let mut hops = trace
                    .hops
                    .iter()
                    .filter(|hop| hop.latlng.is_some())
                    .cloned()
                    .collect::<Vec<Hop>>();
                hops.push(dst_hop.clone());
                let trace = traceroutes::Trace {
                    prb_id: Some(trace.prb_id.unwrap()),
                    hops,
                    src: trace.src,
                    dst: trace.dst,
                };
                // for hop in hops {
                //     println!("Hop: {:?}", hop);
                // }
                // println!("route_dist_km: {}", trace.route_dist_km());
                Some((trace.prb_id.unwrap(), trace.route_dist_km()))
            }
        })
        .collect::<HashMap<u32, f64>>();

    let mut inflation = prb_dist2
        .iter()
        .map(|(_prb_id, route_dist)| {
            let g = geographiclib_rs::Geodesic::wgs84();
            let src_hop = all_traces
                .iter()
                .find(|trace| trace.prb_id.unwrap() == *_prb_id)
                .and_then(|trace| trace.hops.first())
                .expect("Trace has no hops");
            let dist: f64 = g.inverse(
                src_hop.latlng.unwrap().0 as f64,
                src_hop.latlng.unwrap().1 as f64,
                dst_hop.latlng.unwrap().0 as f64,
                dst_hop.latlng.unwrap().1 as f64,
            );
            // println!("Trace: {:?}, dist: {}", trace, dist);
            *route_dist / (dist / 1000.0) // Convert to km
        })
        .collect::<Vec<f64>>();

    inflation.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = inflation[inflation.len() / 2];
    let p90 = inflation[inflation.len() * 9 / 10];
    let p95 = inflation[inflation.len() * 19 / 20];
    println!("hop by hop");
    println!("median inflation: {:.2} ", median);
    println!("90% inflation: {:.2}", p90);
    println!("95% inflation: {:.2}", p95);

    let mut inflation = all_traces
        .iter()
        .filter_map(|trace| {
            let src_hop = trace.hops.first()?;
            let g = geographiclib_rs::Geodesic::wgs84();
            let dist: f64 = g.inverse(
                src_hop.latlng?.0 as f64,
                src_hop.latlng?.1 as f64,
                dst_hop.latlng?.0 as f64,
                dst_hop.latlng?.1 as f64,
            );
            let dist1 = prb_dist1.get(&trace.prb_id.unwrap())?;
            if *dist1 > 1000.0 * 1000.0 && dist - dist1 < 500.00 * 1000.0 {
                println!("Trace: {:?}, dist: {}", trace, dist);
            }
            let trace_dst_hop = trace.hops.last()?;
            if trace_dst_hop.ip == args.dst_ip {
                Some(trace_dst_hop.rtt / (dist / 1000.0 / 100.0)) // Convert to km
            } else {
                None
            }
            // Convert to km
        })
        .collect::<Vec<f64>>();
    println!("{} traces with dst_ip {}", inflation.len(), args.dst_ip);
    inflation.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = inflation[inflation.len() / 2];
    let p90 = inflation[inflation.len() * 9 / 10];
    let p95 = inflation[inflation.len() * 19 / 20];
    println!("rtt inflation");
    println!("median inflation: {:.2} ", median);
    println!("90% inflation: {:.2}", p90);
    println!("95% inflation: {:.2}", p95);

    let inflation = all_traces
        .iter()
        .filter_map(|trace| {
            let trace_dst_hop = trace.hops.last()?;
            if !prb_dist1.contains_key(&trace.prb_id.unwrap()) || trace_dst_hop.ttl == 1023 {
                None // Skip traces with prb_id not in retained_probes
            } else {
                // Some((prb_dist1[&trace.prb_id.unwrap()], trace_dst_hop.rtt))
                if prb_dist2[&trace.prb_id.unwrap()] < prb_dist1[&trace.prb_id.unwrap()] + 500.0 {
                    println!(
                        "{}: {} < {} + 500",
                        trace.prb_id.unwrap(),
                        prb_dist2[&trace.prb_id.unwrap()],
                        prb_dist1[&trace.prb_id.unwrap()]
                    );
                    println!("Trace: {:?}", trace);
                }
                Some((
                    prb_dist1[&trace.prb_id.unwrap()],
                    prb_dist2[&trace.prb_id.unwrap()],
                    // trace_dst_hop.rtt,
                ))
            }
        })
        .collect::<Vec<(f64, f64)>>();
    let file = "temp-dist-dist.txt";
    let mut file = std::fs::File::create(file).expect(
        "Failed to create
    file",
    );
    for (dist, rtt) in &inflation {
        writeln!(file, "{} {}", dist, rtt).expect("Failed to write to file");
    }
    Ok(())
}
