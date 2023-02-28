use anyhow::{anyhow, bail, Context, Result};
use nanoid::nanoid;
use serde::{de::DeserializeOwned, Serialize};
use serde_yaml::{Mapping, Value};
use std::{fs, path::PathBuf, process::Command, str::FromStr, thread};
use tauri::{AppHandle};
use std::time::Duration;
use crate::{utils::{resolve,dirs}, cmds,config::{Proxy}};

/// read data from yaml as struct T
pub fn read_yaml<T: DeserializeOwned>(path: &PathBuf) -> Result<T> {
    if !path.exists() {
        bail!("file not found \"{}\"", path.display());
    }

    let yaml_str = fs::read_to_string(&path)
        .context(format!("failed to read the file \"{}\"", path.display()))?;

    serde_yaml::from_str::<T>(&yaml_str).context(format!(
        "failed to read the file with yaml format \"{}\"",
        path.display()
    ))
}

/// read mapping from yaml fix #165
pub fn read_merge_mapping(path: &PathBuf) -> Result<Mapping> {
    let mut val: Value = read_yaml(path)?;
    val.apply_merge()
        .context(format!("failed to apply merge \"{}\"", path.display()))?;

    Ok(val
        .as_mapping()
        .ok_or(anyhow!(
            "failed to transform to yaml mapping \"{}\"",
            path.display()
        ))?
        .to_owned())
}

/// save the data to the file
/// can set `prefix` string to add some comments
pub fn save_yaml<T: Serialize>(path: &PathBuf, data: &T, prefix: Option<&str>) -> Result<()> {
    let data_str = serde_yaml::to_string(data)?;

    let yaml_str = match prefix {
        Some(prefix) => format!("{prefix}\n\n{data_str}"),
        None => data_str,
    };

    let path_str = path.as_os_str().to_string_lossy().to_string();
    fs::write(path, yaml_str.as_bytes()).context(format!("failed to save file \"{path_str}\""))
}

const ALPHABET: [char; 62] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i',
    'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B',
    'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U',
    'V', 'W', 'X', 'Y', 'Z',
];

/// generate the uid
pub fn get_uid(prefix: &str) -> String {
    let id = nanoid!(11, &ALPHABET);
    format!("{prefix}{id}")
}

/// parse the string
/// xxx=123123; => 123123
pub fn parse_str<T: FromStr>(target: &str, key: &str) -> Option<T> {
    target.find(key).and_then(|idx| {
        let idx = idx + key.len();
        let value = &target[idx..];

        match value.split(';').nth(0) {
            Some(value) => value.trim().parse(),
            None => value.trim().parse(),
        }
        .ok()
    })
}

/// open file
/// use vscode by default
pub fn open_file(path: PathBuf) -> Result<()> {
    // use vscode first
    if let Ok(code) = which::which("code") {
        let mut command = Command::new(&code);

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            if let Err(err) = command.creation_flags(0x08000000).arg(&path).spawn() {
                log::error!(target: "app", "failed to open with VScode `{err}`");
                open::that(path)?;
            }
        }

        #[cfg(not(target_os = "windows"))]
        if let Err(err) = command.arg(&path).spawn() {
            log::error!(target: "app", "failed to open with VScode `{err}`");
            open::that(path)?;
        }

        return Ok(());
    }

    open::that(path)?;
    Ok(())
}

#[derive(Debug)]
pub enum ExtractDeeplinkError{
    InvalidInput
}
// pub fn extract_url_and_profile_name_from_deep_link(deep_link:&String) -> Result<(String,String),ExtractDeeplinkError>{
//     // Sample: clash://install-config?url=https://mysite.com/all.yml&name=profilename
//     let (url,profile) = {
//         let pruned = deep_link.split("url=").collect::<Vec<_>>();
//         if pruned.len() < 2 {
//             return Err(ExtractDeeplinkError::InvalidInput)
//         }
//         let url_and_profile_name = pruned[1].split("&").collect::<Vec<_>>();
//         if url_and_profile_name.len() < 2{
//             return Err(ExtractDeeplinkError::InvalidInput)
//         }
//         let url = url_and_profile_name[0].to_string();
//         let profile_name = {
//             let splitted: Vec<_> = url_and_profile_name[1].split("=").collect();
//             if splitted.len() < 2 {
//                 return Err(ExtractDeeplinkError::InvalidInput)
//             }
//             splitted[1].to_string()
//         };

//         (url,profile_name)
//     };

//     return Ok((url,profile));
// }

pub fn convert_deeplink_to_url_for_import_profile(deep_link:&String) -> Result<String,ExtractDeeplinkError>{
    // Sample: clash://install-config?url=https://mysite.com/all.yml&name=profilename
    let import_profile_url_raw = {
        let url_part:Vec<_> = deep_link.split("url=").collect();
        if url_part.len() < 2{
            return Err(ExtractDeeplinkError::InvalidInput)
        }
        url_part
    };

    // Convert url to something that import_profile functin can use
    let import_profile_url = import_profile_url_raw[1].replacen('&', "?", 1);
    Ok(import_profile_url)
}

