use super::distance::{self, EarthLocation};
use super::{error::SpeedTestError, speedtest::SpeedTestServer, speedtest_config::SpeedTestConfig};
use std::cmp::Ordering::Less;

pub struct SpeedTestServersConfig {
    pub servers: Vec<SpeedTestServer>,
}

impl SpeedTestServersConfig {
    pub fn parse_with_config(
        server_config_xml: &str,
        config: &SpeedTestConfig,
    ) -> Result<SpeedTestServersConfig, SpeedTestError> {
        let document = roxmltree::Document::parse(server_config_xml)?;
        let servers = document
            .descendants()
            .filter(|node| node.tag_name().name() == "server")
            .map::<Result<_, SpeedTestError>, _>(|n| {
                let location = EarthLocation {
                    latitude: n
                        .attribute("lat")
                        .ok_or(SpeedTestError::ServerParseError)?
                        .parse()?,
                    longitude: n
                        .attribute("lon")
                        .ok_or(SpeedTestError::ServerParseError)?
                        .parse()?,
                };
                Ok(SpeedTestServer {
                    country: n
                        .attribute("country")
                        .ok_or(SpeedTestError::ServerParseError)?
                        .to_string(),
                    host: n
                        .attribute("host")
                        .ok_or(SpeedTestError::ServerParseError)?
                        .to_string(),
                    id: n
                        .attribute("id")
                        .ok_or(SpeedTestError::ServerParseError)?
                        .parse()?,
                    location: location.clone(),
                    distance: Some(distance::compute_distance(&config.location, &location)),
                    name: n
                        .attribute("name")
                        .ok_or(SpeedTestError::ServerParseError)?
                        .to_string(),
                    sponsor: n
                        .attribute("sponsor")
                        .ok_or(SpeedTestError::ServerParseError)?
                        .to_string(),
                    url: n
                        .attribute("url")
                        .ok_or(SpeedTestError::ServerParseError)?
                        .to_string(),
                })
            })
            .filter_map(Result::ok)
            .filter(|server| !config.ignore_servers.contains(&server.id))
            .collect();
        Ok(SpeedTestServersConfig { servers })
    }

    pub fn servers_sorted_by_distance(&self, config: &SpeedTestConfig) -> Vec<SpeedTestServer> {
        let location = &config.location;
        let mut sorted_servers = self.servers.clone();
        sorted_servers.sort_by(|a, b| {
            let a_distance = distance::compute_distance(location, &a.location);
            let b_distance = distance::compute_distance(location, &b.location);
            a_distance.partial_cmp(&b_distance).unwrap_or(Less)
        });
        sorted_servers
    }
}