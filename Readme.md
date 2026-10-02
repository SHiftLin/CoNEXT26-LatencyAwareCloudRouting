# BGPSimulator

## Simulator core

### Build and run

```sh
cargo build --locked --bin simulator
mkdir -p output/readme-example
touch output/readme-example/no-pref-none.txt
./target/debug/simulator \
  --user --input input/eg1 --asn 1 --mode vanilla --rounds 0 \
  --no-pref-scope-path output/readme-example/no-pref-none.txt \
  --tag readme --output output/readme-example
```

The scope file above must be empty: it enables relationship preference everywhere by specifying that no AS disables it. Use a fresh file if that path already contains data. To disable preference everywhere, use `--no-pref-scope-path all`, which is also the CLI default.

The example writes `rib_vanilla_readme_1.txt` and `latency_vanilla_readme_1.txt` under `output/readme-example/eg1/`. The program appends the input directory's final name to `--output`. Use a new output directory or tag to preserve earlier runs.

Use `./target/debug/simulator --help` for the full option list. For larger experiments, build with `cargo build --release --locked --bin simulator` and use `target/release/simulator`.

### User-defined topology inputs

With `--user`, place the following files in the directory passed to `--input`:

| File | Format | Meaning |
| --- | --- | --- |
| `as.in` | `router_id ASN` | Required; one AS assignment per router. |
| `links.in` | `router_a router_b weight` | Required; undirected links with nonnegative integer costs. Same-AS links provide iBGP connectivity; cross-AS links provide eBGP connectivity. |
| `as-rel.in` | `ASN1\|ASN2\|relationship` | Required; `-1` means ASN1 is ASN2's provider; `0` means peers. Both directions are installed automatically. |
| `reflectors.in` | `router_id` | Optional; one existing router ID per line, identifying a synthetic reflector. |

### Origin selection

`--asn` selects the origin AS. The current CLI uses the fixed destination `1.1.1.1`; without `--locs`, it originates at the highest-ID eligible router in that AS.

### Outputs

| Output | Interpretation |
| --- | --- |
| `rib_<tag>.txt` | `router: next_hop, ingress_router, AS_path_length`; one selected route per represented non-reflector router. |
| `latency_<tag>.txt` | `router cost`; accumulated cost along the resulting selected forwarding route. |
| Standard output | Origin, topology statistics, rounds, and logical update counts. Redirect to a log when recording experiments. |
| `router_mapping.json` | Dataset mode only: original-to-condensed router IDs, stored as one JSON object per line. |

## Data sources

The measurement pipeline prepares router/interface mappings, AS assignments, coordinates, links, and relationship policies for the core. Traceroutes additionally support topology augmentation and comparison with observed paths; raw measurements are not required to rerun a simulation once its processed topology is available.

### Base topology

[CAIDA ITDK](https://www.caida.org/catalog/datasets/internet-topology-data-kit/) supplies router/interface associations, inferred links, node-to-AS assignments, and geographic information. The converter currently targets the `midar-iff.nodes`, `midar-iff.links`, `midar-iff.nodes.as`, and `midar-iff.nodes.geo` raw files. Match the converter to the chosen release's actual format.

### AS relationships

[CAIDA AS Relationships](https://www.caida.org/catalog/datasets/as-relationships/) supplies inferred provider/customer and peer relationships used for preference and export filtering. These are modeled policies, not recovered configurations of individual routers.

### Inbound traceroutes

RIPE Atlas measurements are identified by measurement ID. Preserve the result JSON, probe metadata, target address/ASN, and collection interval so the paths and source locations can be reconstructed. The processing tools use local result files, commonly `input/ripe/msm_results/<measurement_id>.json`, plus a probe metadata snapshot.

### AS ownership mapping (bdrmapIT)

[bdrmapIT](https://www.caida.org/catalog/papers/2018_pushing_boundaries_bdrmapit/pushing_boundaries_bdrmapit.pdf) is used  to annotate router/interface AS ownership when processing measured paths. The current Scamper processing tool consumes a local `node.as` export; it does not run bdrmapIT itself. That reader expects four tab-separated columns, taking the IP address from column 2 and ASN from column 3.

### IP geolocation

[IPinfo](https://ipinfo.io/data/ip-geolocation) provides commercial IP geolocation data for measurement-hop annotation. The processing code reads prepared PostgreSQL tables; it does not download the commercial database automatically. Obtain the data under its applicable access terms.

### Traceroute measurements (Scamper)

[Scamper](https://www.caida.org/catalog/software/scamper/) provides active traceroute measurements. The corresponding processor reads JSON-lines traces containing hop addresses, probe TTLs, and RTTs; keep the measurement parameters and any conversion from raw measurement files alongside the results.

## Data processing

### Small conversion example

For illustration, raw ITDK-style records such as:

```text
node N1: 192.0.2.1 192.0.2.2
node.AS N1 64500 example-method
link L1: N1:192.0.2.1 N2:198.51.100.1
```

produce these normalized records in separate files:

| File | Example record |
| --- | --- |
| `nodes.in` | `1: 192.0.2.1 192.0.2.2` |
| `nodes.as.in` | `1 64500` |
| `nodes.geo.in` | `1 40.71 -74.01` (illustrative coordinates from the separate geolocation input) |
| `links.in` | `1 2` |

The converter removes the `N` prefix from node IDs, extracts AS/coordinate fields, and converts multi-node links into connections through synthetic hub nodes. This is a format illustration, not a complete topology. For a runnable small topology, use the weighted `--user` files in `input/eg1` as shown above.

### Dataset-mode input files

Without `--user`, the loader currently expects these exact filenames under `--input`:

| Filename | Record format |
| --- | --- |
| `itdk-run-20240828.addrs` | IPv4 address in the first whitespace-separated field; used for router identification. |
| `midar-iff.nodes.in` | `router_id: IPv4_address [IPv4_address ...]` |
| `midar-iff.nodes.as.in` | `router_id ASN` |
| `midar-iff.nodes.geo.in` | `router_id latitude longitude` |
| `midar-iff.links.in` | `router_a router_b` |
| `20240801.as-rel2.txt` | `ASN1\|ASN2\|relationship` |

These names are currently fixed in `Topology::read_from_path`.

### Additional topology inputs

An optional `--extra-input-opt <directory>` adds `links.in`, `nodes.as.in`, and `nodes.geo.in`; `nodes.in` and `as-rel.txt` are optional. Extra `nodes.in` records use `router_id IPv4_address [IPv4_address ...]` (a space, not the base file's colon). Extra router IDs are offset when loaded, so use consistent local IDs across all files in each addition.