// Focus to the main window, and back the NEED_WINDOW_BE_FOCUS to false, and wait for NEED_WINDOW_BE_FOCUS be true to do its job
pub fn focus_to_main_window_if_needed(app_handle:&AppHandle){
    loop{
        unsafe{
            if *crate::NEED_WINDOW_BE_FOCUS.lock().unwrap() == true{
                // Show main window is exist, otherwise create main window and show it
                resolve::create_window(app_handle);
                *crate::NEED_WINDOW_BE_FOCUS.lock().unwrap() = false;
            }
        }
        thread::sleep(Duration::from_millis(1400));
    }
}

pub async fn select_last_profile() -> Result<(),()>{
    match cmds::get_profiles(){
        Ok(mut prf_config) => {
            match prf_config.get_items(){
                Some(profiles) => {
                    match profiles.last(){
                        Some(last_prf) => {
                            prf_config.current = last_prf.uid.clone();

                            if let Err(_) = cmds::patch_profiles_config(prf_config).await{
                                return Err(())
                            }
                            return Ok(())
                        },
                        None => return Err(())
                    }
                },
                None => return Err(())
            }
        },
        Err(_) => return Err(())
    }
}

pub async fn get_current_profile_proxies() -> Result<Vec<Proxy>>{
    let current_profile_name = {
        if let Ok(config) = cmds::get_profiles(){
            if let Some(mut current) = config.get_current(){
                current.push_str(".yaml");
                current
            }else{
                return Err(anyhow::Error::msg("Error occurred during get current profile file name"))
            }
            
        }else{
            return Err(anyhow::Error::msg("Error occurred during get profile config(IProfile)"))
        }
    };

    let current_profile_file_path = dirs::app_profiles_dir()?.join(current_profile_name);

    let current_profile_yaml = read_yaml::<Value>(&current_profile_file_path).unwrap();
    let proxies = {
        if let   Some(value) = current_profile_yaml.get("proxies"){
            value
        }else{
            return Err(anyhow::Error::msg("Error occurred during get proxies field from profile file)"))
        }
    };
    if let Ok(proxies) = serde_yaml::from_value::<Vec<Proxy>>(proxies.to_owned()){
        Ok(proxies)
    }else{
        Err(anyhow::Error::msg("Error occurred during deserialize yaml to Proxy struct"))
    }
}

#[macro_export]
macro_rules! error {
    ($result: expr) => {
        log::error!(target: "app", "{}", $result);
    };
}

#[macro_export]
macro_rules! log_err {
    ($result: expr) => {
        if let Err(err) = $result {
            log::error!(target: "app", "{err}");
        }
    };

    ($result: expr, $err_str: expr) => {
        if let Err(_) = $result {
            log::error!(target: "app", "{}", $err_str);
        }
    };
}

/// wrap the anyhow error
/// transform the error to String
#[macro_export]
macro_rules! wrap_err {
    ($stat: expr) => {
        match $stat {
            Ok(a) => Ok(a),
            Err(err) => {
                log::error!(target: "app", "{}", err.to_string());
                Err(format!("{}", err.to_string()))
            }
        }
    };
}

/// return the string literal error
#[macro_export]
macro_rules! ret_err {
    ($str: expr) => {
        return Err($str.into())
    };
}

#[test]
fn test_parse_value() {
    let test_1 = "upload=111; download=2222; total=3333; expire=444";
    let test_2 = "attachment; filename=Clash.yaml";

    assert_eq!(parse_str::<usize>(test_1, "upload=").unwrap(), 111);
    assert_eq!(parse_str::<usize>(test_1, "download=").unwrap(), 2222);
    assert_eq!(parse_str::<usize>(test_1, "total=").unwrap(), 3333);
    assert_eq!(parse_str::<usize>(test_1, "expire=").unwrap(), 444);
    assert_eq!(
        parse_str::<String>(test_2, "filename=").unwrap(),
        format!("Clash.yaml")
    );

    assert_eq!(parse_str::<usize>(test_1, "aaa="), None);
    assert_eq!(parse_str::<usize>(test_1, "upload1="), None);
    assert_eq!(parse_str::<usize>(test_1, "expire1="), None);
    assert_eq!(parse_str::<usize>(test_2, "attachment="), None);
}
//#[test]
// fn test_extract_url_and_profile_name_from_deep_link(){
//     let s = "clash://install-config?url=https://mysite.com/all.yml&name=profilename";
//     let (url,prof_name) = extract_url_and_profile_name_from_deep_link(&s.to_string()).unwrap();
//     assert_eq!(url,"https://mysite.com/all.yml");
//     assert_eq!(prof_name,"profilename");
// }
#[test]
fn test_convert_deeplink_to_url_for_import_profile(){
    let s = "clashy://install-config?url=https://antyfilter.aeycia.cl/80467cf865c2ef1af111716ddf30dd29/80467cf865c2ef1af111716ddf30dd29/clash/all.yml&name=all_antyfilter.aeycia.cl";
    let res = convert_deeplink_to_url_for_import_profile(&s.to_string());
    if res.is_err(){
        println!("Test failed: {:?}",res.err().unwrap())
    }else{
        panic!("Test successfully completed")
    }
}