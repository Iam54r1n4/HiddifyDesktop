use std::{
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use log::info;
use reqwest::{blocking::{Body, Client, Request, Response}};
use reqwest::header::{HeaderValue, CONNECTION, CONTENT_TYPE, REFERER, USER_AGENT};
use reqwest::Url;
use md5;
use super::{distance::EarthLocation, speedtest_config::{SpeedTestConfig,Proxy}};
use super::error::SpeedTestError;
use super::speedtest_servers_config::SpeedTestServersConfig;
use rayon::prelude::*;

const ST_USER_AGENT: &str = concat!("reqwest/speedtest-rs ", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug)]
pub struct SpeedTestServer {
    pub country: String,
    pub host: String,
    pub id: u32,
    pub location: EarthLocation,
    pub distance: Option<f32>,
    pub name: String,
    pub sponsor: String,
    pub url: String,
}

pub fn download_configuration(proxy: &Proxy) -> Result<Response, SpeedTestError> {
    info!("Downloading Configuration from speedtest.net");

    let url = "http://www.speedtest.net/speedtest-config.php";
    let client;
    match &proxy.0{
        Some(proxy) => {
            client = Client::builder().proxy(proxy.to_owned()).build()?
        },
        None => client = Client::new(),
    }
    // Creating an outgoing request.
    let res = client
        .get(url)
        .header(CONNECTION, "close")
        .header(USER_AGENT, ST_USER_AGENT.to_owned())
        .send()?;
    info!("Downloaded Configuration from speedtest.net");
    Ok(res)
}

pub fn get_configuration(proxy: &Proxy) -> Result<SpeedTestConfig, SpeedTestError> {
    let config_body = download_configuration(proxy)?;
    info!("Parsing Configuration");
    let mut spt_config = SpeedTestConfig::parse(&(config_body.text()?))?;
    info!("Parsed Configuration");
    spt_config.proxy = proxy.to_owned();
    Ok(spt_config)
}

pub fn download_server_list(config: &SpeedTestConfig) -> Result<Response, SpeedTestError> {
    info!("Download Server List");
    let url = "http://www.speedtest.net/speedtest-servers.php";
    let client;
    match &config.proxy.0{
        Some(v) => client = Client::builder().proxy(v.to_owned()).build()?,
        None => client = Client::new()
    }
    let server_res = client
        .get(url)
        .header(CONNECTION, "close")
        .header(USER_AGENT, ST_USER_AGENT)
        .send()?;
    info!("Downloaded Server List");
    Ok(server_res)
}

pub fn get_server_list_with_config(
    config: &SpeedTestConfig,
) -> Result<SpeedTestServersConfig, SpeedTestError> {
    let config_body = download_server_list(config)?;
    info!("Parsing Server List");
    let server_config_string = config_body.text()?;
    let spt_config = SpeedTestServersConfig::parse_with_config(&server_config_string, config);
    info!("Parsed Server List");
    spt_config
}

#[derive(Debug)]
pub struct SpeedTestLatencyTestResult<'a> {
    pub server: &'a SpeedTestServer,
    pub latency: Duration,
}

#[derive(Debug)]
pub struct SpeedTestLatencyTestResultOwned{
    pub server: SpeedTestServer,
    pub latency: Duration,
}

