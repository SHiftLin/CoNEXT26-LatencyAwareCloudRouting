// use database::PostgresClient;
use core::panic;
use database::PostgresClient;
use geographiclib_rs::InverseGeodesic;
use memmap::Mmap;
use rayon::prelude::*;
use serde::{
    Deserialize,
    de::{DeserializeOwned, Deserializer, Visitor},
};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    fs::File,
    io::{self, BufRead, BufReader},
    net::{IpAddr, Ipv4Addr},
    path::Path,
};

use topo::utils::LatLng;
#[macro_export]
macro_rules! def_field {
    ($func:ident,$ret:ty) => {
        fn $func(&self) -> &$ret;

        paste::paste! {
            fn [< $func _mut >] (&mut self) -> &mut $ret;
        }
    };
}

#[macro_export]
macro_rules! impl_field {
    ($func:ident,$ret:ty) => {
        fn $func(&self) -> &$ret {
            &self.$func
        }

        paste::paste! {
            fn [< $func _mut >] (&mut self) -> &mut $ret{
                &mut self.$func
            }
        }
    };
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct RawResult {
    from: Option<Ipv4Addr>,
    rtt: Option<f64>,
    ttl: Option<u32>,
    itos: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct RawRIPEHop {
    hop: u32,
    result: Vec<RawResult>,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct RawScamperHop {
    addr: Ipv4Addr,
    probe_ttl: u32,
    rtt: f64,
}

#[derive(Debug, Clone)]
pub struct RawTrace<T> {
    pub prb_id: Option<u32>,
    pub src: Option<Ipv4Addr>,
    pub dst: Option<Ipv4Addr>,
    pub hops: Vec<T>,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct Geometry {
    pub coordinates: (f32, f32),
    pub type_: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Hop {
    pub ip: Ipv4Addr,
    pub asn: Option<u32>,
    pub rtt: f64,
    pub ttl: u32,
    pub latlng: Option<LatLng>,
}

impl Hop {
    pub fn dist_to(&self, other: &Hop) -> Option<f64> {
        if let (Some(latlng1), Some(latlng2)) = (self.latlng, other.latlng) {
            let g = geographiclib_rs::Geodesic::wgs84();
            let dist: f64 = g.inverse(
                latlng1.0 as f64,
                latlng1.1 as f64,
                latlng2.0 as f64,
                latlng2.1 as f64,
            );
            Some(dist / 1000.0)
        } else {
            None
        }
    }
}
#[derive(Clone)]
pub struct Trace {
    pub prb_id: Option<u32>,
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    // hops[0]={src, asn(prb), 0, 0, geo(prb)}
    pub hops: Vec<Hop>,
}

impl fmt::Debug for Trace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Trace(prb_id: {:?}, src: {}, dst: {}, #hops: {})",
            self.prb_id,
            self.src,
            self.dst,
            self.hops.len()
        )?;
        for hop in &self.hops {
            writeln!(
                f,
                "\tHop(IP: {}, ASN: {:?}, RTT: {:.2}, TTL: {}, LatLng: {:?})",
                hop.ip, hop.asn, hop.rtt, hop.ttl, hop.latlng
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct IpInfo {
    pub ip: Ipv4Addr,
    pub asn: Option<u32>,
    pub latlng: Option<LatLng>,
}
#[derive(serde::Deserialize, Debug, Clone)]
pub struct RIPEProbeRaw {
    #[serde(rename = "id")]
    pub prb_id: u32,
    #[serde(rename = "asn_v4")]
    pub asn: Option<u32>,
    #[serde(rename = "address_v4")]
    pub addr: Option<Ipv4Addr>,
    pub geometry: Option<Geometry>,
}

#[derive(Debug)]
pub struct RIPEProbe {
    pub prb_id: u32,
    pub addr: Ipv4Addr,
    pub asn: u32,
    pub latlng: LatLng,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct RIPEProbeList {
    #[serde(rename = "objects")]
    pub probes: Vec<RIPEProbeRaw>,
}

// prb_id -> RIPEProbe(info)
type RIPEProbes = HashMap<u32, RIPEProbe>;

impl<'de, T> Deserialize<'de> for RawTrace<T>
where
    T: Deserialize<'de> + RawTraceHop,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RawTraceVisitor<T> {
            marker: std::marker::PhantomData<T>,
        }
        impl<'de, T> Visitor<'de> for RawTraceVisitor<T>
        where
            T: Deserialize<'de> + RawTraceHop,
        {
            type Value = RawTrace<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct RawTrace")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut prb_id = None;
                let mut src = None;
                let mut dst = None;
                let mut hops = Vec::new();

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "prb_id" => {
                            if prb_id.is_some() {
                                return Err(serde::de::Error::duplicate_field("prb_id"));
                            }
                            prb_id = Some(map.next_value()?);
                        }
                        "src_addr" | "src" => {
                            if src.is_some() {
                                return Err(serde::de::Error::duplicate_field("src_addr"));
                            }
                            src = Some(map.next_value()?);
                        }
                        "dst_addr" | "dst" => {
                            if dst.is_some() {
                                return Err(serde::de::Error::duplicate_field("dst_addr"));
                            }
                            dst = Some(map.next_value()?);
                        }
                        "result" | "hops" => {
                            if !hops.is_empty() {
                                return Err(serde::de::Error::duplicate_field("hops"));
                            }
                            hops = map.next_value()?;
                        }
                        _ => {
                            let _: serde::de::IgnoredAny = map.next_value()?;
                        }
                    }
                }

                // let prb_id = prb_id.ok_or_else(|| serde::de::Error::missing_field("prb_id"))?;
                // let src = src.ok_or_else(|| serde::de::Error::missing_field("src_addr"))?;
                // let dst = dst.ok_or_else(|| serde::de::Error::missing_field("dst_addr"))?;

                Ok(RawTrace {
                    prb_id,
                    src,
                    dst,
                    hops,
                })
            }
        }

        deserializer.deserialize_struct(
            "RawTrace",
            &["prb_id", "src_addr", "dst_addr", "hops"],
            RawTraceVisitor {
                marker: std::marker::PhantomData,
            },
        )
    }
}

pub trait RawTraceHop {
    fn ip(&self) -> Option<Ipv4Addr>;
    fn rtt(&self) -> f64;
    fn ttl(&self) -> u32;
}

impl RawTraceHop for RawRIPEHop {
    fn ip(&self) -> Option<Ipv4Addr> {
        self.result
            .iter()
            .find(|r| r.from.is_some())
            .and_then(|r| r.from)
    }
    fn rtt(&self) -> f64 {
        self.result
            .iter()
            .find(|r| r.rtt.is_some())
            .and_then(|r| r.rtt)
            .unwrap_or(0.0)
    }
    fn ttl(&self) -> u32 {
        self.hop
    }
}

impl RawTraceHop for RawScamperHop {
    fn ip(&self) -> Option<Ipv4Addr> {
        Some(self.addr)
    }
    fn rtt(&self) -> f64 {
        self.rtt
    }
    fn ttl(&self) -> u32 {
        self.probe_ttl
    }
}

impl Trace {
    pub fn route_dist_km(&self) -> f64 {
        let trace = Trace::reduce_intermediate(self);

        let g = geographiclib_rs::Geodesic::wgs84();
        let mut total_dist = 0.0;
        for i in 0..trace.hops.len() {
            if trace.hops[i].latlng.is_none() {
                // temporary
                panic!("Hop {} has no latlng: {:?}", i, trace.hops[i]);
            }
        }
        for i in 0..trace.hops.len() - 1 {
            if let (Some(latlng1), Some(latlng2)) = (trace.hops[i].latlng, trace.hops[i + 1].latlng)
            {
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

    pub fn refine_last_hop(trace: Trace, dst_hop: &Hop) -> Trace {
        let mut new_trace = trace.clone();
        if trace.hops.last().is_some() {
            let last_hop = new_trace.hops.last_mut().unwrap();
            if last_hop.ip != dst_hop.ip {
                new_trace.hops.push(dst_hop.clone());
            } else {
                // If it matches destination IP, rewrite ASN (eg. 396982-->15169)
                // meanwhile. rewrite ttl to 1023
                last_hop.asn = dst_hop.asn;
                last_hop.ttl = 1023
            }
        }
        return new_trace;
    }

    pub fn get_asn_ttl_range(trace: Trace) -> HashMap<u32, (usize, usize)> {
        let mut asn_range: HashMap<u32, (usize, usize)> = HashMap::new();
        for (i, hop) in trace.hops.iter().enumerate() {
            if let Some(asn) = hop.asn {
                asn_range
                    .entry(asn)
                    .and_modify(|(_, end)| {
                        *end = i;
                    })
                    .or_insert((i, i));
            }
        }
        asn_range
    }

    pub fn reduce_intermediate(trace: &Trace) -> Trace {
        let mut new_trace = trace.clone();
        new_trace.hops = trace
            .clone()
            .hops
            .into_iter()
            .filter(|hop| hop.asn.is_some() && hop.latlng.is_some())
            .collect();
        let asn_range = Trace::get_asn_ttl_range(new_trace.clone());
        new_trace.hops = new_trace
            .hops
            .into_iter()
            .enumerate()
            .filter_map(|(i, hop)| {
                if let Some(asn) = hop.asn {
                    let (start, end) = asn_range.get(&asn).unwrap();
                    if *start == i || *end == i {
                        return Some(hop);
                    } else {
                        return None;
                    }
                } else {
                    Some(hop)
                }
            })
            .collect();
        new_trace
    }
}

pub fn read_json_file<T>(filename: &str) -> io::Result<T>
where
    T: DeserializeOwned + Send,
{
    let file = File::open(filename)?;
    let reader = BufReader::new(file);
    let json: T = serde_json::from_reader(reader)?;
    Ok(json)
}

pub fn read_json_lines<T>(file_path: &str) -> io::Result<Vec<T>>
where
    T: DeserializeOwned + Send,
{
    let time = std::time::Instant::now();
    let file = File::open(file_path)?;
    // let _reader = BufReader::new(file);
    // SAFETY: the file is read-only and with valid utf-8 encoding
    let mmap = unsafe { Mmap::map(&file)? };

    let lines: Vec<String> = mmap.lines().filter_map(|l| l.ok()).collect();
    let parsed_data = lines
        .par_iter()
        .filter_map(|line| {
            serde_json::from_str::<T>(line)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
                .ok()
        })
        .collect::<Vec<T>>();
    println!(
        "Read {} lines, parsed {} lines, in {:?}",
        lines.len(),
        parsed_data.len(),
        time.elapsed()
    );
    Ok(parsed_data)
}

pub fn batch_read_json<T>(json_path_list: &[String]) -> io::Result<Vec<T>>
where
    T: DeserializeOwned + Send,
{
    let mut result = Vec::new();
    for json_path in json_path_list {
        let data = read_json_lines(json_path)?;
        result.extend(data);
    }
    Ok(result)
}

pub fn convert<T>(
    raw_trace: RawTrace<T>,
    ip_info: &HashMap<Ipv4Addr, IpInfo>,
    probes: &RIPEProbes,
) -> Option<Trace>
where
    T: RawTraceHop,
{
    let hops = raw_trace
        .hops
        .into_iter()
        .filter_map(|raw_hop| {
            let ip = raw_hop.ip();
            let ttl = raw_hop.ttl();
            let rtt = raw_hop.rtt();
            if let Some(ip) = ip {
                let info = ip_info.get(&ip);
                let (asn, latlng) = if let Some(info) = info {
                    (info.asn, info.latlng)
                } else {
                    (None, None)
                };
                return Some(Hop {
                    ip,
                    asn,
                    rtt,
                    ttl,
                    latlng,
                });
            } else {
                None
            }
        })
        .collect();
    // add hop[0] (probe itself, not in raw traceroutes)
    if let (Some(src), Some(dst)) = (raw_trace.src, raw_trace.dst) {
        if let Some(prb_id) = raw_trace.prb_id {
            if let Some(probe) = probes.get(&prb_id) {
                let lat_2 = (probe.latlng.0 * 100.0).round() / 100.0;
                let lng_2 = (probe.latlng.1 * 100.0).round() / 100.0;
                let prb_hop = Hop {
                    ip: src,
                    asn: Some(probe.asn),
                    rtt: 0.0,
                    ttl: 0,
                    latlng: Some((lat_2, lng_2)),
                };
                let mut probe_hop_vec = vec![prb_hop];
                probe_hop_vec.extend(hops);
                return Some(Trace {
                    prb_id: Some(prb_id),
                    src,
                    dst,
                    hops: probe_hop_vec,
                });
            }
        }
        Some(Trace {
            prb_id: raw_trace.prb_id,
            src,
            dst,
            hops,
        })
    } else {
        None
    }
}

pub fn batch_convert<T>(
    raw_trace_vec: Vec<RawTrace<T>>,
    ripe_probe_path: &str,
    ip_table: &str,
) -> Vec<Trace>
where
    T: RawTraceHop,
{
    let asn_key = "asn_bdrmapit";
    let mut client = PostgresClient::new().expect("Failed to create Postgres client");
    let ip_info = client
        .query(ip_table, &format!("ip, {}, lat, lng", asn_key), "true")
        .expect("Failed to query IP info");
    let probes = get_probes(ripe_probe_path);
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
            let asn: Option<i32> = row.get(asn_key);
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
    raw_trace_vec
        .into_iter()
        .filter_map(|raw_trace| convert(raw_trace, &ip_info, &probes))
        .collect()
}

// Extract the distinct IP addresses from raw traces file
// for python scripts to use (for geolocation)
pub fn extract_ip<T>(json_path: &str) -> Vec<Ipv4Addr>
where
    T: RawTraceHop + DeserializeOwned + Send,
{
    let trace_vec: Result<Vec<RawTrace<T>>, io::Error> = read_json_lines(json_path);
    if let Ok(trace_vec) = trace_vec {
        trace_vec
            .into_iter()
            .flat_map(|trace| trace.hops.into_iter().filter_map(|hop| hop.ip()))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<Ipv4Addr>>()
    } else {
        Vec::new()
    }
}

pub fn get_traces(
    trace_path: Vec<String>,
    ripe_probe_path: &str,
    ip_table: &str,
) -> Result<Vec<Trace>, io::Error> {
    let mut processed_traces = Vec::new();
    for path in &trace_path {
        println!("{}", path);
        if !Path::new(path).exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Trace file not found: {}", path),
            ));
        }
        let raw_trace_vec: Result<Vec<RawTrace<RawRIPEHop>>, io::Error> = read_json_lines(path);
        if let Ok(raw_trace_vec) = raw_trace_vec {
            let converted_traces = batch_convert(raw_trace_vec, ripe_probe_path, ip_table);
            processed_traces.extend(converted_traces);
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Failed to read raw traces",
            ));
        }
    }
    return Ok(processed_traces);
}

