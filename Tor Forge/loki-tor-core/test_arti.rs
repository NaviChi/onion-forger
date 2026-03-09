use arti_client::TorClientConfig;
use arti_client::config::dir::CfgPath;
fn main() {
    let mut builder = TorClientConfig::builder();
    builder.storage()
        .cache_dir(CfgPath::new(".loki_tor_state/cache".into()))
        .state_dir(CfgPath::new(".loki_tor_state/state".into()));
    let _config = builder.build().unwrap();
}
