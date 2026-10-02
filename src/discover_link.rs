// use topo::Topology;

use std::{io::BufRead, net::Ipv4Addr, path::PathBuf};

use clap::Parser;
use compare;
use topo::{utils::Reader, Topology};
use traceroutes::Hop;

#[derive(Parser, Debug)]
struct Args {
    // if give ripe_log_path, read trace path from it
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

    #[arg(short, long)]
    topology_path: Option<String>,

    #[arg(short, long, default_value = "34.93.241.6")]
    dst_ip: Ipv4Addr,

    #[arg(short, long, default_value = "")]
    locs: PathBuf,

    #[arg(short, long)]
    dst_asn: u32,
}

fn main() {
    let args = Args::parse();
    // let trace_path = "../traceroutes/data/result.json";
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

    let all_traces =
        traceroutes::get_traces(trace_path, &args.ripe_probe_path, &args.ip_table).unwrap();
    println!("read in {}", all_traces.len());

    let reader = Reader::new(args.locs).unwrap();
    let mut locs = Vec::new();
    for line in reader {
        let items: Vec<&str> = line.split(',').collect();
        let lat: f32 = items[2].trim().parse().unwrap();
        let lng: f32 = items[3].trim().parse().unwrap();
        locs.push((lat, lng));
    }
    let dst_hop = Hop {
        ip: args.dst_ip,
        asn: Some(args.dst_asn),
        rtt: 1000.0,
        ttl: 1023,
        latlng: Some(locs[0]),
    };

    let mut topo = Topology::new(false);
    let as_rel_path = PathBuf::from("input/caida-0824/20240801.as-rel2.txt");
    topo.read_as_rel(as_rel_path).unwrap();

    let (processed_traces, topo_diff) =
        compare::process_traces(all_traces, Some(topo.as_rel.clone()), Some(&dst_hop));
    assert!(topo_diff.nodes_as.len() == topo_diff.nodes_geo.len());
    println!(
        "retained {} traces; discoverd {} border links, {} nodes",
        processed_traces.len(),
        topo_diff.links.len(),
        topo_diff.nodes_as.len()
    );
    for (u, v) in &topo_diff.nodes_as {
        if *v == 12312 {
            println!("{} {}", u, v);
        }
    }
    if let Some(path) = &args.topology_path {
        topo_diff.dump(path.into()).unwrap();
    } else {
        println!("No topology path provided, skipping dump.");
    }
}
