use clap::Parser;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use sim::sim::SimOpt;
use sim::Simulator;
use topo::utils::Reader;
use topo::Topology;

fn parse_mode(mode: &str) -> String {
    match mode.chars().nth(0).unwrap() {
        'v' => "vanilla".to_string(),
        'a' => "aigp".to_string(),
        'g' => "geo".to_string(),
        'p' => "path".to_string(),
        _ => {
            panic!("Invalid mode option!");
        }
    }
}

#[derive(Parser, Debug)]
#[structopt(name = "simulator", about = "BGP simulator on the Internet")]
struct Opt {
    /// vanilla, aigp, geo
    #[structopt(short, long, default_value = "vanilla")]
    mode: String,

    /// tag appended to output file
    #[structopt(short, long, default_value = "")]
    tag: String,

    #[structopt(short, long, default_value = "0")]
    asn: u32,

    /// announce from these locations
    #[structopt(short, long)]
    locs: Option<PathBuf>,

    /// use user input
    #[structopt(short, long)]
    user: bool,

    #[structopt(short, long, default_value = "./input/caida")]
    input: PathBuf,

    #[structopt(short, long)]
    extra_input_opt: Option<Vec<PathBuf>>,

    #[structopt(short, long, default_value = "./output/cloud")]
    output: PathBuf,

    #[structopt(short, long, default_value = "1")]
    start: usize,

    #[structopt(short, long, default_value = "1")]
    num: usize,

    /// number of rounds in BGP convergence
    #[structopt(short, long, default_value = "0")]
    rounds: usize,

    /// disable bgp additional path
    #[structopt(long)]
    no_bgp_add: bool,

    /// disable next_hop geolocation
    #[structopt(long)]
    no_geo_next_hop: bool,

    /// scope of ASes adopting the new protocol
    #[structopt(long, default_value = "all")]
    no_pref_scope_path: String,

    #[structopt(long, default_value = "all")]
    metric_scope_path: String,

    /// quantizer (ms) for path mode
    #[structopt(short, long, default_value = "10")]
    quant: u32,

    // config file, includes multiple set of SimConf
    #[structopt(short, long)]
    config_path: Option<String>,
}

fn default_scope() -> String {
    "all".to_string()
}
fn default_quant() -> u32 {
    10
}
#[derive(Debug, Deserialize)]
struct SimConf {
    #[serde(rename = "no_pref", default = "default_scope")]
    no_pref_scope_path: String,
    #[serde(rename = "metric", default = "default_scope")]
    metric_scope_path: String,
    mode: String,
    #[serde(default = "default_quant")]
    quant: u32,
}

#[derive(Debug, Deserialize)]
struct Conf {
    configs: Vec<SimConf>,
}

fn main() -> std::io::Result<()> {
    env_logger::init();
    let mut opt = Opt::parse();
    opt.output = opt
        .output
        .join(opt.input.file_name().unwrap().to_str().unwrap());
    println!("{:?}", opt);

    // run multiple if config is provided in file
    let conf_list = if let Some(config_path) = &opt.config_path {
        let conf_str = fs::read_to_string(config_path).expect("Failed to read config file");
        toml::from_str(&conf_str).unwrap()
    } else {
        Conf {
            configs: vec![SimConf {
                no_pref_scope_path: opt.no_pref_scope_path,
                metric_scope_path: opt.metric_scope_path,
                mode: opt.mode,
                quant: opt.quant,
            }],
        }
    };

    for conf in &conf_list.configs {
        println!("{:?}", conf);
    }
    let start = Instant::now();

    let seed = 42;
    let mut rng = StdRng::seed_from_u64(seed);
    let mut topo = if opt.user {
        Topology::read_user_from_path(&opt.input)?
    } else {
        Topology::read_from_path(
            opt.user,
            &opt.input,
            &opt.extra_input_opt.unwrap_or_default(),
        )?
    };
    println!("Read finished at {}s", start.elapsed().as_secs());

    let origin_locs =
        if let Some(reader) = opt.locs.as_ref().and_then(|path| Reader::new(path).ok()) {
            let mut locs = Vec::new();
            for line in reader {
                let items: Vec<&str> = line.split(',').collect();
                let lat: f32 = items[2].trim().parse().unwrap();
                let lng: f32 = items[3].trim().parse().unwrap();
                locs.push((lat, lng));
            }
            Some(locs)
        } else {
            None
        };
    println!("Origin locs: {:?}", origin_locs);

    if !opt.user {
        topo.condense_border_graph(Some(opt.output.clone()));
    }
    topo.print();
    // topo.print_ixp("./output/ixp.txt")?;
    // topo.print_stub("./output/stub.txt")?;
    let border_as_list: Vec<u32> = topo.border_as_list.clone().into_iter().collect();
    let mut stats_list = Vec::new();

    for conf in conf_list.configs {
        let i = 1; // just to align with post-processing scripts
        println!("\nExpt {}", i);

        let mut origin = opt.asn;
        while origin == topo::IXP_ASN
            || origin == topo::HUB_ASN
            || topo
                .as_borders
                .get(origin)
                .map_or(true, |borders| borders.is_empty())
        {
            origin = *border_as_list.choose(&mut rng).unwrap();
        }
        println!("Origin: {}", origin);
        let prefix = "1.1.1.1".parse().unwrap();
        let sim_opt = SimOpt {
            mode: conf.mode.clone(),
            prefix,
            origin,
            origin_locs: origin_locs.clone(),
            rounds: opt.rounds,
            no_bgp_add: opt.no_bgp_add,
            no_geo_next_hop: opt.no_geo_next_hop,
            no_pref_scope: conf.no_pref_scope_path.clone(),
            metric_scope: conf.metric_scope_path.clone(),
            quant: conf.quant,
        };

        let tag = if opt.tag.len() > 0 {
            format!("{}_{}_{}", conf.mode, opt.tag, i)
        } else {
            let no_pref_scope_last = if conf.no_pref_scope_path == "all" {
                "all"
            } else {
                conf.no_pref_scope_path.split('/').last().unwrap()
            };
            let metric_scope_last = if conf.metric_scope_path == "all" {
                "all"
            } else {
                conf.metric_scope_path.split('/').last().unwrap()
            };
            format!(
                "{}_{}_{}_{}_{}",
                conf.mode, no_pref_scope_last, metric_scope_last, conf.quant, i
            )
        };
        println!("Use {} simulator", tag);

        let mut sim = Simulator::new_for_topology(sim_opt, &topo);
        let stats = sim.run(&topo);
        sim.check_latency(
            &topo,
            &stats,
            opt.output.join(format!("latency_{}.txt", tag)),
        )?;
        sim.check_rib(opt.output.join(format!("rib_{}.txt", tag)))?;
        stats_list.push(stats)
    }

    println!("Finished at {}s", start.elapsed().as_secs());
    Ok(())
}
