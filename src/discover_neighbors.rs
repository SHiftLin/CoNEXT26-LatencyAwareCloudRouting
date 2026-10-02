use clap::Parser;
use std::collections::HashSet;
use std::path::PathBuf;
use std::{fs::File, io::Write, path::Path};

use topo::{get_router, Topology};
#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long, default_value = "input/caida-0824")]
    topology_path: String,

    #[arg(short, long)]
    extra_topology_path: Option<Vec<PathBuf>>,

    #[arg(short, long, default_value = "15169")]
    cloud_asn: u32,

    #[arg(short, long)]
    output_path: Option<PathBuf>,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let start = std::time::Instant::now();
    let input = Path::new(&args.topology_path);
    let extra_input = if let Some(extra_input) = args.extra_topology_path {
        extra_input
        // vec![]
    } else {
        vec![]
    };
    let topo = Topology::read_from_path(false, &PathBuf::from(input), &extra_input)?;

    println!("Read Topology finished at {}s", start.elapsed().as_secs());
    println!("ASRel(2497, 9304) = {:?}", topo.as_rel.get(&(2497, 9304)));
    let cloud_asns = vec![45903];
    let mut cloud_neighbors: HashSet<u32> = HashSet::new();
    for asn in &cloud_asns {
        let cloud_borders = topo.as_borders.get_unsafe(*asn);
        for u in cloud_borders {
            let ru = get_router!(topo, *u);
            let vs = topo.bgp_neighbors(ru);
            for v in &vs {
                let rv = get_router!(topo, *v);
                if rv.asn == Some(9304) {
                    println!("{}:{:?} - {}:{:?}", ru.id, ru.latlng, rv.id, rv.latlng);
                }
                cloud_neighbors.insert(rv.asn.unwrap());
            }
        }
    }
    println!("AS{:?} has {} neighbors", cloud_asns, cloud_neighbors.len());
    println!("{:?}", cloud_neighbors);

    // if let Some(output_path) = args.output_path {
    //     let mut output_file = File::create(output_path.join("neighbors.txt"))?;
    //     // write asn in one line, separated by space
    //     let asn_line = cloud_neighbors
    //         .iter()
    //         .map(|asn| asn.to_string())
    //         .collect::<Vec<String>>()
    //         .join(",");
    //     writeln!(output_file, "{}", asn_line)?;
    // }
    Ok(())
}
