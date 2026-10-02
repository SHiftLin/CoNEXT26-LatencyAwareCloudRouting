use geoutils;
use std::fs::File;
use std::io;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

pub(crate) struct CAIDAReader {
    lines: io::Lines<io::BufReader<File>>,
}

impl CAIDAReader {
    pub(crate) fn new<P>(filename: P) -> io::Result<Self>
    where
        P: AsRef<Path>,
    {
        let file = File::open(filename)?;
        Ok(CAIDAReader {
            lines: BufReader::new(file).lines(),
        })
    }
}

impl Iterator for CAIDAReader {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(line) = self.lines.next() {
                if line.is_ok() {
                    let line = line.unwrap();
                    if let Some((content, _comment)) = line.split_once('#') {
                        if content.len() == 0 {
                            continue;
                        }
                        return Some(content.to_owned());
                    } else {
                        return Some(line);
                    }
                }
            } else {
                return None;
            }
        }
    }
}

pub struct Reader {
    pub lines: io::Lines<io::BufReader<File>>,
}

impl Reader {
    pub fn new<P>(filename: P) -> io::Result<Self>
    where
        P: AsRef<Path>,
    {
        let file = File::open(filename)?;
        Ok(Reader {
            lines: BufReader::new(file).lines(),
        })
    }
}

impl Iterator for Reader {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(line) = self.lines.next() {
                if line.is_ok() {
                    return Some(line.unwrap());
                }
            } else {
                return None;
            }
        }
    }
}

pub struct Writer {
    file: File,
    buffer: Vec<String>,
}

impl Writer {
    pub fn new<P>(filename: P) -> io::Result<Self>
    where
        P: AsRef<Path>,
    {
        Ok(Writer {
            file: File::create(filename)?,
            buffer: Vec::new(),
        })
    }

    pub fn flush_to_kernel(&mut self) -> io::Result<()> {
        self.buffer.push("".to_owned());
        self.file.write_all(self.buffer.join("\n").as_bytes())?;
        self.buffer.clear();
        Ok(())
    }

    pub fn write_line(&mut self, s: String) -> io::Result<()> {
        self.buffer.push(s);
        if self.buffer.len() >= 100 {
            self.flush_to_kernel()?;
        }
        Ok(())
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.flush_to_kernel().expect("Write error in writer drop");
    }
}

pub fn dedup_vecs<'a, M, V>(vecs: M)
where
    M: IntoIterator<Item = &'a mut Vec<V>>,
    V: 'a + Ord,
{
    for v in vecs {
        v.sort_unstable();
        v.dedup();
    }
}

pub type LatLng = (f32, f32);
pub type LatLngInt = (i16, i16);

pub fn encode_latlng(latlng: LatLng) -> LatLngInt {
    ((latlng.0 * 100.0) as i16, (latlng.1 * 100.0) as i16)
}

pub fn decode_latlng(latlng: LatLngInt) -> LatLng {
    (latlng.0 as f32 / 100.0, latlng.1 as f32 / 100.0)
}

pub fn pair_ord<T: PartialOrd>(a: T, b: T) -> (T, T) {
    if a > b {
        return (b, a);
    }
    return (a, b);
}

pub fn geo_distance(latlng1: LatLng, latlng2: LatLng) -> f64 {
    let a = geoutils::Location::new(latlng1.0, latlng1.1);
    let b = geoutils::Location::new(latlng2.0, latlng2.1);
    match b.distance_to(&a) {
        Ok(d) => d.meters(),
        Err(_) => b.haversine_distance_to(&a).meters(),
    }
}

// one-way <-> one-way
// 100m one-way <-> 0.5us one-way
pub fn distance_to_latency(dist: f64) -> u32 {
    return (dist / 200000000.0 * 1000000.0) as u32; // meter to us
}

pub fn latency_to_distance(lat: u32) -> u64 {
    return lat as u64 * 200000000 / 1000000; // us to meter
}

#[cfg(test)]
mod tests {
    use super::{decode_latlng, distance_to_latency, encode_latlng, geo_distance};

    #[test]
    fn geo_location() {
        let a1 = (36.001465, -78.939133);
        let b1 = (31.2974, 121.5036);
        let a2 = decode_latlng(encode_latlng(a1));
        let b2 = decode_latlng(encode_latlng(b1));
        let dist1 = geo_distance(a1, b1);
        let dist2 = geo_distance(a2, b2);
        println!("{} {} {}", dist1, dist2, distance_to_latency(dist1));
        assert!((dist1 - dist2).abs() <= 3000.0);

        let o = (27.71722, 109.18528); // id: 17226
        let n = (36.6268, 101.7548); // id: 59356578
        let w = (59.3247, 18.056); // id: 634148
        let v = (59.3247, 18.056); // id: 76491
        let ro = decode_latlng(encode_latlng(o));
        let rn = decode_latlng(encode_latlng(n));
        let rw = decode_latlng(encode_latlng(w));
        let rv = decode_latlng(encode_latlng(v));
        let dist1 = distance_to_latency(geo_distance(o, v));
        let dist2 = distance_to_latency(geo_distance(o, n))
            + distance_to_latency(geo_distance(n, w))
            + distance_to_latency(geo_distance(w, v));
        let dist3 = distance_to_latency(geo_distance(ro, rv));
        let dist4 = distance_to_latency(geo_distance(ro, rn))
            + distance_to_latency(geo_distance(rn, rw))
            + distance_to_latency(geo_distance(rw, rv));

        println!("{} {} {} {}", dist1, dist2, dist3, dist4);
        assert!(dist1 < dist2);
        assert!(dist3 < dist4);
    }
}
