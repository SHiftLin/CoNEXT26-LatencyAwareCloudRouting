// use topo::Topology;

use std::{
    collections::HashSet,
    io::{BufRead, Write},
};

use clap::Parser;
use traceroutes::{Hop, RawRIPEHop};

#[derive(Parser, Debug)]
struct Args {
    // if give ripe_log_path, read trace path from it
    #[arg(
        short,
        long,
        // default_value = "../ripe/log_google/TR_google-asia-south1_34.93.241.6.txt"
    )]
    ripe_log_path: Option<Vec<String>>,
    // Path to the trace file
}

fn main() {
    let args = Args::parse();
    // let trace_path = "../traceroutes/data/result.json";
    let ripe_log_path = args.ripe_log_path.unwrap();
    let mut trace_path = Vec::new();

    for p in ripe_log_path {
        let reader = std::fs::File::open(p).expect("Failed to open ripe log file");
        let lines = std::io::BufReader::new(reader).lines();
        for line in lines {
            if let Ok(line) = line {
                let parts: Vec<&str> = line.split(|c| c == '[' || c == ']').collect();
                trace_path.push(format!(
                    "input/ripe/msm_results/{}.json",
                    parts[parts.len() - 2].to_string()
                ));
            }
        }
    }
    let mut file = std::fs::File::create("temp/paths.txt").expect("Failed to create file");
    for path in &trace_path {
        writeln!(file, "../BGPSimulator/{}", path).expect("Failed to write to file");
    }

    let mut ip = Vec::new();
    for path in trace_path {
        let ips: Vec<_> = traceroutes::extract_ip::<RawRIPEHop>(&path)
            .iter()
            .map(|ip| ip.to_string())
            .collect();
        ip.extend(ips);
    }
    let distinct_ip: HashSet<_> = ip.into_iter().collect();
    let mut file = std::fs::File::create("temp/ip.txt").expect("Failed to create file");
    println!("Distinct IPs: {}", distinct_ip.len());
    for ip in distinct_ip {
        writeln!(file, "{}", ip).expect("Failed to write to file");
    }
}
