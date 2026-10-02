use clap::Parser;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::{fs::File, path::Path};
use topo::{get_router, Topology};
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

    #[arg(short, long, default_value = "input/caida-0824")]
    topology_path: String,

    #[arg(short, long)]
    extra_topology_path: Option<Vec<PathBuf>>,

    // The method to compare with ripe trace; If none, go over all methods under the given sim_result_path
    #[arg(short, long)]
    sim_method: Option<Vec<String>>,

    #[arg(
        short,
        long,
        default_value = "output/cloud/asia-south1/extra/ripe_AS_EU-c_AS_EU_US/wan/caida-0824"
    )]
    sim_result_path: String,

    // path of the output file
    #[arg(short, long)]
    output_path: Option<String>,

    #[arg(short, long, default_value = "false")]
    extend: bool,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    println!("{:?}", args);

    let start = std::time::Instant::now();

    let sim_method = "path_all_all_2_1";
    let latency_path = format!("{}/latency_{}.txt", args.sim_result_path, sim_method);
    let rib_path = format!("{}/rib_{}.txt", args.sim_result_path, sim_method);
    let result = sim_result::read_result(&latency_path, &rib_path).unwrap();

    let extra_input = if let Some(extra_input) = args.extra_topology_path {
        extra_input
    } else {
        vec![]
    };
    let topo = Topology::read_from_path(false, &PathBuf::from(&args.topology_path), &extra_input)?;
    println!("Read Topology finished at {}s", start.elapsed().as_secs());

    let path = "input/as_scope/top500.txt-ext-top-ext-top-ext-top";
    let file = File::open(&path);
    let reader = BufReader::new(file.unwrap());
    let scope = if let Some(Ok(scope)) = reader.lines().next() {
        let parts = scope.trim().split(',').collect::<Vec<_>>();
        parts
            .iter()
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    println!("total={}", scope.len());
    let non_stub_scope = scope
        .iter()
        .filter(|&&asn| !topo.single_homed_stub_as.contains(&asn))
        .cloned()
        .collect::<HashSet<_>>();
    println!("non_stub_scope={}", non_stub_scope.len());

    let mut covered_as = result
        .latency
        .iter()
        .filter_map(|(router_id, _)| {
            let asn = get_router!(topo, *router_id).asn.unwrap();
            if !topo.stub_as.contains(&asn) {
                Some(asn)
            } else {
                None
            }
        })
        .collect::<HashSet<_>>();
    println!("Total covered non-stub ASNs: {}", covered_as.len());
    println!(
        "Percentage {}/{}={:.2}%",
        scope.len(),
        covered_as.len(),
        scope.len() as f64 / covered_as.len() as f64 * 100.0
    );
    Ok(())
}