pub fn get_best_server_based_on_latency<'a>(
    servers: &'a [SpeedTestServer],
    config: &'a SpeedTestConfig
) -> Result<SpeedTestLatencyTestResult<'a>, SpeedTestError> {
    info!("Testing for fastest server");
    let client;
    match &config.proxy.0{
        Some(v) => client = Client::builder().proxy(v.to_owned()).build()?,
        None => client = Client::new()
    }
    let mut fastest_server = None;
    let mut fastest_latency = Duration::new(u64::MAX, 0);
    'server_loop: for server in servers {
        let path = Path::new(&server.url);
        let latency_path = format!(
            "{}/latency.txt",
            path.parent()
                .ok_or(SpeedTestError::LatencyTestInvalidPath)?
                .display()
        );
        info!("Downloading: {:?}", latency_path);
        let mut latency_measurements = vec![];
        for _ in 0..3 {
            let start_time = SystemTime::now();
            let res = client
                .get(&latency_path)
                .header(CONNECTION, "close")
                .header(USER_AGENT, ST_USER_AGENT.to_owned())
                .send();
            if res.is_err() {
                continue 'server_loop;
            }
            res?.bytes()?.last();
            let latency_measurement = SystemTime::now().duration_since(start_time)?;
            info!("Sampled {} ms", latency_measurement.as_millis());
            latency_measurements.push(latency_measurement);
        }
        // Divide by the double to get the non-RTT time but the trip time.
        // NOT PING or RTT
        // https://github.com/sivel/speedtest-cli/pull/199
        let latency = latency_measurements
            .iter()
            .fold(Duration::new(0, 0), |a, &i| a + i)
            / ((latency_measurements.len() as u32) * 2);
        info!("Trip calculated to {} ms", latency.as_millis());

        if latency < fastest_latency {
            fastest_server = Some(server);
            fastest_latency = latency;
        }
    }
    info!(
        "Fastest Server @ {}ms : {:?}",
        fastest_latency.as_millis(),
        fastest_server
    );
    Ok(SpeedTestLatencyTestResult {
        server: fastest_server.ok_or(SpeedTestError::LatencyTestClosestError)?,
        latency: fastest_latency,
    })
}

#[derive(Debug)]
pub struct SpeedMeasurement {
    pub size: usize,
    pub duration: Duration,
}

impl SpeedMeasurement {
    pub fn kbps(&self) -> u32 {
        (self.size as u32 * 8) / self.duration.as_millis() as u32
    }
    pub fn Mbps(&self) -> u32{
        ((self.kbps()) as f32 / 1000.00) as u32
    }
    pub fn MBps(&self) -> u32{
        (((self.kbps() / 8) as f32 ) / 1000.00) as u32
    }
    pub fn bps_f64(&self) -> f64 {
        (self.size as f64 * 8.0) / (self.duration.as_millis() as f64 / (1000.0))
    }
}

pub fn test_download_with_progress_and_config<F>(
    server: &SpeedTestServer,
    progress_callback: F,
    config: &mut SpeedTestConfig,
) -> Result<SpeedMeasurement, SpeedTestError>
where
    F: Fn() + Send + Sync + 'static,
{
    info!("Testing Download speed");
    let root_url = Url::parse(&server.url)?;

    let mut urls = vec![];
    for size in &config.sizes.download {
        let mut download_with_size_url = root_url.clone();
        {
            let mut path_segments_mut = download_with_size_url
                .path_segments_mut()
                .map_err(|_| SpeedTestError::ServerParseError)?;
            path_segments_mut.push(&format!("random{}x{}.jpg", size, size));
        }
        for _ in 0..config.counts.download {
            urls.push(download_with_size_url.clone());
        }
    }

    let _request_count = urls.len();

    let requests: Vec<_> = urls
        .iter()
        .enumerate()
        .map(|(i, url)| {
            let mut cache_busting_url = url.clone();
            cache_busting_url.query_pairs_mut().append_pair(
                "x",
                &format!(
                    "{}.{}",
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)?
                        .as_millis()
                        .to_string(),
                    i
                ),
            );
            let mut request = Request::new(reqwest::Method::GET, url.clone());
            request.headers_mut().insert(
                reqwest::header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache"),
            );
            request.headers_mut().insert(
                reqwest::header::USER_AGENT,
                HeaderValue::from_static(ST_USER_AGENT),
            );
            request.headers_mut().insert(
                reqwest::header::CONNECTION,
                HeaderValue::from_static("close"),
            );
            Ok(request)
        })
        .collect::<Result<Vec<_>, SpeedTestError>>()?;

    // TODO: Setup Ctrl-C Termination to use this "event".
    let early_termination = AtomicBool::new(false);

    // Start Timer
    let start_time = SystemTime::now();

    info!("Download Threads: {}", config.threads.download);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(config.threads.download)
        .build()?;

    info!("Total to be requested {:?}", requests);

    let total_transferred_per_thread = pool.install(|| {
        requests
            .into_iter()
            // Make it sequential like the original. Ramp up the file sizes.
            .par_bridge()
            .map(|r| -> Result<_, SpeedTestError> {
                let client;
                match &config.proxy.0{
                    Some(v) => client = Client::builder().proxy(v.to_owned()).build()?,
                    None => client = Client::new()
                }
                                
                // let downloaded_count = vec![];
                progress_callback();
                info!("Requesting {}", r.url());
                let mut response = client.execute(r)?;
                let mut buf = [0u8; 10240];
                let mut read_amounts = vec![];
                while (SystemTime::now().duration_since(start_time)? < config.length.upload)
                    && !early_termination.load(Ordering::Relaxed)
                {
                    let read_amount = response.read(&mut buf)?;
                    read_amounts.push(read_amount);
                    if read_amount == 0 {
                        break;
                    }
                }
                let total_transfered = read_amounts.iter().sum::<usize>();
                progress_callback();

                Ok(total_transfered)
            })
            .collect::<Result<Vec<usize>, SpeedTestError>>()
    });

    let total_transferred: usize = total_transferred_per_thread?.iter().sum();

    let end_time = SystemTime::now();

    let measurement = SpeedMeasurement {
        size: total_transferred,
        duration: end_time.duration_since(start_time)?,
    };

    if measurement.bps_f64() > 100000.0 {
        config.threads.upload = 8
    }

    Ok(measurement)
}

