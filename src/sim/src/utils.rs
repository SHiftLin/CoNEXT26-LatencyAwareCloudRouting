use std::{collections::HashMap, fmt::Debug};

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

pub fn quantize_as_path_len(latency: u32, quantizer_us: u32) -> u32 {
    ((latency + 1) as f64 / quantizer_us as f64).ceil() as u32
}

pub fn quantize_aigp(latency: u32) -> u32 {
    // return latency / 2500 + 1;
    return latency;
}

pub fn print_top_count<K>(tag: &str, cnt_map: &HashMap<K, usize>)
where
    K: Clone + Debug,
{
    let mut kvs: Vec<(K, usize)> = cnt_map.clone().into_iter().collect();
    kvs.sort_by(|a, b| b.1.cmp(&a.1));
    let mut i = 0;
    print!("{} ", tag);
    for (k, v) in &kvs {
        print!("({:?}: {}), ", k, v);
        i += 1;
        if i >= 10 {
            break;
        }
    }
    println!();
}
