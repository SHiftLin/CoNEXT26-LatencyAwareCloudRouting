use postgres::{Client, Error, NoTls};
use std::io;

pub struct PostgresClient {
    client: Client,
}
impl PostgresClient {
    pub fn new() -> Result<PostgresClient, Box<dyn std::error::Error + Send + Sync>> {
        let connection = std::env::var("BGP_SIM_DATABASE_URL").map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "set BGP_SIM_DATABASE_URL to a PostgreSQL connection URL or parameter string",
            )
        })?;
        let client = PostgresClient {
            client: Client::connect(&connection, NoTls)?,
        };
        Ok(client)
    }
    pub fn query(
        &mut self,
        table: &str,
        cols: &str,
        cond: &str,
    ) -> Result<Vec<postgres::Row>, Error> {
        let query = format!("SELECT {} FROM {} WHERE {}", cols, table, cond);
        let rows = self.client.query(&query, &[])?;
        Ok(rows)
    }
    pub fn update(
        &mut self,
        table: &str,
        col: &str,
        value: &str,
        wherekey: &str,
        wherevalue: &str,
    ) -> Result<u64, Error> {
        let query = format!(
            "UPDATE {} SET {} = '{}' WHERE {}={}",
            table, col, value, wherekey, wherevalue
        );
        let rows_updated = self.client.execute(&query, &[])?;
        Ok(rows_updated)
    }
    pub fn insert(&mut self, table: &str, cols: &str, values: &str) -> Result<u64, Error> {
        let query = format!(
            "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT DO NOTHING",
            table, cols, values
        );
        let rows_inserted = self.client.execute(&query, &[])?;
        Ok(rows_inserted)
    }
}

#[cfg(test)]
mod test {
    use std::{
        collections::{HashMap, HashSet},
        io::{self, BufRead, Read, Write},
        net::{IpAddr, Ipv4Addr},
    };

    use super::*;
    #[test]
    fn test_query() {
        let client = PostgresClient::new();
        assert!(client.is_ok(), "Failed to create Postgres client");
        let mut client = client.unwrap();
        let rows = client.query("border_geo_202506", "ip", "method='ipinfo'");
        assert!(rows.is_ok(), "Failed to execute query");
        let rows = rows.unwrap();
        assert!(rows.len() == 113153, "Expected none empty results");
        println!("{:?}", rows[0].get::<usize, String>(0));
    }
    #[test]
    fn test_query_probe() {
        let mut client = PostgresClient::new().unwrap();
        let rows = client
            .query("selected_probes_20250713", "prb_id", "true")
            .unwrap();
        let probe_str = rows
            .iter()
            .map(|row| row.get::<usize, i32>(0).to_string())
            .collect::<Vec<String>>()
            .join(",");
        let file = std::fs::File::create("../../input/ripe/selected_probes_20250713.txt").unwrap();
        println!("{}", probe_str);
        let mut writer = std::io::BufWriter::new(file);
        writer
            .write(probe_str.as_bytes())
            .expect("Failed to write to file");
        // let rows = client
        //     .query(
        //         "probe20250713",
        //         "prb_id, city, country, asn_v4",
        //         "asn_v4 IS NOT NULL",
        //     )
        //     .unwrap();
        // let mut selected_probes = HashMap::new();
        // for row in &rows {
        //     let city: Option<String> = row.get("city");
        //     let country: Option<String> = row.get("country");
        //     let city = city.unwrap_or_else(|| "null".to_string());
        //     let country = country.unwrap_or_else(|| "null".to_string());
        //     let asn: i32 = row.get("asn_v4");
        //     let city_country = format!("{},{}", city, country);
        //     if selected_probes.contains_key(&(city_country.clone(), asn)) {
        //         continue;
        //     }
        //     let prb_id: i32 = row.get(0);
        //     selected_probes.insert((city_country, asn), prb_id);
        // }
        // println!("{}", selected_probes.len());
        // for (key, prb_id) in &selected_probes {
        //     println!("{}: {}", key.0, prb_id);
        // }
        // let values = selected_probes.values().cloned().collect::<Vec<i32>>();
        // for id in values {
        //     let _ = client.insert("selected_probes_20250713", "prb_id", &id.to_string());
        // }
        println!("{:?}", rows[0]);
    }
    #[test]
    fn test_update() -> io::Result<()> {
        let node_as_file = "/nfs/lsh/bdrmapit/node.as";
        let file = std::fs::File::open(node_as_file).unwrap();
        let reader = std::io::BufReader::new(file);
        let ip_to_as = reader
            .lines()
            .filter_map(|line| {
                if let Ok(line) = line {
                    let parts = line.split_whitespace().collect::<Vec<&str>>();
                    if parts.len() >= 2 {
                        if let Ok(ip) = parts[1].parse::<IpAddr>() {
                            if parts[2] != "-1" {
                                if let Ok(asn) = parts[2].parse::<u32>() {
                                    return Some((ip, (asn, parts[3].to_string())));
                                }
                            }
                        }
                    }
                }
                None
            })
            .collect::<HashMap<IpAddr, (u32, String)>>();
        let table = "ip_geo_202509";
        let col_asn = "asn_bdrmapit";
        let col_method = "method_bdrmapit";
        let wherekey = "ip";
        let mut client = PostgresClient::new().unwrap();

        let rows: Result<Vec<postgres::Row>, Error> =
            client.query(table, "ip", "asn_bdrmapit is  null");
        assert!(rows.is_ok(), "Failed to execute query of existing info");
        let rows = rows.unwrap();
        let mut no_asn_ip = rows
            .iter()
            .map(|row| row.get::<usize, IpAddr>(0))
            .collect::<HashSet<IpAddr>>();
        println!("Existing IPs: {}", no_asn_ip.len());
        println!("{}", no_asn_ip.iter().next().unwrap());
        let mut n = 0;

        let ip_file = "/nfs/lsh/BGPSimulator/temp/ip.txt";
        let file = std::fs::File::open(ip_file).unwrap();
        let reader = std::io::BufReader::new(file);
        let ip_vec = reader.lines().collect::<Result<Vec<String>, _>>()?;
        println!("Total IPs in ip file: {}", ip_vec.len());
        let mut count_0 = 0;
        for (i, line) in ip_vec.iter().enumerate() {
            if i % 1000 == 0 {
                println!("Processing line {}, no asn {}", i, count_0);
            }

            // if !no_asn_ip.contains(&parts[1].parse::<IpAddr>().unwrap()) {
            //     continue; // Skip if IP already exists in the database
            // }
            n += 1;
            let ip = line.parse::<IpAddr>().unwrap();
            if ip_to_as.get(&ip).is_none() {
                count_0 += 1;
                // println!("IP not found in bdrmapit: {}", ip);
                continue;
            }
            let (asn, method) = ip_to_as.get(&ip).unwrap();
            // if asn != &16509 {
            //     continue;
            // }
            // print!("IP: {}({}), ASN: {}, Method: {}", ip, line, asn, method);
            let asn = asn.to_string();

            let ip = format!("'{}'", ip);
            let _rows_updated = client.update(table, col_asn, &asn, wherekey, &ip);
            let _rows_updated = client.update(table, col_method, method, wherekey, &ip);

            // break;
        }
        println!("Total new IPs annotated: {}", n);
        Ok(())
    }
}
