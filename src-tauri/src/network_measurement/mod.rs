// This crate really isn't meant to be stable.

mod distance;
pub mod error;
mod speedtest;
mod speedtest_config;
mod speedtest_servers_config;

pub use speedtest_config::Proxy;


// Measures download and upload speed
fn measure(proxy: Option<&reqwest::Proxy>) -> Result<speedtest::SpeedTestResultOwned,error::SpeedTestError>{
    // Build proxy if exist
    let proxy= {
        match proxy{
            Some(reqwest_proxy) => {
                Proxy(Some(reqwest_proxy.to_owned()))
            },
            // Set proxy to nothing
            None => Proxy::default()
        }
    };
    // Measurement configs
    let mut config = speedtest::get_configuration(&proxy)?;
    // Custom configs
    config.sizes.download.truncate(4);
    config.sizes.upload.truncate(4);
    config.counts.download = 2;
    config.counts.upload = 2;

    // Get test server list
    let server_list = speedtest::get_server_list_with_config(&config.clone())?;
    // Sort servers by distance
    let mut server_list_sorted = server_list.servers_sorted_by_distance(&config.clone());
    // Remove farthest servers
    server_list_sorted.truncate(5);

    // Detect best server and do a latency test with that server
    let config_clone = config.clone();
    let best_server_info = speedtest::get_best_server_based_on_latency(&server_list_sorted[..],&config_clone)?;

    // Best test server
    let best_server = best_server_info.server.to_owned();
    
    // Download measurement
    let download_measurement = Some(speedtest::test_download_with_progress_and_config(&best_server, || {}, &mut config)?);
    // Upload measurement
    let upload_measurement = Some(speedtest::test_upload_with_progress_and_config(&best_server, || {}, &mut config)?);


    let result = speedtest::SpeedTestResultOwned { 
        download_measurement: download_measurement,
        upload_measurement:upload_measurement,
        server: best_server.to_owned(), 
        latency_measurement: speedtest::SpeedTestLatencyTestResultOwned { server: best_server, latency: best_server_info.latency.to_owned()}
    };

    Ok(result)
}