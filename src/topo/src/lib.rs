use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io;
use std::io::Write;
use std::net::Ipv4Addr;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::Instant;
pub mod utils;
use fastmap::FastMap;
use geographiclib_rs::{Geodesic, InverseGeodesic};
use rayon::prelude::*;
use utils::{
    decode_latlng, distance_to_latency, encode_latlng, geo_distance, pair_ord, CAIDAReader,
    LatLngInt, Reader, Writer,
};

pub const MAX_CAP: u32 = 1.2e8 as u32;
pub const MAX_NID: u32 = 1.1e8 as u32;
pub const MAX_AS: u32 = 3e6 as u32;
pub const HUB_ASN: u32 = 65534;
pub const IXP_ASN: u32 = 65535;

macro_rules! new_writer {
    ($filename:expr, $enable:expr) => {
        if $enable {
            Writer::new($filename)?
        } else {
            Writer::new("/dev/null")?
        }
    };
}

macro_rules! write_line {
    ($writer:expr, $enable:expr, $line:expr) => {
        if $enable {
            $writer.write_line($line)?;
        }
    };
}

pub struct Converter {
    switch_range: u32,
    pub links: Vec<Vec<(u32, Option<Ipv4Addr>)>>,
    pub router_as: FastMap<Option<NonZeroU32>>,
}

impl Converter {
    pub fn new() -> Self {
        Converter {
            switch_range: MAX_NID,
            links: Vec::with_capacity(MAX_NID as usize),
            router_as: FastMap::new(MAX_CAP),
        }
    }

    pub fn read_routers_ifaces_raw(
        &mut self,
        filename: &str,
        dst: &str,
        to_write: bool,
    ) -> io::Result<()> {
        let start = Instant::now();
        let reader = CAIDAReader::new(filename)?;
        let mut writer = new_writer!(dst, to_write);
        for line in reader {
            if let Some((node, ifaces_str)) = line.split_once(':') {
                let nid = node.trim().split_ascii_whitespace().collect::<Vec<&str>>()[1];
                let nid: u32 = nid[1..].parse().unwrap();
                write_line!(writer, to_write, format!("{}:{}", nid, ifaces_str));
            }
        }
        println!(
            "Read routers and interfaces finished in {}s.",
            start.elapsed().as_secs()
        );
        Ok(())
    }

    pub fn read_geo_raw(&mut self, filename: &str, dst: &str, to_write: bool) -> io::Result<()> {
        let start = Instant::now();
        let reader = CAIDAReader::new(filename)?;
        let mut writer = new_writer!(dst, to_write);
        for line in reader {
            if let Some((node, geo)) = line.split_once(':') {
                let items: Vec<&str> = node.trim().split_ascii_whitespace().collect();
                let nid: u32 = items[1][1..].parse().unwrap();
                let items: Vec<&str> = geo.trim().split('\t').collect();
                let lat: f32 = items[items.len() - 5].parse().unwrap();
                let lng: f32 = items[items.len() - 4].parse().unwrap();
                write_line!(writer, to_write, format!("{} {} {}", nid, lat, lng));
            }
        }
        println!("Read geo finished in {}s.", start.elapsed().as_secs());
        Ok(())
    }

    pub fn read_links_raw(&mut self, filename: &str, dst: &str, to_write: bool) -> io::Result<()> {
        let start = Instant::now();
        let reader = CAIDAReader::new(filename)?;
        let mut writer = new_writer!(dst, to_write);
        for line in reader {
            if let Some((_link, nodes)) = line.split_once(':') {
                let mut nids: Vec<(u32, Option<Ipv4Addr>)> = Vec::with_capacity(4);
                for node in nodes.trim().split_ascii_whitespace() {
                    let nid = if let Some((id, iface)) = node.split_once(':') {
                        (id[1..].parse().unwrap(), Some(iface.parse().unwrap()))
                    } else {
                        (node[1..].parse().unwrap(), None)
                    };
                    nids.push(nid);
                }
                if nids.len() < 2 {
                    continue;
                }
                if nids.len() == 2 {
                    let a = nids[0].0;
                    let b = nids[1].0;
                    write_line!(writer, to_write, format!("{} {}", a, b));
                    // write_line!(writer, to_write, format!("{} {}", b, a));
                } else {
                    self.switch_range += 1;
                    for &i in &nids {
                        write_line!(writer, to_write, format!("{} {}", i.0, self.switch_range));
                        // write_line!(writer, to_write, format!("{} {}", self.switch_range, i.0));
                    }
                }
                self.links.push(nids);
            }
        }
        println!(
            "Read links finished in {}s. switch_range: {}, switch_cnt: {}",
            start.elapsed().as_secs(),
            self.switch_range,
            self.switch_range - MAX_NID
        );
        Ok(())
    }

    pub fn read_as_raw(&mut self, filename: &str, dst: &str, to_write: bool) -> io::Result<()> {
        let start = Instant::now();
        let reader = CAIDAReader::new(filename)?;
        let mut writer = new_writer!(dst, to_write);
        for line in reader {
            let items: Vec<&str> = line.trim().split_ascii_whitespace().collect();
            let nid: u32 = items[1][1..].parse().unwrap();
            let asn: i32 = items[2].parse().unwrap();
            if asn == -1 {
                continue;
            }
            self.router_as
                .insert_unsafe(nid, NonZeroU32::new(asn as u32));
            write_line!(writer, to_write, format!("{} {}", nid, asn));
        }
        println!("Read AS finished in {}s.", start.elapsed().as_secs());
        Ok(())
    }
}

pub type RouterId = u32;
pub type AsNumber = u32;

#[derive(Default, Clone)]
pub struct Router {
    pub id: RouterId,
    pub asn: Option<AsNumber>,
    pub border: bool,
    pub reflector: bool,
    pub ifaces: Vec<Ipv4Addr>,
    pub links: Vec<RouterId>,
    pub border_links: Vec<RouterId>,
    pub ebgp_cnt: usize, // Usually border_links.len() - 1 (route reflector)
    pub latlng: Option<LatLngInt>,
}

impl Router {
    pub fn new(id: RouterId) -> Self {
        Router {
            id: id,
            asn: None,
            border: false,
            reflector: false,
            ifaces: Vec::new(),
            links: Vec::new(),
            border_links: Vec::new(),
            ebgp_cnt: 0,
            latlng: None,
        }
    }
}

