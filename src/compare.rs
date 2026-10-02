use clap::Parser;
use itertools::Itertools;
use regex::{CaptureLocations, Regex};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::Hash;
use std::io::{BufRead, BufReader, Write};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::{fs::File, path::Path};
use topo::utils::Reader;
use topo::{get_router, Topology};
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

    #[arg(short, long, default_value = "input/caida-0824")]
    topology_path: String,

    #[arg(short, long)]
    extra_input_opt: Option<Vec<PathBuf>>,

    // The method to compare with ripe trace; If none, go over all methods under the given sim_result_path
    #[arg(short, long)]
    sim_method: Option<Vec<String>>,

    #[arg(short, long)]
    sim_method_toml: Option<String>,

    #[arg(
        short,
        long,
        default_value = "output/cloud/asia-south/extra/asia-test-bdrmapit/wan/caida-0824"
    )]
    sim_result_path: String,

    // path of the output file
    #[arg(short, long)]
    output_path: Option<String>,

    #[arg(short, long, default_value = "34.93.241.6")]
    dst_ip: Ipv4Addr,

    #[arg(short, long, default_value = "")]
    locs: PathBuf,

    #[arg(short, long)]
    asn: u32,

    #[arg(short, long, default_value = "false")]
    extend: bool,

    #[arg(short, long)]
    region: String,
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
    let args = Args::parse();
    println!("{:?}", args);
    let methods = if let Some(sim_method_toml) = &args.sim_method_toml {
        let conf_str = fs::read_to_string(sim_method_toml).expect("Failed to read config file");
        let conf: Conf = toml::from_str(&conf_str).unwrap();
        let methods = conf
            .configs
            .iter()
            .map(|c| {
                let no_pref_scope_last = if c.no_pref_scope_path == "all" {
                    "all"
                } else {
                    c.no_pref_scope_path.split('/').last().unwrap()
                };
                let metric_scope_last = if c.metric_scope_path == "all" {
                    "all"
                } else {
                    c.metric_scope_path.split('/').last().unwrap()
                };
                format!(
                    "{}_{}_{}_{}_1",
                    c.mode, no_pref_scope_last, metric_scope_last, c.quant
                )
            })
            .collect::<Vec<_>>();
        Some(methods)
    } else {
        args.sim_method
    };
    let methods = methods.unwrap_or({
        let entries = fs::read_dir(&args.sim_result_path).unwrap();
        let entries = entries
            .into_iter()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                let file_name = path.file_name().unwrap().to_str().unwrap();
                let pattern = Regex::new(r"^latency_(.+)\.txt$").unwrap();
                if let Some(caps) = pattern.captures(file_name) {
                    Some(caps.get(1).unwrap().as_str().to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        entries
    });

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

    println!("{:?}", methods);
    println!("{:?}", trace_path);

    let all_traces =
        traceroutes::get_traces(trace_path, &args.ripe_probe_path, &args.ip_table).unwrap();
    println!("read in {}", all_traces.len());
    let router_mapping =
        compare::read_mapping(&format!("{}/router_mapping.json", args.sim_result_path));

    let reader = Reader::new(args.locs)?;
    let mut locs = Vec::new();
    for line in reader {
        let items: Vec<&str> = line.split(',').collect();
        let lat: f32 = items[2].trim().parse().unwrap();
        let lng: f32 = items[3].trim().parse().unwrap();
        locs.push((lat, lng));
    }

    println!("Origin locs: {:?}", locs);
    let dst_hop = Hop {
        ip: args.dst_ip,
        asn: Some(args.asn),
        rtt: 1000.0,
        ttl: 1023,
        latlng: Some(locs[0]),
    };

    let (processed_traces, topo_diff) = compare::process_traces(all_traces, None, Some(&dst_hop));
    assert!(topo_diff.nodes_as.len() == topo_diff.nodes_geo.len());
    println!(
        "retained {} traces; discoverd {} border links, {} nodes",
        processed_traces.len(),
        topo_diff.links.len(),
        topo_diff.nodes_as.len()
    );

    println!("read+process result finished");
    let start = std::time::Instant::now();
    let extra_input = if let Some(extra_input) = args.extra_input_opt {
        extra_input
    } else {
        vec![]
    };
    let topo = Topology::read_from_path(false, &PathBuf::from(&args.topology_path), &extra_input)?;
    let mut topo_caida = Topology::new(false);
    topo_caida.read_as_rel(format!("{}/20240801.as-rel2.txt", args.topology_path))?;

    println!("Read Topology finished at {}s", start.elapsed().as_secs());

    // let router_set = vec![4981, 26608, 7171, 5007];
    // for (iface, router) in topo.iface_router.iter() {
    //     let group_id = router_mapping.get(router).unwrap_or(&0);
    //     if router_set.contains(group_id) {
    //         println!(
    //             "Router {}: {:?}; AS {}",
    //             group_id,
    //             iface,
    //             get_router!(topo, *router).asn.unwrap()
    //         );
    //     }
    // }

    // let mut neighbor_list = Vec::new();
    // for &target_asn in &topo.border_as_list {
    //     let mut has_link = false;
    //     let borders = topo.as_borders.get_unsafe(target_asn);
    //     for &u in borders {
    //         let ru = get_router!(topo, u);
    //         for &v in &ru.border_links {
    //             let rv = get_router!(topo, v);
    //             if rv.asn == Some(args.asn) {
    //                 has_link = true;
    //                 break;
    //             }
    //         }
    //         if has_link {
    //             break;
    //         }
    //     }
    //     if has_link {
    //         neighbor_list.push(target_asn);
    //     }
    // }

    // let mut neighbor_file = std::fs::File::create(format!("neighbor-{}", args.region)).unwrap();
    // writeln!(
    //     neighbor_file,
    //     "{}",
    //     neighbor_list
    //         .iter()
    //         .map(|asn| asn.to_string())
    //         .collect::<Vec<_>>()
    //         .join(",")
    // )
    // .unwrap();
    // println!(
    //     "Found {} neighbors for AS {}",
    //     neighbor_list.len(),
    //     args.asn
    // );

    for sim_method in methods {
        let latency_path = format!("{}/latency_{}.txt", args.sim_result_path, sim_method);
        let rib_path = format!("{}/rib_{}.txt", args.sim_result_path, sim_method);
        let metric_scope = sim_method.as_str().split('_').collect::<Vec<_>>()[2];
        let metric_scope_path = format!("input/as_scope/{}", metric_scope);
        // read from the metric_scope_path
        let metric_scope = if !Path::new(&metric_scope_path).exists() {
            println!(
                "Metric scope file {} does not exist, skipping",
                metric_scope_path
            );
            vec![]
        } else {
            let file = File::open(&metric_scope_path);
            let reader = BufReader::new(file.unwrap());
            if let Some(Ok(scope)) = reader.lines().next() {
                let parts = scope.trim().split(',').collect::<Vec<_>>();
                parts
                    .iter()
                    .filter_map(|s| s.trim().parse::<u32>().ok())
                    .collect::<Vec<_>>()
            } else {
                vec![]
            }
        };
        println!("{}, {}, {}", latency_path, rib_path, metric_scope.len());
        let result = sim_result::read_result(&latency_path, &rib_path).unwrap();
        let mx_as_len = result
            .rib
            .values()
            .map(|entry| entry.as_path_len)
            .max()
            .unwrap_or(0);
        println!(
            "Read result finished. {} routers, max AS path length {}, length>32 = {}",
            result.rib.len(),
            mx_as_len,
            result.rib.values().filter(|e| e.as_path_len > 32).count()
        );
        let mut all_diff = HashMap::new();
        let mut rel_diff_stats = HashMap::new();
        let mut involved_as: Vec<(u32, u32, u32, u32)> = Vec::new();
        let mut all_as = Vec::new();
        let mut degraded_as: HashMap<u32, u32> = HashMap::new();
        let mut probe_as: HashSet<u32> = HashSet::new();
        let mut error_count = HashMap::new();
        let mut as_path_length = HashMap::new();
        let mut n_long_distance_no_improvement = 0;
        let mut n_long_distance = 0;
        let mut n_as_path_matched = 0;
        for trace in processed_traces.iter() {
            // let new_trace = trace_simplify(trace.clone());
            let res = compare::compare(&trace, &result, &topo, &router_mapping);
            match res {
                Ok(res) => {
                    as_path_length
                        .entry(res.org_as_path_len)
                        .and_modify(|c| *c += 1)
                        .or_insert(1);

                    if res.trace_dist - res.sim_dist > 1000.0 {
                        probe_as.insert(res.sim_as_path[0]);

                        if res.sim_as_path == res.trace_as_path {
                            n_as_path_matched += 1;
                        }
                    }
                    if res.trace_dist / res.direct_dist > 1.05 {
                        for i in 0..res.sim_as_path.len() {
                            all_as.push((res.sim_as_path[i], res.sim_as_path.len() - i));
                        }
                    }
                    all_diff.insert(trace.prb_id.unwrap(), (res.trace_dist, res.sim_dist));
                    if res.trace_dist - res.sim_dist < -100.0 {
                        // println!("{} {}", diff.0, diff.1);
                        if let Some((as0, as_1a, as_1b)) = res.as_triple {
                            let rel_1 =
                                topo.as_rel.get(&(as0, as_1a)).unwrap_or(&topo::AsRel::None);
                            let rel_2 =
                                topo.as_rel.get(&(as0, as_1b)).unwrap_or(&topo::AsRel::None);
                            let rel_key = (rel_1.clone(), rel_2.clone());

                            if rel_key == (topo::AsRel::PeerPeer, topo::AsRel::CustomerProvider) {
                                involved_as.push((trace.prb_id.unwrap(), as0, as_1a, as_1b));
                            }
                            //     || rel_key
                            //         == (topo::AsRel::ProviderCustomer, topo::AsRel::CustomerProvider)
                            //     || rel_key == (topo::AsRel::None, topo::AsRel::CustomerProvider)
                            {
                                // println!("{}: {}->{}, diff={}", trace.prb_id.unwrap(), rel_1, rel_2, diff);
                                *rel_diff_stats.entry(Some((rel_1, rel_2))).or_insert(0) += 1;
                            }
                        } else {
                            *rel_diff_stats.entry(None).or_insert(0) += 1;
                        }
                    }

                    if res.sim_dist > 16000.0 {
                        n_long_distance += 1;
                        if (res.trace_dist - res.sim_dist).abs() < 1000.0 {
                            n_long_distance_no_improvement += 1;
                        }
                    }
                    if res.trace_dist - res.sim_dist < -500.0 {
                        // println!("Sim: {:?}", res.sim_route);
                        // println!("Trace: (Default) {:?}", res.trace_in_sim);
                        let range = res.sim_route.get_asn_rtt_range();
                        let add = if sim_method.starts_with("aigp") {
                            let mut add = HashMap::new();
                            let mut in_scope_found = false;
                            for i in 0..res.sim_as_path.len() {
                                let asn = res.sim_as_path[i];
                                if metric_scope.contains(&asn) {
                                    in_scope_found = true;
                                }
                                if in_scope_found && !metric_scope.contains(&asn) {
                                    add.insert(asn, (range[&asn].0 - range[&asn].1) as i64);
                                }
                            }
                            add
                        } else {
                            range
                                .iter()
                                .filter(|(asn, _)| {
                                    !metric_scope.contains(asn)
                                        && !topo.single_homed_stub_as.contains(asn)
                                        && *asn != &args.asn
                                })
                                .map(|(asn, (l, r))| (*asn, (l - r) as i64))
                                .sorted_by_key(|&(_, r)| -r)
                                .take(1)
                                .collect::<HashMap<_, _>>()
                        };

                        // for (k, v) in range.iter() {
                        //     println!("AS{}: rtt diff {:?}", k, v);
                        // }
                        // println!("addtional ASes {:?}", additional);
                        // for (asn, (l, r)) in &range {
                        //     println!("ASN: {}, ttl: {}-{}", asn, l, r);
                        //     if topo.single_homed_stub_as.contains(asn)
                        //         || metric_scope.contains(asn)
                        //         || asn == &15169
                        //     {
                        //         println!("skip {}", asn);
                        //         continue;
                        // single-homed stub AS does not need to prepend
                        // 15169 will always prepend, though I have some issues with missing it in some of the files.
                        //     }
                        // }
                        // for (asn, _rtt) in &add {
                        //     if asn == &45903 {
                        //         println!("Probe AS {}: {}", asn, trace.prb_id.unwrap());
                        //         println!("Trace Route: {:?}", res.trace_in_sim);
                        //         for i in 1..res.trace_in_sim.hops.len() {
                        //             let asu = res.trace_in_sim.hops[i - 1].asn.unwrap_or(0);
                        //             let asv = res.trace_in_sim.hops[i].asn.unwrap_or(0);
                        //             if asu != asv {
                        //                 let rel = topo
                        //                     .as_rel
                        //                     .get(&(asu, asv))
                        //                     .unwrap_or(&topo::AsRel::None);
                        //                 println!("AS{} --({:?})-> AS{}", asu, rel, asv);
                        //             }
                        //         }
                        //         println!("Sim Route: {:?}", res.sim_route);
                        //         for i in 1..res.sim_as_path.len() {
                        //             let rel = topo
                        //                 .as_rel
                        //                 .get(&(res.sim_as_path[i - 1], res.sim_as_path[i]))
                        //                 .unwrap_or(&topo::AsRel::None);
                        //             println!(
                        //                 "AS{} --({:?})-> AS{}",
                        //                 res.sim_as_path[i - 1],
                        //                 rel,
                        //                 res.sim_as_path[i]
                        //             );
                        //         }
                        //         break;
                        //     }
                        // }
                        for (asn, _rtt) in add {
                            *degraded_as.entry(asn).or_insert(0) += 1;
                        }
                        // println!("");
                    }
                }
                Err(s) => {
                    error_count.entry(s).and_modify(|c| *c += 1).or_insert(1);
                }
            }
        }
        if sim_method == "aigp_all_all_10_1" || sim_method == "aigp_no.txt_all_10_1" {
            // let max_i = all_as.iter().map(|(_, i)| *i).max().unwrap_or(0);
            // for i in 1..=max_i {
            //     let n = all_as.iter().filter(|(_, j)| *j == i).count();
            //     println!("#{{|sim AS path|>={}}}={}", i, n);
            // }

            // // Write AS on each position of AS path
            // for i in 1..=max_i {
            //     let asns = all_as
            //         .iter()
            //         .filter(|(_, j)| *j <= i)
            //         .map(|(asn, _)| *asn)
            //         .collect::<HashSet<_>>();
            //     let file = File::create(format!("input/as_scope/aigp-{}-{}-.txt", args.region, i))?;
            //     let mut writer = std::io::BufWriter::new(file);
            //     println!("Writing {} AS to {}-{}", asns.len(), args.region, i);
            //     writeln!(
            //         writer,
            //         "{}",
            //         asns.into_iter()
            //             .map(|asn| asn.to_string())
            //             .collect::<Vec<_>>()
            //             .join(",")
            //     )?;
            // }

            // // calculate occurence of each AS
            // let mut as_count: HashMap<u32, u32> = HashMap::new();
            // for (asn, _) in all_as {
            //     *as_count.entry(asn).or_insert(0) += 1;
            // }

            // let mut as_count_vec: Vec<(u32, u32)> = as_count.into_iter().collect();
            // as_count_vec.sort_by(|a, b| b.1.cmp(&a.1));

            // // for k in [50] {
            // for k in [50, 60, 70, 80, 90, 100] {
            //     // output topK ASes
            //     let file =
            //         File::create(format!("input/as_scope/aigp-{}-top{}.txt", args.region, k))?;
            //     let mut writer = std::io::BufWriter::new(file);
            //     // println!("Writing top 50 AS to {}", args.region);
            //     // for (asn, count) in as_count_vec.iter().take(k) {
            //     //     println!("AS{}: {} times", asn, count);
            //     // }
            //     // output top50; including every AS that has the same count as the 50th AS
            //     // let threshold = as_count_vec[k - 1].1;
            //     // let top_as = as_count_vec
            //     //     .iter()
            //     //     .filter(|(_, count)| *count >= threshold)
            //     //     .map(|(asn, _)| *asn)
            //     //     .collect::<HashSet<_>>();
            //     let top_as = as_count_vec
            //         .iter()
            //         .take(k)
            //         .map(|(asn, _)| *asn)
            //         .collect::<HashSet<_>>();
            //     println!("Top {} AS size: {}", k, top_as.len());
            //     writeln!(
            //         writer,
            //         "{}",
            //         top_as
            //             .into_iter()
            //             .map(|asn| asn.to_string())
            //             .collect::<Vec<_>>()
            //             .join(",")
            //     )?;
            // }

            // Write all probe ASes
            let file = File::create(format!("input/as_scope/probes-{}.txt", args.region))?;
            let mut writer = std::io::BufWriter::new(file);
            println!("Writing {} AS to probes", probe_as.len());
            writeln!(
                writer,
                "{}",
                probe_as
                    .into_iter()
                    .map(|asn| asn.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )?;
        }
        println!(
            "Long distance traces (>16,000km): {}, no improvement: {}",
            n_long_distance, n_long_distance_no_improvement
        );
        let mx_as_len = as_path_length.keys().max().unwrap_or(&0);
        println!("Max AS path covered by traces: {}", mx_as_len);
        let n_ge_16: i32 = as_path_length
            .iter()
            .filter(|(&k, _)| k > 32)
            .map(|(_, &v)| v)
            .sum();
        let n = as_path_length.iter().map(|(_, &v)| v).sum::<i32>();
        println!(
            "AS path>32 covered by traces: {}; {}%",
            n_ge_16,
            n_ge_16 as f32 / n as f32 * 100.0
        );
        for (k, v) in rel_diff_stats.iter() {
            println!("{:?}: {} times", k, v);
        }

        for (k, v) in error_count.iter() {
            println!("Error {}: {} times", k, v);
        }
        // output top-10 degraded as

        let mut degraded_as: Vec<_> = degraded_as
            .into_iter()
            .filter(|(asn, _)| !metric_scope.contains(asn))
            .collect();
        degraded_as.sort_by(|a, b| b.1.cmp(&a.1));
        println!("Top degraded ASes:(n={})", degraded_as.len());
        for (asn, count) in degraded_as.iter().take(10) {
            println!("AS{}: {} times", asn, count);
        }
        let sum_all = degraded_as.iter().map(|(_, c)| *c).sum::<u32>();
        let sum_top10 = degraded_as.iter().take(10).map(|(_, c)| *c).sum::<u32>();
        // find the minimal set of ASes that contribute to 50% of the degradations
        let n_top_as = degraded_as
            .iter()
            .scan(0, |acc, (_, c)| {
                *acc += *c;
                Some((*acc, *c))
            })
            .take_while(|(acc, _)| *acc < sum_all * 9 / 10)
            .count()
            + 1;
        println!("Top ASes contributing to 90% of degradations: {}", n_top_as);
        println!(
            "Top 10 degraded ASes contribute {}/{} = {:.2}%",
            sum_top10,
            sum_all,
            sum_top10 as f32 / sum_all as f32 * 100.0
        );
        let degraded_top10 = degraded_as
            .iter()
            .take(10)
            .map(|(asn, _)| *asn)
            .collect::<HashSet<_>>();
        let mut metric_scope_ext = metric_scope.into_iter().collect::<HashSet<_>>();
        metric_scope_ext.extend(
            degraded_top10
                .iter()
                // .map(|(asn, _)| *asn)
                .collect::<HashSet<_>>(),
        );
        println!("Extended Metric Scope Size {}", metric_scope_ext.len());
        if args.extend {
            // write to file
            let ext = if sim_method.starts_with("aigp") {
                "aigp-ext"
            } else {
                // temporary naming fix
                &format!("ext-{}-5", args.region)
            };
            let metric_scope_path = format!("{}-{}", metric_scope_path, ext);
            let mut metric_scope_file = File::create(metric_scope_path)?;
            writeln!(
                metric_scope_file,
                "{}",
                metric_scope_ext
                    .iter()
                    .map(|asn| asn.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )?;
        }
        // println!(
        //     "Peer -> Provider: {} times",
        //     rel_diff_stats
        //         .get(&Some((
        //             topo::AsRel::PeerPeer,
        //             topo::AsRel::CustomerProvider,
        //         )))
        //         .unwrap_or(&0),
        // );

        if let Some(output_path) = &args.output_path {
            std::fs::create_dir_all(output_path.clone()).unwrap();
            let output_path = format!("{}/compare_ripe_{}.txt", output_path, sim_method);
            let fout = File::create(output_path)?;
            let mut writer = std::io::BufWriter::new(fout);
            for (prb_id, diff) in all_diff.iter() {
                writeln!(writer, "{} {} {}", prb_id, diff.0, diff.1)?;
            }
        }
        println!(
            "Matched {} out of {}; AS path matched {}/{}",
            all_diff.len(),
            processed_traces.len(),
            n_as_path_matched,
            all_diff.len()
        );
        let all_diff = all_diff
            .into_iter()
            .map(|(k, diff)| (k, diff.0 - diff.1))
            .collect::<HashMap<_, _>>();
        let n_pos = all_diff.values().filter(|&&d| d > 1000.0).count();
        let n_neg = all_diff.values().filter(|&&d| d < -500.0).count();
        let n_equal = all_diff.values().filter(|&&d| d == 0.0).count();
        println!(
            "Positive(trace>sim): {}, Negative(sim>trace): {}, Equal(trace=sim): {}",
            n_pos, n_neg, n_equal
        );
        // for (k, v) in rel_diff_stats.iter() {
        //     println!("{:?}->{:?}: {} times", k.0, k.1, v);
        // }
    }
    Ok(())
}