pub fn get_probes(json_path: &str) -> RIPEProbes {
    let probes: Result<Vec<RIPEProbeList>, io::Error> = read_json_lines(json_path);
    let probes = probes.unwrap();
    let probes = probes
        .first()
        .cloned()
        .unwrap_or(RIPEProbeList { probes: Vec::new() });

    probes
        .probes
        .into_iter()
        .filter_map(|probe| {
            if let (Some(addr), Some(latlng), Some(asn)) = (
                probe.addr,
                probe
                    .geometry
                    .as_ref()
                    .map(|g| (g.coordinates.1 as f32, g.coordinates.0 as f32)),
                // flipped to latitude, longitude, original ripe API gives [longitude, latitude]
                probe.asn,
            ) {
                Some((
                    probe.prb_id,
                    RIPEProbe {
                        prb_id: probe.prb_id,
                        addr,
                        asn,
                        latlng,
                    },
                ))
            } else {
                None
            }
        })
        .collect::<RIPEProbes>()
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::Value;

    use std::io::Write;
    use std::time::Instant;

    #[test]
    fn test_read_ripe() {
        let file_path = "../../input/ripe/msm_results/117777501.json";
        println!("test read");
        let result: Result<Vec<Value>, io::Error> = read_json_lines(file_path);
        assert!(
            result.is_ok(),
            "Failed to read JSON lines: {:?}",
            result.err()
        );
        let data = result.unwrap();
        println!("{:?}", data[0]);
        assert!(!data.is_empty(), "Expected non-empty data");

        let test_trace_vec = data
            .iter()
            .map(|v| serde_json::from_value::<RawTrace<RawRIPEHop>>(v.clone()).unwrap())
            .collect::<Vec<_>>();

        let ripe_probe_path = "../../input/ripe/20250713.json";
        let converted_traces = batch_convert(test_trace_vec, ripe_probe_path, "ip_geo_202506");

        let dst_hop = Hop {
            ip: [34, 47, 203, 230].into(),
            asn: Some(15169),
            rtt: 1000.0,
            ttl: 1023,
            latlng: Some((19.0728, 72.8826)),
        };
        for trace in converted_traces.iter().take(10) {
            let trace = Trace::refine_last_hop(trace.clone(), &dst_hop);
            let trace = Trace::reduce_intermediate(&trace);
            println!("{:?}", trace);
        }
        // for test_trace in converted_traces.iter().take(10) {
        //     println!(
        //         "Converted Trace: prb_id: {:?}, src: {}, dst: {}",
        //         test_trace.prb_id, test_trace.src, test_trace.dst
        //     );
        //     for hop in &test_trace.hops {
        //         println!(
        //             "Hop IP: {}, RTT: {}, TTL: {}, ASN: {:?}, Geo: {:?}",
        //             hop.ip, hop.rtt, hop.ttl, hop.asn, hop.latlng
        //         );
        //     }
        // }
    }

    #[test]
    fn test_read_scamper() {
        let time = Instant::now();

        let file_path = "/home/lsh/nfs/traceroutes/all/output_062025_asia-south1_10000.json";
        let test_trace_vec: Result<Vec<RawTrace<RawScamperHop>>, io::Error> =
            read_json_lines(file_path);
        assert!(
            test_trace_vec.is_ok(),
            "Failed to read JSON lines: {:?}",
            test_trace_vec.err()
        );
        let test_trace_vec = test_trace_vec.unwrap();
        assert!(!test_trace_vec.is_empty(), "Expected non-empty data");
        println!("Time(Read+Deserialize to Trace) in {:?}", time.elapsed());

        let time = Instant::now();
        let converted_traces = batch_convert(
            test_trace_vec,
            "../../input/ripe/20250713.json",
            "ip_geo_202506",
        );
        println!("Time(Convert) in {:?}", time.elapsed());
        assert!(converted_traces.len() == 14478333);

        let nhops_distribution = converted_traces.iter().map(|trace| trace.hops.len()).fold(
            HashMap::new(),
            |mut acc, nhops| {
                *acc.entry(nhops).or_insert(0) += 1;
                acc
            },
        );
        for (nhops, count) in nhops_distribution.iter() {
            println!("Number of hops: {}, Count: {}", nhops, count);
        }
        // for test_trace in converted_traces {
        //     println!(
        //         "Converted Trace: prb_id: {:?}, src: {}, dst: {}",
        //         test_trace.prb_id, test_trace.src, test_trace.dst
        //     );
        //     for hop in test_trace.hops {
        //         println!(
        //             "Hop IP: {}, RTT: {}, TTL: {}, ASN: {:?}, Geo: {:?}",
        //             hop.ip, hop.rtt, hop.ttl, hop.asn, hop.latlng
        //         );
        //     }
        // }
    }

    #[test]
    fn test_extract_ip() {
        let file_path = "../../input/ripe/msm_results/117777501.json";
        let ips: Vec<_> = extract_ip::<RawRIPEHop>(file_path)
            .iter()
            .map(|ip| ip.to_string())
            .collect();
        assert!(!ips.is_empty(), "Expected non-empty IPs");
        println!("Extracted IPs: #{:?}", ips.len());
        let distinct_ips: std::collections::HashSet<_> = ips.iter().collect();
        assert!(
            ips.len() == distinct_ips.len(),
            "{} {}",
            ips.len(),
            distinct_ips.len()
        );
        let output_path = "data/ip.txt";
        let mut file = File::create(output_path).unwrap();
        // for ip in distinct_ips {
        //     write!(file, "{}", ip).unwrap();
        // }
        write!(file, "{}", ips.join(" ")).unwrap();
    }
    #[test]
    fn test_ripe_probe() {
        let file_path = "../../input/ripe/20250713.json";
        let probes = get_probes(file_path);
        // output 10 probes
        assert!(!probes.is_empty(), "Expected non-empty probes");
        println!("Number of probes: {}", probes.len());
        // Print the first 10 probes
        for (i, (prb_id, probe)) in probes.iter().take(10).enumerate() {
            println!(
                "Probe {}: ID: {}, ASN: {:?}, LatLng: {:?}, Addr: {}",
                i + 1,
                prb_id,
                probe.asn,
                probe.latlng,
                probe.addr
            );
        }
    }
}
