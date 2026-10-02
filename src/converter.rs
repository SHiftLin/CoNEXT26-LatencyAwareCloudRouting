use std::time::Instant;

use topo::Converter;
use topo::Topology;

const TPO_DIR: &str = "./input/caida-0325";
// const TPO_DIR: &str = "../ark/ipv4/itdk/ITDK-2022-02";
// const AS_DIR: &str = "../ark/as-relationships";

fn main() -> std::io::Result<()> {
    let start = Instant::now();

    let mut cvert = Converter::new();
    cvert.read_routers_ifaces_raw(
        &(TPO_DIR.to_owned() + "/midar-iff.nodes"),
        &(TPO_DIR.to_owned() + "/nodes.in"),
        true,
    )?;
    println!("{}s", start.elapsed().as_secs());
    cvert.read_links_raw(
        &(TPO_DIR.to_owned() + "/midar-iff.links"),
        &(TPO_DIR.to_owned() + "/links.in"),
        true,
    )?;
    println!("{}s", start.elapsed().as_secs());
    cvert.read_as_raw(
        &(TPO_DIR.to_owned() + "/midar-iff.nodes.as"),
        &(TPO_DIR.to_owned() + "/nodes.as.in"),
        true,
    )?;
    println!("{}s", start.elapsed().as_secs());
    cvert.read_geo_raw(
        &(TPO_DIR.to_owned() + "/midar-iff.nodes.geo"),
        &(TPO_DIR.to_owned() + "/nodes.geo.in"),
        true,
    )?;

    println!("Convert finished at {}s", start.elapsed().as_secs());

    // Test reading time for the new files
    let start = Instant::now();

    let seed = 42;
    let mut topo = Topology::new(false);
    println!("Init takes {}s.", start.elapsed().as_secs());
    topo.read_routers_ifaces(TPO_DIR.to_owned() + "/midar-iff.nodes.in")?;
    println!("{}s", start.elapsed().as_secs());
    topo.read_links(TPO_DIR.to_owned() + "/midar-iff.links.in")?;
    println!("{}s", start.elapsed().as_secs());
    topo.read_as(TPO_DIR.to_owned() + "/midar-iff.nodes.as.in")?;
    println!("{}s", start.elapsed().as_secs());
    topo.read_geo(TPO_DIR.to_owned() + "/midar-iff.nodes.geo.in")?;

    println!("Reading finished at {}s", start.elapsed().as_secs());

    Ok(())
}