#[derive(Debug)]
pub struct SpeedTestUploadRequest {
    pub request: Request,
    pub size: usize,
}

pub fn test_upload_with_progress_and_config<F>(
    server: &SpeedTestServer,
    progress_callback: F,
    config: &SpeedTestConfig,
) -> Result<SpeedMeasurement, SpeedTestError>
where
    F: Fn() + Send + Sync + 'static,
{
    info!("Testing Upload speed");

    let mut sizes = vec![];
    for &size in &config.sizes.upload {
        for _ in 0..config.counts.upload {
            sizes.push(size)
        }
    }

    let best_url = Url::parse(&server.url)?;

    let request_count = config.upload_max;

    let requests: Vec<SpeedTestUploadRequest> = sizes
        .into_iter()
        .map(|size| {
            let content_iter = b"content1="
                .iter()
                .chain(b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ".iter().cycle())
                .take(size);
            let content_iter_read = iter_read::IterRead::new(content_iter);
            let body = Body::sized(content_iter_read, size as u64);
            let mut request = Request::new(reqwest::Method::POST, best_url.clone());
            request.headers_mut().insert(
                reqwest::header::USER_AGENT,
                HeaderValue::from_static(ST_USER_AGENT),
            );
            request.headers_mut().insert(
                reqwest::header::CONNECTION,
                HeaderValue::from_static("close"),
            );
            *request.body_mut() = Some(body);
            Ok(SpeedTestUploadRequest { request, size })
        })
        .collect::<Result<Vec<_>, SpeedTestError>>()?;
    // TODO: Setup Ctrl-C Termination to use this "event".
    let early_termination = AtomicBool::new(false);

    // Start Timer
    let start_time = SystemTime::now();

    info!("Upload Threads: {}", config.threads.upload);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(config.threads.upload)
        .build()?;

    info!("Total to be requested {:?}", requests.len());
    let total_transferred_per_thread = pool.install(|| {
        requests
            .into_iter()
            .take(request_count)
            // Make it sequential like the original. Ramp up the file sizes.
            .par_bridge()
            .map(|r| -> Result<usize, SpeedTestError> {
                progress_callback();

                if (SystemTime::now().duration_since(start_time)? < config.length.upload)
                    && !early_termination.load(Ordering::Relaxed)
                {
                    let client;
                    match &config.proxy.0{
                        Some(v) => client = Client::builder().proxy(v.to_owned()).build()?,
                        None => client = Client::new()
                    }
                    
                    info!("Requesting {}", r.request.url());
                    let response = client.execute(r.request);
                    if response.is_err() {
                        return Ok(r.size);
                    };
                } else {
                    return Ok(0);
                }
                progress_callback();

                Ok(r.size)
            })
            .collect::<Result<Vec<usize>, SpeedTestError>>()
    });

    let total_transferred: usize = total_transferred_per_thread?.iter().sum();

    let end_time = SystemTime::now();

    let measurement = SpeedMeasurement {
        size: total_transferred,
        duration: end_time.duration_since(start_time)?,
    };

    Ok(measurement)
}

#[derive(Debug)]
pub struct SpeedTestResult<'a, 'b, 'c> {
    pub download_measurement: Option<&'a SpeedMeasurement>,
    pub upload_measurement: Option<&'b SpeedMeasurement>,
    pub server: &'c SpeedTestServer,
    pub latency_measurement: &'c SpeedTestLatencyTestResult<'c>,
}

