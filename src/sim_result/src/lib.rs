use geographiclib_rs::InverseGeodesic;
use std::fmt;
use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufRead, BufReader},
};
use topo::{Topology, get_router, utils::decode_latlng};

type LatLng = (f32, f32);
type RouterId = u32;

pub struct ResultRIBEntry {
    pub next_hop: RouterId,
    pub router_from: RouterId,
    pub as_path_len: u32,
}

pub struct SimResult {
    pub latency: HashMap<RouterId, f64>,
    pub rib: HashMap<RouterId, ResultRIBEntry>,
}

#[derive(Debug)]
pub struct SimHop {
    pub router_id: RouterId,
    pub asn: Option<u32>,
    pub rtt: f64,
    pub latlng: Option<LatLng>,
}
pub struct SimTrace {
    pub src: RouterId,
    // hops here includes the source router for the ease of processing
    pub hops: Vec<SimHop>,
}

impl fmt::Debug for SimTrace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Sim Trace(router_id: {:?}, #hops: {})",
            self.src,
            self.hops.len()
        )?;
        for hop in &self.hops {
            writeln!(
                f,
                "\tHop(RouterID: {}, ASN: {:?}, RTT: {:.2}, LatLng: {:?})",
                hop.router_id, hop.asn, hop.rtt, hop.latlng
            )?;
        }
        Ok(())
    }
}
impl SimTrace {
    pub fn route_dist_km(&self) -> f64 {
        let g = geographiclib_rs::Geodesic::wgs84();
        let mut total_dist = 0.0;
        for i in 0..self.hops.len() - 1 {
            if let (Some(latlng1), Some(latlng2)) = (self.hops[i].latlng, self.hops[i + 1].latlng) {
                let dist: f64 = g.inverse(
                    latlng1.0 as f64,
                    latlng1.1 as f64,
                    latlng2.0 as f64,
                    latlng2.1 as f64,
                );
                total_dist += dist;
            }
        }
        total_dist / 1000.0
    }

    pub fn get_asn_rtt_range(&self) -> HashMap<u32, (f64, f64)> {
        let mut asn_range: HashMap<u32, (f64, f64)> = HashMap::new();
        for i in 0..self.hops.len() {
            // since the ebgp from hop to last_hop (if exists) is included by hop.asn, the distance also needs to be included in hop.asn.
            let hop = &self.hops[i];
            if let Some(asn) = hop.asn {
                let dist_l = if i == 0 {
                    self.hops[i].rtt
                } else {
                    self.hops[i - 1].rtt
                };
                asn_range
                    .entry(asn)
                    .and_modify(|(_, end)| {
                        *end = hop.rtt;
                    })
                    .or_insert((dist_l, hop.rtt));
            }
        }
        asn_range
    }
}

impl SimResult {
    pub fn print(&self) {
        let mut sorted_latency = self.latency.iter().map(|(_k, v)| v).collect::<Vec<_>>();
        sorted_latency.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("50%= {}", sorted_latency[sorted_latency.len() / 2]);
        println!("90%= {}", sorted_latency[sorted_latency.len() * 9 / 10]);
        println!("95%= {}", sorted_latency[sorted_latency.len() * 19 / 20]);
    }
}

pub fn read_rib_line(line: &str) -> Option<(RouterId, ResultRIBEntry)> {
    let parts = line.split(' ').collect::<Vec<&str>>();
    if parts.len() != 4 {
        return None;
    }
    let parts = parts
        .iter()
        .map(|s| s.trim_matches(|c| ":,".contains(c)))
        .collect::<Vec<&str>>();
    if let (Ok(router_id), Ok(next_hop), Ok(router_from), Ok(as_path_len)) = (
        parts[0].parse::<RouterId>(),
        parts[1].parse::<RouterId>(),
        parts[2].parse::<RouterId>(),
        parts[3].parse::<u32>(),
    ) {
        Some((
            router_id,
            ResultRIBEntry {
                next_hop,
                router_from,
                as_path_len,
            },
        ))
    } else {
        None
    }
}

pub fn read_result(latency_path: &str, rib_path: &str) -> Result<SimResult, io::Error> {
    let latency_file = BufReader::new(File::open(latency_path)?);
    let rib_file = BufReader::new(File::open(rib_path)?);

    let latency = latency_file
        .lines()
        .into_iter()
        .filter_map(|line| {
            let line = line.unwrap();
            let parts = line.split(' ').collect::<Vec<&str>>();
            if parts.len() == 2 {
                if let (Ok(router_id), Ok(latency)) =
                    (parts[0].parse::<RouterId>(), parts[1].parse::<f64>())
                {
                    Some((router_id, latency / 1000.0)) // Convert to ms
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect::<HashMap<RouterId, f64>>();

    let rib = rib_file
        .lines()
        .into_iter()
        .filter_map(|line| {
            // let line = line.unwrap();
            read_rib_line(&line.unwrap())
        })
        .collect::<HashMap<RouterId, ResultRIBEntry>>();
    Ok(SimResult { latency, rib })
}

// If given topo_opt, this function will return a complete route with asn/latlng annotation
// Otherwise, these two fields will be None for further processing
pub fn route(src: RouterId, result: &SimResult, topo_opt: Option<&Topology>) -> SimTrace {
    let mut hops = Vec::new();
    let mut cur = src;
    while let Some(entry) = result.rib.get(&cur) {
        let (asn, latlng) = if let Some(topo) = topo_opt {
            let router = get_router!(topo, cur);
            let latlng = router.latlng.map(|(lat, lng)| decode_latlng((lat, lng)));
            (router.asn, latlng)
        } else {
            (None, None) // Default ASN and latlng if no topology provided
        };

        hops.push(SimHop {
            router_id: cur,
            asn,
            rtt: 2.0 * result.latency.get(&cur).cloned().unwrap_or(0.0),
            latlng,
        });
        if entry.router_from == cur {
            if entry.next_hop == cur {
                break;
            }
            cur = entry.next_hop;
        } else {
            cur = entry.router_from;
        }
    }
    SimTrace { src, hops }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_read_rib_line() {
        let line = "1: 38693672, 1, 3";
        let result = read_rib_line(line);
        assert!(result.is_some(), "Expected Some result");
        let (router_id, entry) = result.unwrap();
        assert_eq!(router_id, 1);
        assert_eq!(entry.next_hop, 38693672);
        assert_eq!(entry.router_from, 1);
        assert_eq!(entry.as_path_len, 3);
    }
    #[test]
    pub fn test_read() {
        let method = "vanilla_no.txt_all_10_1";
        // let method = "aigp_all_all_10_1";
        let latency_path = format!(
            "../../output/cloud/asia-south/extra/asia-test/wan/caida-0824/latency_{}.txt",
            method
        );
        let rib_path = format!(
            "../../output/cloud/asia-south/extra/asia-test/wan/caida-0824/rib_{}.txt",
            method
        );
        let result = read_result(&latency_path, &rib_path);
        match result {
            Ok(res) => {
                assert!(!res.latency.is_empty(), "Latency map should not be empty");
                assert!(!res.rib.is_empty(), "RIB map should not be empty");
                res.print();
                let router_id = 3223;
                let route = route(router_id, &res, None);
                println!("{:?}", route);
            }
            Err(e) => panic!("Failed to read result: {}", e),
        }
    }
}
