use std::fmt::format;

use serde::Serialize;
use toml::Table;

#[derive(Serialize)]
struct SimConf {
    mode: String,
    no_pref: String,
    metric: String,
    quant: u32,
}
#[derive(Serialize)]
struct Conf {
    configs: Vec<SimConf>,
}

fn run_shell(command: &str) -> std::io::Result<()> {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .status()?;
    Ok(())
}
fn main() {
    // let all_region = vec![
    //     "asia-south1",
    //     "europe-west3",
    //     "us-east4",
    //     "africa-south1",
    //     "australia-southeast1",
    //     "southamerica-east1",
    //     "me-central1",
    //     "us-west1",
    // ];
    // let cloud = "google";

    let all_region = vec![
        "centralindia",
        // "germanywestcentral",
        // "eastus",
        // "southafricanorth",
        // "australiaeast",
        // "brazilsouth",
        // "qatarcentral",
        // "westus",
    ];
    let cloud = "azure";

    // let all_region = vec![
    //     "ap-south-1",
    //     "eu-central-1",
    //     "us-east-1",
    //     "af-south-1",
    //     "ap-southeast-2",
    //     "sa-east-1",
    //     "me-central-1",
    //     "us-west-1",
    // ];
    // let cloud = "aws";

    for region in &all_region {
        let template_path = format!("examples/{}/exp_fmt_{}.sh", cloud, region);
        let ext = format!("ext-{}-{}", region, 5);
        let mut conf = Conf {
            configs: vec![
                // SimConf {
                // mode: "path".to_string(),
                // no_pref: "./input/as_scope/no.txt".to_string(),
                // metric: format!("./input/as_scope/neighbor-{}-{ext}", region),
                // quant: 5,
            // }
            ],
        };
        for n in 9..=9 {
            conf.configs.push(SimConf {
                mode: "path".to_string(),
                no_pref: "./input/as_scope/no.txt".to_string(),
                metric: format!(
                    "./input/as_scope/as-scope-30-0-0.txt-{}-{}-{}-{}-T{}",
                    ext,
                    ext,
                    ext,
                    ext,
                    n * 100
                ),
                quant: 5,
            });
        }
        // quantization factors
        // let no_pref = format!("./input/as_scope/probes-{}.txt", region);
        // let q = [1, 2, 5, 7, 10];
        // let mut conf = Conf {
        //     configs: q
        //         .iter()
        //         .map(|&quant| SimConf {
        //             mode: "path".to_string(),
        //             metric: "all".to_string(),
        //             // no_pref: "./input/as_scope/no.txt".to_string(),
        //             no_pref: no_pref.clone(),
        //             quant: quant,
        //         })
        //         .collect(),
        // };

        // partial aigp
        // let n = [60, 70, 80, 90, 100];
        // let conf = Conf {
        //     configs: n
        //         .iter()
        //         .map(|&num| SimConf {
        //             mode: "aigp".to_string(),
        //             metric: format!("./input/as_scope/aigp-{}-top{}.txt", region, num),
        //             no_pref: "./input/as_scope/no.txt".to_string(),
        //             quant: 10,
        //         })
        //         .collect(),
        // };

        // let mut conf = Conf { configs: vec![] };
        // full aigp
        // conf.configs.push(SimConf {
        //     mode: "aigp".to_string(),
        //     no_pref: "all".to_string(),
        //     metric: "all".to_string(),
        //     quant: 10,
        // });

        // write config to sim-conf.toml
        let conf_toml = toml::to_string(&conf).unwrap();
        let conf_path = format!("input/experiments/sim-config-{}.toml", region);
        println!("write conf to {}", conf_path);

        let file = std::fs::File::create(&conf_path).unwrap();
        std::io::Write::write_all(&mut std::io::BufWriter::new(file), conf_toml.as_bytes())
            .unwrap();
        println!("wrote conf {}", conf_toml);
        let template = std::fs::read_to_string(&template_path).unwrap();
        let cmd = template;
        let cmd = cmd.replace("input/experiments/sim-config.toml", &conf_path);
        println!("run: {}", region);
        let cmd = if let Some(index) = cmd.find("./target/release/compare") {
            cmd[index..].to_string()
        } else {
            cmd
        };
        println!("exec: {}", cmd);

        run_shell(&cmd).unwrap();
        // conf.configs[0].metric = format!("{}-{}", conf.configs[0].metric, ext);
    }
}