#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub enum AsRel {
    None,
    ProviderCustomer,
    CustomerProvider,
    PeerPeer,
}

impl Default for AsRel {
    fn default() -> Self {
        AsRel::PeerPeer
    }
}

impl Ord for AsRel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        use AsRel::*;

        match (self, other) {
            (None, None)
            | (ProviderCustomer, ProviderCustomer)
            | (CustomerProvider, CustomerProvider)
            | (PeerPeer, PeerPeer) => Equal,

            (None, _) => Greater, // If the relationship is missing
            (_, None) => Less,

            (ProviderCustomer, CustomerProvider) => Less,
            (CustomerProvider, ProviderCustomer) => Greater,

            (ProviderCustomer, PeerPeer) => Less,
            (PeerPeer, ProviderCustomer) => Greater,

            // Following Gao-Rexford Guideline B, the preference of Peer and Customer could be equal
            // (ProviderCustomer, PeerPeer) => Equal,
            // (PeerPeer, ProviderCustomer) => Equal,
            (PeerPeer, CustomerProvider) => Less,
            (CustomerProvider, PeerPeer) => Greater,
        }
    }
}

impl PartialOrd for AsRel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub struct Topology {
    user: bool,
    switch_range: u32,
    // reflector_range: u32,
    pub reflectors: HashSet<RouterId>,

    pub routers: HashSet<RouterId>,
    // Use get_router method to avoid error.
    pub router_info: FastMap<Option<Router>>,
    pub iface_router: BTreeMap<Ipv4Addr, RouterId>,
    pub as_routers: FastMap<Vec<RouterId>>,
    pub router_as_list: HashSet<AsNumber>,
    pub router_link_cnt: usize,
    pub link_weight: HashMap<(RouterId, RouterId), u32>,
    // Use RefCell to allow modify it through &Topology
    pub geo_weight: RefCell<HashMap<(LatLngInt, LatLngInt), u32>>,

    pub as_rel: HashMap<(AsNumber, AsNumber), AsRel>,
    pub stub_as: HashSet<AsNumber>,
    pub single_homed_stub_as: HashSet<AsNumber>,

    pub borders: HashSet<RouterId>,
    pub as_borders: FastMap<Vec<RouterId>>,
    pub border_as_list: HashSet<AsNumber>,
    pub border_link_cnt: usize,

    pub router_addrs: HashSet<Ipv4Addr>,
    pub ixp: Vec<RouterId>,
}

// Use macro instead of fn to avoid mutability check error
#[macro_export]
macro_rules! get_router {
    ($topo:expr,$id:expr) => {
        $topo.router_info.get_unsafe($id).as_ref().unwrap()
    };
}

#[macro_export]
macro_rules! get_router_mut {
    ($topo:expr,$id:expr) => {
        $topo.router_info.get_mut_unsafe($id).as_mut().unwrap()
    };
}

#[macro_export]
macro_rules! get_router_or_new_mut {
    ($topo:expr,$id:expr) => {
        $topo
            .router_info
            .get_mut_unsafe($id)
            // Use get_or_insert_with instead of get_or_insert to avoid duplicate new
            .get_or_insert_with(|| Router::new($id))
    };
}

impl Topology {
    pub fn new(user: bool) -> Self {
        Self::with_capacity(user, MAX_CAP, MAX_AS)
    }

    fn with_capacity(user: bool, router_capacity: u32, as_capacity: u32) -> Self {
        Topology {
            user: user,
            switch_range: MAX_NID,
            // reflector_range: MAX_NID,
            reflectors: Default::default(),

            routers: Default::default(),
            router_info: FastMap::new(router_capacity),
            iface_router: Default::default(),
            as_routers: FastMap::new(as_capacity),
            router_as_list: Default::default(),
            router_link_cnt: 0,
            link_weight: Default::default(),
            geo_weight: Default::default(),

            // provider_customer: Default::default(),
            // peer_peer: Default::default(),
            as_rel: Default::default(),
            stub_as: Default::default(),
            single_homed_stub_as: Default::default(),

            borders: Default::default(),
            as_borders: FastMap::new(as_capacity),
            border_as_list: Default::default(),
            border_link_cnt: 0,

            router_addrs: HashSet::new(),
            ixp: Default::default(),
        }
    }