impl<'a, 'b, 'c> SpeedTestResult<'a, 'b, 'c> {
    pub fn hash(&self) -> String {
        let hashed_str = format!(
            "{}-{}-{}-{}",
            self.latency_measurement.latency.as_millis(),
            if let Some(upload_measurement) = self.upload_measurement {
                upload_measurement.kbps()
            } else {
                0
            },
            if let Some(download_measurement) = self.download_measurement {
                download_measurement.kbps()
            } else {
                0
            },
            "297aae72"
        );

        format!("{:x}", md5::compute(hashed_str))
    }
}


#[derive(Debug)]
pub struct SpeedTestResultOwned{
    pub download_measurement: Option<SpeedMeasurement>,
    pub upload_measurement: Option<SpeedMeasurement>,
    pub server: SpeedTestServer,
    pub latency_measurement: SpeedTestLatencyTestResultOwned,
}
pub fn get_share_url(speedtest_result: &SpeedTestResult) -> Result<String, SpeedTestError> {
    info!("Generating share URL");
    let download = if let Some(download_measurement) = speedtest_result.download_measurement {
        download_measurement.kbps()
    } else {
        0
    };
    info!("Download parameter is {:?}", download);
    let upload = if let Some(upload_measurement) = speedtest_result.upload_measurement {
        upload_measurement.kbps()
    } else {
        0
    };
    info!("Upload parameter is {:?}", upload);
    let server = speedtest_result.server.id;
    info!("Server parameter is {:?}", server);
    let ping = speedtest_result.latency_measurement.latency;
    info!("Ping parameter is {:?}", ping);

    let pairs = [
        (
            "download",
            format!(
                "{}",
                if let Some(download_measurement) = speedtest_result.download_measurement {
                    download_measurement.kbps()
                } else {
                    0
                }
            ),
        ),
        ("ping", format!("{}", ping.as_millis())),
        (
            "upload",
            format!(
                "{}",
                if let Some(upload_measurement) = speedtest_result.upload_measurement {
                    upload_measurement.kbps()
                } else {
                    0
                }
            ),
        ),
        ("promo", format!("")),
        ("startmode", "pingselect".to_string()),
        ("recommendedserverid", format!("{}", server)),
        ("accuracy", "1".to_string()),
        ("serverid", format!("{}", server)),
        ("hash", speedtest_result.hash()),
    ];

    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter())
        .finish();

    info!("Share Body Request: {:?}", body);

    let client = Client::new();
    let res = client
        .post("http://www.speedtest.net/api/api.php")
        .header(CONNECTION, "close")
        .header(REFERER, "http://c.speedtest.net/flash/speedtest.swf")
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send();
    let encode_return = res?.text()?;
    let response_id = parse_share_request_response_id(encode_return.as_bytes())?;
    Ok(format!(
        "http://www.speedtest.net/result/{}.png",
        response_id
    ))
}

pub fn parse_share_request_response_id(input: &[u8]) -> Result<String, SpeedTestError> {
    let pairs = url::form_urlencoded::parse(input);
    for pair in pairs {
        if pair.0 == "resultid" {
            return Ok(pair.1.into_owned());
        }
    }
    Err(SpeedTestError::ParseShareUrlError)
}