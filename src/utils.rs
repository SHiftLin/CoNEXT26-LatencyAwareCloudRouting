use std::fs::File;
use std::io;
use std::io::BufReader;
use std::path::Path;

pub fn read_json_file<P>(filename: P) -> io::Result<serde_json::Value>
where
    P: AsRef<Path>,
{
    let file = File::open(filename)?;
    let reader = BufReader::new(file);
    let json: serde_json::Value = serde_json::from_reader(reader)?;
    Ok(json)
}