    /// Load a small, explicitly specified topology without Internet-scale allocations.
    pub fn read_user_from_path(input: &Path) -> io::Result<Self> {
        let mut max_router = 0;
        let mut max_as = 0;
        for (filename, router_columns) in [("links.in", 2), ("as.in", 1)] {
            for line in Reader::new(input.join(filename))? {
                let fields: Vec<_> = line.split_ascii_whitespace().collect();
                if fields.len() < 2 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Expected at least two columns",
                    ));
                }
                for field in &fields[..router_columns] {
                    let id: u32 = field
                        .parse()
                        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                    max_router = max_router.max(id);
                }
                if filename == "as.in" {
                    let asn: u32 = fields[1]
                        .parse()
                        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                    max_as = max_as.max(asn);
                }
            }
        }
        let router_capacity = max_router
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Router ID too large"))?;
        let as_capacity = max_as
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "AS number too large"))?;
        let mut topo = Self::with_capacity(true, router_capacity, as_capacity);
        topo.read_links(input.join("links.in"))?;
        match topo.read_reflectors(input.join("reflectors.in")) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        topo.read_as(input.join("as.in"))?;
        topo.read_as_rel(input.join("as-rel.in"))?;
        topo.build_border_graph();
        Ok(topo)
    }

    pub fn read_from_path(
        user: bool,
        input: &PathBuf,
        extra_input: &Vec<PathBuf>,
    ) -> Result<Self, io::Error> {
        let mut topo = Self::new(user);
        topo.read_addrs(input.join("itdk-run-20240828.addrs"))?;
        topo.read_routers_ifaces(input.join("midar-iff.nodes.in"))?;
        topo.read_geo(input.join("midar-iff.nodes.geo.in"))?;
        topo.read_as(input.join("midar-iff.nodes.as.in"))?;
        topo.read_as_rel(input.join("20240801.as-rel2.txt"))?;
        let mut nid_offset = topo.read_links(input.join("midar-iff.links.in"))?;
        // the following reads are offset by nid_offset (max router id so far)
        for extra_input in extra_input {
            println!("Reading extra topology from {}", extra_input.display());
            let new_nid_offset = topo.read_links_extra(extra_input.join("links.in"), nid_offset)?;
            topo.read_as_extra(extra_input.join("nodes.as.in"), nid_offset)?;
            topo.read_geo_extra(extra_input.join("nodes.geo.in"), nid_offset)?;
            let router_ifaces_path = extra_input.join("nodes.in");
            if router_ifaces_path.exists() {
                topo.read_router_ifaces_extra(router_ifaces_path, nid_offset)?;
            } else {
                println!("No nodes.in found in {}", extra_input.display());
            }
            let as_rel_path = extra_input.join("as-rel.txt");
            if as_rel_path.exists() {
                topo.read_as_rel(as_rel_path)?;
            } else {
                println!("No as-rel.txt found in {}", extra_input.display());
            }
            nid_offset = new_nid_offset
        }
        topo.ixp_detection();
        topo.build_border_graph();
        Ok(topo)
    }

    pub fn is_reflector(&self, router: &Router) -> bool {
        // self.switch_range < *id && *id <= self.reflector_range
        router.reflector
    }

    pub fn is_switch(&self, id: &RouterId) -> bool {
        MAX_NID < *id && *id <= self.switch_range
    }

    pub fn get_weight(&self, a: &Router, b: &Router) -> Option<u32> {
        if self.user {
            self.link_weight.get(&pair_ord(a.id, b.id)).map(|x| *x)
        } else {
            let loc_a = a.latlng;
            let loc_b = b.latlng;
            if loc_a.is_some() && loc_b.is_some() {
                let locs = pair_ord(loc_a.unwrap(), loc_b.unwrap());
                match self.geo_weight.borrow_mut().entry(locs) {
                    Entry::Occupied(entry) => Some(*entry.get()),
                    Entry::Vacant(entry) => {
                        let value = distance_to_latency(geo_distance(
                            decode_latlng(locs.0),
                            decode_latlng(locs.1),
                        ));
                        entry.insert(value);
                        Some(value)
                    }
                }
            } else {
                None
            }
        }
    }

    pub fn read_addrs<P>(&mut self, filename: P) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        let start = Instant::now();
        let reader = Reader::new(filename)?;
        for line in reader {
            let items: Vec<&str> = line.split_ascii_whitespace().collect();
            let addr: Ipv4Addr = items[0].parse().unwrap();
            self.router_addrs.insert(addr);
        }
        println!(
            "Read router addresses finished in {}s. router_cnt: {}",
            start.elapsed().as_secs(),
            self.router_addrs.len()
        );
        Ok(())
    }

    pub fn read_routers_ifaces<P>(&mut self, filename: P) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        let start = Instant::now();
        let reader = Reader::new(filename)?;
        let content = reader
            .lines
            .filter_map(|line| line.ok())
            .collect::<Vec<_>>();
        let nid_ifaces = content
            .par_iter()
            .filter_map(|line| {
                if let Some((nid, ifaces_str)) = line.split_once(':') {
                    let nid = nid.trim().parse::<u32>().ok()?;
                    let ifaces: Vec<Ipv4Addr> = ifaces_str
                        .trim()
                        .split_ascii_whitespace()
                        .map(|x| x.parse().ok())
                        .collect::<Option<Vec<_>>>()?;
                    Some((nid, ifaces))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let iface_routers: Vec<_> = nid_ifaces
            .par_iter()
            .flat_map(|(nid, _ifaces)| _ifaces.par_iter().map(move |iface| (*iface, *nid)))
            .collect();
        self.iface_router.extend(iface_routers);
        println!("Elapsed: {}", start.elapsed().as_secs());

        let routers: Vec<_> = nid_ifaces
            .par_iter()
            .filter_map(|(nid, ifaces)| {
                if ifaces.iter().any(|iface| self.router_addrs.contains(iface)) || ifaces.len() >= 2
                {
                    Some(*nid)
                } else {
                    None
                }
            })
            .collect();

        self.routers.extend(routers);
        // for (nid, ifaces) in nid_ifaces {
        //     let mut is_router = false;
        //     for iface in &ifaces {
        //         if self.router_addrs.contains(iface) {
        //             is_router = true;
        //         }
        //         // self.iface_router.insert(*iface, nid);
        //     }
        //     // let router = get_router_or_new_mut!(self, nid);
        //     // router.ifaces = ifaces;
        //     // We skip this since we will do it in read_links
        //     if is_router || ifaces.len() >= 2 {
        //         self.routers.insert(nid);
        //     }
        // }
        println!(
            "Read routers and interfaces finished in {}s. router_cnt: {}",
            start.elapsed().as_secs(),
            self.routers.len()
        );
        Ok(())
    }

    pub fn read_router_ifaces_extra<P>(&mut self, filename: P, nid_offset: u32) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        let start = Instant::now();
        let reader = Reader::new(filename)?;
        for line in reader {
            let (nid, ifaces_str) = line.split_once(' ').unwrap();
            let nid = nid.parse::<u32>().unwrap() + nid_offset;
            let ifaces: Vec<Ipv4Addr> = ifaces_str
                .trim()
                .split_ascii_whitespace()
                .map(|x| x.parse().unwrap())
                .collect();
            for iface in &ifaces {
                self.iface_router.insert(*iface, nid);
            }
        }
        println!(
            "Read routers and interfaces(Extra) finished in {}s.",
            start.elapsed().as_secs()
        );
        Ok(())
    }
    pub fn read_geo<P>(&mut self, filename: P) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        let start = Instant::now();
        let reader = Reader::new(filename)?;
        for line in reader {
            let items: Vec<&str> = line.split_ascii_whitespace().collect();
            let nid: u32 = items[0].parse().unwrap();
            let lat: f32 = items[1].parse().unwrap();
            let lng: f32 = items[2].parse().unwrap();
            if self.routers.contains(&nid) {
                let router = get_router_or_new_mut!(self, nid);
                router.latlng = Some(encode_latlng((lat, lng)));
            }
        }
        println!("Read geo finished in {}s.", start.elapsed().as_secs());
        Ok(())
    }

    pub fn read_geo_extra<P>(&mut self, filename: P, nid_offset: u32) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        let start = Instant::now();
        let reader = Reader::new(filename)?;
        for line in reader {
            let items: Vec<&str> = line.split_ascii_whitespace().collect();
            let nid: u32 = items[0].parse::<u32>().unwrap() + nid_offset;
            let lat: f32 = items[1].parse().unwrap();
            let lng: f32 = items[2].parse().unwrap();
            // extra nodes, which come from traceroutes, are all considered as routers;
            // we do not check if this node is a router in itdk
            // if self.routers.contains(&nid) {
            let router = get_router_or_new_mut!(self, nid);
            router.latlng = Some(encode_latlng((lat, lng)));
            // }
        }
        println!(
            "Read geo(Extra) finished in {}s.",
            start.elapsed().as_secs()
        );
        Ok(())
    }

    pub fn read_links<P>(&mut self, filename: P) -> io::Result<u32>
    where
        P: AsRef<Path>,
    {
        let start = Instant::now();
        let reader = Reader::new(filename)?;
        for line in reader {
            let items: Vec<&str> = line.split_ascii_whitespace().collect();
            let a = items[0].parse().unwrap();
            let b = items[1].parse().unwrap();

            let mut is_link = true;
            if items.len() >= 3 {
                self.link_weight
                    .insert(pair_ord(a, b), items[2].parse().unwrap());
                if items.len() >= 4 {
                    if items[3] == "G" {
                        // To indicate the distance only. Do not have a connection
                        is_link = false;
                    }
                }
            }
            // Use get_router_or_new_mut here so that we can skip read_routers_ifaces to save time
            if !self.user
                && ((a < MAX_NID && !self.routers.contains(&a))
                    || (b < MAX_NID && !self.routers.contains(&b)))
            {
                continue;
            }
            let ra = get_router_or_new_mut!(self, a);
            if is_link {
                ra.links.push(b)
            };
            let rb = get_router_or_new_mut!(self, b);
            if is_link {
                rb.links.push(a);
            }
            self.switch_range = std::cmp::max(self.switch_range, std::cmp::max(a, b));
        }
        let max_router_id = self
            .router_info
            .keys(move |v| matches!(v, Some(r) if r.id < MAX_NID))
            .into_iter()
            .max()
            .unwrap_or(0);
        self.routers = self.router_info.keys(|v| v.is_some()).into_iter().collect();
        self.router_link_cnt = self
            .router_info
            .values_mut()
            .map(|v| {
                if let Some(r) = v {
                    // This is necessary since the input indeed includes duplicate links
                    r.links.sort_unstable();
                    r.links.dedup();
                    r.links.len()
                } else {
                    0
                }
            })
            .sum::<usize>()
            / 2;
        println!(
            "Read links finished in {}s. switch_range: {}, switch_cnt: {}, max_router_id: {}",
            start.elapsed().as_secs(),
            self.switch_range,
            self.switch_range - MAX_NID,
            max_router_id
        );
        Ok(max_router_id)
    }

    pub fn read_links_extra<P>(&mut self, filename: P, nid_offset: u32) -> io::Result<u32>
    where
        P: AsRef<Path>,
    {
        let start = Instant::now();
        let reader = Reader::new(filename)?;
        for line in reader {
            let items: Vec<&str> = line.split_ascii_whitespace().collect();
            let a = items[0].parse::<u32>().unwrap() + nid_offset;
            let b = items[1].parse::<u32>().unwrap() + nid_offset;

            // extra links are all weighted by geographic distance

            // Use get_router_or_new_mut here so that we can skip read_routers_ifaces to save time
            // if (a < MAX_NID && !self.routers.contains(&a))
            //     || (b < MAX_NID && !self.routers.contains(&b))
            // {
            //     continue;
            // }

            // extra links contains no switches
            let ra = get_router_or_new_mut!(self, a);
            ra.links.push(b);
            let rb = get_router_or_new_mut!(self, b);
            rb.links.push(a);
        }
        let max_router_id = self
            .router_info
            .keys(move |v| matches!(v, Some(r) if r.id < MAX_NID))
            .into_iter()
            .max()
            .unwrap_or(0);
        self.routers = self.router_info.keys(|v| v.is_some()).into_iter().collect();
        self.router_link_cnt = self
            .router_info
            .values_mut()
            .map(|v| {
                if let Some(r) = v {
                    // This is necessary since the input indeed includes duplicate links
                    r.links.sort_unstable();
                    r.links.dedup();
                    r.links.len()
                } else {
                    0
                }
            })
            .sum::<usize>()
            / 2;
        println!(
            "Read links(extra) finished in {}s. switch_range: {}, switch_cnt: {}, max_router_id: {}",
            start.elapsed().as_secs(),
            self.switch_range,
            self.switch_range - MAX_NID,
            max_router_id
        );
        Ok(max_router_id)
    }

    pub fn read_reflectors(&mut self, filename: impl AsRef<Path>) -> io::Result<()> {
        let start = Instant::now();
        let reader = Reader::new(filename)?;
        for line in reader {
            let nid = line.parse().unwrap();
            let router = self
                .router_info
                .get_mut(nid)
                .and_then(Option::as_mut)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Reflector is not in the topology",
                    )
                })?;
            router.reflector = true;
            self.reflectors.insert(nid);
        }
        println!(
            "Read route reflectors finished in {}s. reflector_cnt: {}",
            start.elapsed().as_secs(),
            self.reflectors.len()
        );
        Ok(())
    }

    pub fn read_as<P>(&mut self, filename: P) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        let start = Instant::now();
        let reader = Reader::new(filename)?;
        for line in reader {
            let (nid, asn) = line.split_once(' ').unwrap();
            let nid = nid.parse().unwrap();
            let asn = asn.parse().unwrap();
            if asn == 137409 {
                continue;
            }
            // if asn == 15169 {
            //     self.routers.insert(nid);
            // }
            if self.routers.contains(&nid) {
                get_router_or_new_mut!(self, nid).asn = Some(asn);
            }
            self.as_routers.get_mut_unsafe(asn).push(nid);
        }

        self.router_as_list = self
            .as_routers
            .keys(|vs| vs.len() > 0)
            .into_iter()
            .collect();
        println!("Read AS finished in {}s.", start.elapsed().as_secs());
        Ok(())
    }

    pub fn read_as_extra<P>(&mut self, filename: P, nid_offset: u32) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        let start = Instant::now();
        let reader = Reader::new(filename)?;
        for line in reader {
            let (nid, asn) = line.split_once(' ').unwrap();
            let nid = nid.parse::<u32>().unwrap() + nid_offset;
            let asn = asn.parse().unwrap();
            // extra node are all routers
            // We do not check if this node is a router in itdk
            // if self.routers.contains(&nid) {
            get_router_or_new_mut!(self, nid).asn = Some(asn);
            // }
            self.as_routers.get_mut_unsafe(asn).push(nid);
        }

        self.router_as_list = self
            .as_routers
            .keys(|vs| vs.len() > 0)
            .into_iter()
            .collect();
        println!("Read AS(Extra) finished in {}s.", start.elapsed().as_secs());
        Ok(())
    }

    pub fn read_as_rel<P>(&mut self, filename: P) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        let start = Instant::now();
        let reader = CAIDAReader::new(&filename)?;
        for line in reader {
            let items: Vec<&str> = line.split('|').collect();
            let as1 = items[0].parse().unwrap();
            let as2 = items[1].parse().unwrap();
            if items[2] == "-1" {
                // self.provider_customer.insert((as1, as2))
                if as1 == 38731 && as2 == 15169 {
                    println!("{} -> {} from {}", as1, as2, filename.as_ref().display());
                }
                self.as_rel.insert((as1, as2), AsRel::ProviderCustomer);
                self.as_rel.insert((as2, as1), AsRel::CustomerProvider);
            } else {
                // self.peer_peer.insert((as1, as2));
                // self.peer_peer.insert((as2, as1));
                self.as_rel.insert((as1, as2), AsRel::PeerPeer);
                self.as_rel.insert((as2, as1), AsRel::PeerPeer);
            }
        }
        println!(
            "Read AS relationship finished in {}s.",
            start.elapsed().as_secs()
        );
        Ok(())
    }
    pub fn read_as_rel_extra<P>(&mut self, filename: P) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        let start = Instant::now();
        let reader = CAIDAReader::new(filename)?;
        for line in reader {
            let items: Vec<&str> = line.split('|').collect();
            let as1 = items[0].parse().unwrap();
            let as2 = items[1].parse().unwrap();
            if items[2] == "-1" {
                // self.provider_customer.insert((as1, as2));
                self.as_rel.insert((as1, as2), AsRel::ProviderCustomer);
                self.as_rel.insert((as2, as1), AsRel::CustomerProvider);
            } else {
                // self.peer_peer.insert((as1, as2));
                // self.peer_peer.insert((as2, as1));
                self.as_rel.insert((as1, as2), AsRel::PeerPeer);
                self.as_rel.insert((as2, as1), AsRel::PeerPeer);
            }
        }
        println!(
            "Read AS relationship(extra) finished in {}s.",
            start.elapsed().as_secs()
        );
        Ok(())
    }

    pub fn ixp_detection(&mut self) {
        // assign AS number to switch
        let start = Instant::now();
        let mut no_asn = 0;
        let mut asn_cnt = 0;
        let mut hub_cnt = 0;
        for &nid in &self.routers {
            if self.is_switch(&nid) {
                let mut router = self.router_info.remove_unsafe(nid).unwrap();
                let mut asns = HashSet::new();
                for &vid in &router.links {
                    if let Some(asn) = get_router!(self, vid).asn {
                        asns.insert(asn);
                    } else {
                        //ignore
                    }
                }
                if asns.len() == 0 {
                    no_asn += 1;
                } else if asns.len() == 1 {
                    let asn = asns.into_iter().next().unwrap();
                    self.as_routers.get_mut_unsafe(asn).push(nid);
                    router.asn = Some(asn);
                    asn_cnt += 1;
                } else if asns.len() == 2 {
                    // every HUB of two ASs is assigned ASN `HUB_ASN`
                    self.as_routers.get_mut_unsafe(HUB_ASN).push(nid);
                    router.asn = Some(HUB_ASN);
                    hub_cnt += 1;
                } else if asns.len() > 2 {
                    // every IXP is assigned ASN `IXP_ASN`
                    self.as_routers.get_mut_unsafe(IXP_ASN).push(nid);
                    router.asn = Some(IXP_ASN);
                    self.ixp.push(nid);
                }
                self.router_info.insert_unsafe(nid, Some(router));
            }
        }
        println!(
            "AS assignment to switch finished in {}s. no_asn: {}, asn: {}, hub: {}, ixp: {}",
            start.elapsed().as_secs(),
            no_asn,
            asn_cnt,
            hub_cnt,
            self.ixp.len()
        );
    }

    pub fn dijkstra(
        src: &RouterId,
        nodes: &Vec<RouterId>,
        router_info: &FastMap<Option<Router>>,
        link_weight: &mut HashMap<(RouterId, RouterId), u32>,
    ) {
        let nodes: HashSet<u32> = HashSet::from_iter(nodes.iter().cloned());
        let mut heap = BinaryHeap::new();
        heap.push(Reverse((0, *src)));
        let mut dists = HashMap::from([(*src, 0)]);
        let mut vis = HashSet::new();
        while !heap.is_empty() {
            let Reverse((distu, u)) = heap.pop().unwrap();
            match vis.get(&u) {
                Some(_) => continue,
                None => {
                    vis.insert(u);
                    link_weight.insert(pair_ord(*src, u), distu);
                }
            };
            let ru = router_info.get_unsafe(u).as_ref().unwrap();
            for v in &ru.border_links {
                if !nodes.contains(v) || vis.contains(v) {
                    continue;
                }
                let w = *link_weight.get(&pair_ord(u, *v)).unwrap();
                let distv = dists.entry(*v).or_insert(u32::MAX);
                if distu + w < *distv {
                    *distv = distu + w;
                    heap.push(Reverse((*distv, *v)));
                }
            }
        }
    }

    pub fn connected_components(
        nodes: &Vec<RouterId>,
        router_info: &FastMap<Option<Router>>,
    ) -> Vec<Vec<u32>> {
        let mut components = Vec::new();

        let nodes: HashSet<u32> = HashSet::from_iter(nodes.iter().cloned());
        let mut visited = HashSet::new();
        let mut q = VecDeque::new();
        for node in &nodes {
            if visited.contains(node) {
                continue;
            }

            let mut component = Vec::new();
            visited.insert(*node);
            q.push_back(*node);
            while !q.is_empty() {
                let u = q.pop_front().unwrap();
                let ru = router_info.get_unsafe(u).as_ref().unwrap();
                for &v in &ru.links {
                    if nodes.contains(&v) && !visited.contains(&v) {
                        visited.insert(v);
                        q.push_back(v);
                        component.push(v);
                    }
                }
            }
            components.push(component);
        }

        return components;
    }

    pub fn build_border_graph(&mut self) {
        let start = Instant::now();
        let mut origin_border_nodes = HashSet::new();
        let mut origin_border_edge_cnt = 0;
        for u in &self.routers {
            let mut ru = self.router_info.remove_unsafe(*u).unwrap();
            ru.ebgp_cnt = 0;
            for v in &ru.links {
                let rv = get_router!(self, *v);
                let asu = ru.asn;
                let asv = rv.asn;
                if asu.is_none() || asv.is_none() {
                    continue;
                }
                let asu = asu.unwrap();
                let asv = asv.unwrap();

                if self.user {
                    // User mode will keep the links inside an AS
                    if !self.link_weight.contains_key(&pair_ord(ru.id, rv.id)) {
                        continue;
                    }
                } else {
                    if asu == asv {
                        // Assuming routers in an AS is connected by route reflector,
                        // we do not need to connect internal routers
                        continue;
                    }
                    if !self.is_switch(u) {
                        origin_border_nodes.insert(*u);
                    }
                    if !self.is_switch(v) {
                        origin_border_nodes.insert(*v);
                    }
                    origin_border_edge_cnt += 1;

                    if (!self.is_switch(u) && ru.latlng.is_none())
                        || (!self.is_switch(v) && rv.latlng.is_none())
                    {
                        continue;
                    }
                }

                ru.border = true;
                // u,v will be reversed in other iteration
                ru.border_links.push(*v);
                if *u == 103203391 || *v == 103203391 {
                    println!("{}:{}->{}:{}", u, asu, v, asv)
                }

                if asu != asv {
                    ru.ebgp_cnt += 1;
                }
            }
            // internal switch => ru.border=false
            if *u == 19977498 {
                println!(
                    "Router: 19977498; Geo = {:?}, IS Border = {:?}",
                    ru.latlng, ru.border
                );
            }
            if ru.border {
                self.borders.insert(*u);
                self.as_borders.get_mut_unsafe(ru.asn.unwrap()).push(*u);
            }
            self.router_info.insert_unsafe(*u, Some(ru));
        }
        // util::dedup_vecs(self.as_borders.values_mut());
        println!("Border router detected in {}s.", start.elapsed().as_secs());
        println!(
            "Original Borders: {}, Original Border links: {}",
            origin_border_nodes.len(),
            origin_border_edge_cnt
        );
        println!(
            "External links retained {}",
            self.borders
                .iter()
                .map(|u| get_router!(self, *u).border_links.len())
                .sum::<usize>()
                / 2
        );

        if !self.user {
            // In user mode, the route reflector is indicated by the user in reflectors.in
            let mut total_asn = 0;
            for (asu, borders) in &self.as_borders {
                let mut asvs = HashSet::new();
                if borders.len() == 0 {
                    continue;
                }
                total_asn += 1;
                for u in borders {
                    let ru = get_router!(self, *u);
                    for v in &ru.border_links {
                        let asv = get_router!(self, *v).asn.unwrap();
                        asvs.insert(asv);
                    }
                }
                if asvs.iter().all(|asv| {
                    *self.as_rel.get(&(*asv, asu)).unwrap_or(&AsRel::None)
                        == AsRel::ProviderCustomer
                }) {
                    self.stub_as.insert(asu);
                    if asvs.len() == 1 {
                        self.single_homed_stub_as.insert(asu);
                    }
                }
            }
            println!(
                "{} stub AS detected in {} ASes in {}s.",
                self.stub_as.len(),
                total_asn,
                start.elapsed().as_secs()
            );
            println!(
                "{} single-homed AS detected in {}s.",
                self.single_homed_stub_as.len(),
                start.elapsed().as_secs()
            );

            let mut reflector_range = self.switch_range;
            for (asn, borders) in &mut self.as_borders {
                if asn == IXP_ASN || asn == HUB_ASN || borders.len() <= 1 {
                    continue;
                }
                reflector_range += 1;
                let mut reflector = Router::new(reflector_range);
                reflector.asn = Some(asn);
                reflector.border = true;
                reflector.reflector = true;
                // We only consider reflector in the border graph, so do not edit Router.links and self.routers
                reflector.border_links = borders.clone();

                for u in &mut *borders {
                    let ru = get_router_mut!(self, *u);
                    ru.border_links.push(reflector.id);
                }

                self.reflectors.insert(reflector.id);
                self.borders.insert(reflector.id);
                borders.push(reflector.id);
                self.router_info.insert(reflector.id, Some(reflector));
            }
            println!("Built route reflectors in {}s.", start.elapsed().as_secs());
        } else {
            for (_asn, borders) in &self.as_borders {
                for border in borders {
                    Topology::dijkstra(&border, borders, &self.router_info, &mut self.link_weight);
                }
            }
        }

        self.border_as_list = self
            .as_borders
            .keys(|vs| vs.len() > 0)
            .into_iter()
            .collect();
        self.border_link_cnt = self
            .borders // Using BTreeMap may improve the speed because of cacheline
            .iter()
            .map(|u| get_router!(self, *u).border_links.len())
            .sum::<usize>()
            / 2;
        println!("Border graph built in {}s.", start.elapsed().as_secs());
    }

    pub fn condense_border_graph(&mut self, output: Option<PathBuf>) {
        let start = Instant::now();

        let mut n_routers_reduced = 0;
        let mut all_asn = self.as_borders.keys(|_| true);
        all_asn.sort_by(|a, b| {
            self.as_borders
                .get(*b)
                .unwrap()
                .len()
                .cmp(&self.as_borders.get(*a).unwrap().len())
        });

        let mut condensed_borders: Vec<u32> = Vec::new();
        let mut br_to_group: HashMap<u32, u32> = HashMap::new();

        let mut total = 0;
        for asn in all_asn.iter() {
            let borders = self.as_borders.get(*asn).unwrap();
            total += borders.len();
        }
        println!("Total borders: {}", total);
        for asn in all_asn.iter() {
            let mut location_rep_br: HashMap<(i16, i16), u32> = HashMap::new();
            let g = Geodesic::wgs84();
            let mut no_geo = 0;
            let mut has_geo = 0;
            let mut condensed_as_borders = HashSet::new();
            // sort by !is_switch(br)
            let mut borders = self.as_borders.get(*asn).unwrap().clone();
            borders.sort_by(|a, b| self.is_switch(a).cmp(&self.is_switch(b)).then(a.cmp(b)));
            for br in &borders {
                if let Some((lat_0, lng_0)) = get_router!(self, *br).latlng {
                    has_geo += 1;
                    let mut has_group = false;
                    let mut rep_coords = location_rep_br.keys().collect::<Vec<_>>();
                    rep_coords.sort_by(|a, b| {
                        location_rep_br
                            .get(*a)
                            .unwrap()
                            .cmp(location_rep_br.get(*b).unwrap())
                    });
                    for (lat_1, lng_1) in rep_coords {
                        let rep_br: u32 = *location_rep_br.get(&(*lat_1, *lng_1)).unwrap();
                        let (lat_0f, lng_0f) = decode_latlng((lat_0, lng_0));
                        let (lat_1f, lng_1f) = decode_latlng((*lat_1, *lng_1));
                        let dist_m: f64 =
                            g.inverse(lat_0f as f64, lng_0f as f64, lat_1f as f64, lng_1f as f64);
                        if dist_m < 50.0 * 1000.0 {
                            br_to_group.insert(*br, rep_br);
                            has_group = true;
                            break;
                        }
                    }
                    if !has_group {
                        condensed_as_borders.insert(*br);
                        br_to_group.insert(*br, *br);
                        location_rep_br.insert((lat_0, lng_0), *br);
                    }
                } else {
                    if *asn == 15169 {
                        println!("No geo: {}", br);
                    }
                    condensed_as_borders.insert(*br);
                    br_to_group.insert(*br, *br);
                    no_geo += 1;
                }
            }
            self.as_borders
                .get_mut_unsafe(*asn)
                .retain(|&x| condensed_as_borders.contains(&x));
            condensed_borders = condensed_borders
                .into_iter()
                .chain(condensed_as_borders.into_iter())
                .collect();
            if *asn == 15169 {
                println!("AS {} # of groups: {}", asn, location_rep_br.len());
                println!("no_geo: {}, has_geo: {}", no_geo, has_geo);
            }
            // println!("{}", br_to_group.len());
            n_routers_reduced += location_rep_br.len();
        }
        if let Some(output) = output {
            std::fs::create_dir_all(output.clone()).unwrap();
            let mut file = File::create(output.join("router_mapping.json")).unwrap();
            // write the mapping to a file for further analysis
            println!("Output router mapping, {} entries", br_to_group.len());
            for (br, rep_br) in br_to_group.iter() {
                writeln!(file, r#"{{"id": {},  "group_id":{}}}"#, br, rep_br).unwrap();
            }
        }
        println!("Total routers after: {}", n_routers_reduced);
        // condens border links (may include duplicates)
        let mut condensed_border_links: HashMap<u32, Vec<u32>> = HashMap::new();
        for u in &self.borders {
            let ru = get_router!(self, *u);
            let border_links = ru
                .border_links
                .iter()
                .filter_map(|v| br_to_group.get(v).copied())
                .collect::<Vec<u32>>();
            let rep_u = *br_to_group.get(u).unwrap();
            condensed_border_links
                .entry(rep_u)
                .or_insert_with(Vec::new)
                .extend(border_links);
        }
        self.borders = condensed_borders.into_iter().collect();
        for u in &self.borders {
            let ru = get_router_mut!(self, *u);
            let condensed_links = condensed_border_links.get(u).unwrap();
            ru.border_links = condensed_links
                .iter()
                .copied()
                .filter(|&v| v != *u) // remove self loop
                .collect::<HashSet<u32>>() // dedup
                .into_iter()
                .collect();
        }

        self.border_link_cnt = self
            .borders // Using BTreeMap may improve the speed because of cacheline
            .iter()
            .map(|u| get_router!(self, *u).border_links.len())
            .sum::<usize>()
            / 2;
        // println!("condense border links {} -> {} ", N, M);
        println!("condense border graph in {}s.", start.elapsed().as_secs());
    }

    pub fn bgp_neighbors(&self, ru: &Router) -> Vec<RouterId> {
        let u = ru.id;
        let mut vs = BTreeSet::new(); // Use BTreeSet to ensure non-duplicate and keep the iteration order
        for &v in &ru.border_links {
            let rv = get_router!(self, v);
            if self.is_switch(&v) {
                for &w in &rv.border_links {
                    if w != u {
                        // Ensure that w is not a switch
                        if self.is_switch(&w) {
                            panic!("w is switch {}", w);
                        }
                        let rw = get_router!(self, w);
                        if self.get_weight(&ru, &rw).is_some() {
                            if ru.asn != rw.asn {
                                vs.insert(w);
                            }
                        }
                    }
                }
            } else {
                vs.insert(v);
            }
        }
        let vs: Vec<_> = vs.into_iter().collect();
        vs
    }

    // Gives all ASes within `depth` hops from any as in `src_as`
    // If depth=0, return {src_as}
    // If depth=1, return {src_as} and all its neighbors
    pub fn bfs(&self, src_as: Vec<AsNumber>, depth: Option<u32>) -> HashSet<AsNumber> {
        let mut visited = HashSet::new();
        let mut q = VecDeque::new();
        for asn in src_as {
            visited.insert(asn);
            q.push_back((asn, 0, Some(AsRel::CustomerProvider)));
        }
        while !q.is_empty() {
            let (asu, d, rel_opt) = q.pop_front().unwrap();

            if depth.map_or(true, |depth| d >= depth) {
                continue;
            }
            let mut asvs = HashSet::new();
            for br in self.as_borders.get_unsafe(asu) {
                let ru = get_router!(self, *br);
                for v in self.bgp_neighbors(ru) {
                    let asv = get_router!(self, v).asn.unwrap();
                    asvs.insert(asv);
                }
            }

            for asv in &asvs {
                if !visited.contains(asv) {
                    let rel = self.as_rel.get(&(asu, *asv));
                    let allowed = match rel_opt {
                        Some(AsRel::ProviderCustomer) => {
                            matches!(rel, Some(AsRel::ProviderCustomer))
                        }
                        Some(AsRel::CustomerProvider) => {
                            matches!(rel, Some(AsRel::PeerPeer) | Some(AsRel::ProviderCustomer))
                        }
                        Some(AsRel::PeerPeer) => matches!(rel, Some(AsRel::ProviderCustomer)),
                        _ => false,
                    };
                    if allowed {
                        visited.insert(*asv);
                        q.push_back((*asv, d + 1, rel.cloned()));
                    }
                }
            }
            // for v in self.as_rel.neighbors(u) {
            //     if !visited.contains(&v) {
            //         visited.insert(v);
            //         q.push_back((v, d + 1));
            //     }
            // }
        }
        return visited;
    }
    pub fn print(&self) {
        println!(
            "routers: {}, AS: {}, IXP: {}, router links: {}",
            self.routers.len(),
            self.router_as_list.len(),
            self.ixp.len(),
            self.router_link_cnt
        );

        println!(
            "borders: {}, AS: {}, switches:{}, reflectors: {}, border links: {}",
            self.borders.len(),
            self.border_as_list.len(),
            self.switch_range - MAX_NID,
            self.reflectors.len(),
            self.border_link_cnt,
        );
    }

    pub fn print_ixp(&self, filename: &str) -> std::io::Result<()> {
        let mut file = File::create(filename)?;
        for ixp in &self.ixp {
            let rixp = get_router!(self, *ixp);
            file.write_fmt(format_args!("IXP {}: {}\n", ixp, rixp.links.len()))?;

            for u in &rixp.links {
                let router = get_router!(self, *u);
                if let Some(asn) = router.asn {
                    file.write_fmt(format_args!("{}-{}: ", router.id, asn))?;
                } else {
                    file.write_fmt(format_args!("{}-0: ", router.id))?;
                }
                for ip in &router.ifaces {
                    file.write_fmt(format_args!("{}, ", ip.to_string()))?;
                }
                file.write_all(b"\n")?;
            }
            file.write_all(b"\n")?;
        }
        Ok(())
    }

    pub fn print_stub(&self, filename: &str) -> std::io::Result<()> {
        let mut file = File::create(filename)?;
        file.write_fmt(format_args!("Stub networks {}\n", self.stub_as.len()))?;
        for &asn in &self.stub_as {
            file.write_fmt(format_args!(
                "{}: {}\n",
                asn,
                self.as_borders.get_unsafe(asn).len()
            ))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_examples() -> io::Result<()> {
        let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../input");
        for (case, routers, ases) in [("eg1", 6, 3), ("eg2", 9, 5), ("eg8", 10, 4)] {
            let topo = Topology::read_user_from_path(&input.join(case))?;
            assert_eq!(topo.routers.len(), routers, "{case}");
            assert_eq!(topo.borders.len(), routers, "{case}");
            assert_eq!(topo.border_as_list.len(), ases, "{case}");
            assert_eq!(topo.reflectors.len(), usize::from(case == "eg8"));
            // Intra-AS costs must be available even for nonadjacent routers.
            if case == "eg1" {
                assert_eq!(
                    topo.get_weight(get_router!(topo, 2), get_router!(topo, 5)),
                    Some(7)
                );
            }
            if case == "eg8" {
                assert!(topo.is_reflector(get_router!(topo, 7)));
                assert_eq!(
                    topo.get_weight(get_router!(topo, 2), get_router!(topo, 9)),
                    Some(2)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn test_as_rel() -> std::io::Result<()> {
        let mut topo_a = Topology::new(false);
        let input = Path::new("../../input/problink");
        topo_a.read_as_rel(input.join("20210701.txt"))?;

        let path = vec![265619, 1299, 19527, 15169];
        // let path = vec![202895, 6939, 15412, 19527, 15169];
        // let path = vec![44417, 3216, 9198, 50482, 15169];
        for i in 0..path.len() - 1 {
            let a = path[i];
            let b = path[i + 1];
            let rel = topo_a.as_rel.get(&(a, b));
            println!("{} -> {}: {:?}", a, b, rel);
            // assert!(matches!(rel, AsRel::ProviderCustomer | AsRel::PeerPeer));
        }

        let neighbors = vec![
            (63949, 20940),
            (50324, 8881),
            (23889, 37100),
            (42473, 6762),
            (207143, 47692),
            (19529, 395348),
            (8888, 7578),
            (11426, 7843),
            (26283, 3257),
        ];

        let mut topo_b = Topology::new(false);
        let input = Path::new("../../input/caida-0824");
        topo_b.read_as_rel(input.join("20250801.as-rel2.txt"))?;

        for (u, v) in neighbors {
            let rel = topo_a.as_rel.get(&(u, v));
            println!("(a){} -> {}: {:?}", u, v, rel);
            let rel = topo_b.as_rel.get(&(u, v));
            println!("(b){} -> {}: {:?}", u, v, rel);
            // assert!(matches!(
            //     rel,
            //     Some(AsRel::PeerPeer) | Some(AsRel::ProviderCustomer)
            // ));
        }

        let n_a = topo_a
            .as_rel
            .iter()
            .filter_map(|(a, b)| if a.0 < a.1 { Some((a, b)) } else { None })
            .into_iter()
            .count();

        let n_b = topo_b
            .as_rel
            .iter()
            .filter_map(|(a, b)| if a.0 < a.1 { Some((a, b)) } else { None })
            .into_iter()
            .count();

        println!("AS relationship in topo_a: {}", n_a);
        println!("AS relationship in topo_b: {}", n_b);
        Ok(())
    }
}
