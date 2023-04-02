use serde::Serialize;
use std::fmt;

/// Errors related to the `watchdescriptor` command.
#[derive(Debug)]
pub enum WatchError {
    InvalidDescriptor(String),
    InvalidChangeDescriptor(String),
    InvalidBirthday(String),
    InvalidGap(String),
    InvalidFormat(String),
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            WatchError::InvalidDescriptor(x) => write!(f, "{x}"),
            WatchError::InvalidChangeDescriptor(x) => write!(f, "{x}"),
            WatchError::InvalidBirthday(x) => write!(f, "{x}"),
            WatchError::InvalidGap(x) => write!(f, "{x}"),
            WatchError::InvalidFormat(x) => write!(f, "{x}"),
        }
    }
}

/// Parameters related to the `watchdescriptor` command.
#[derive(Debug, Serialize)]
pub struct DescriptorWallet {
    pub descriptor: String,
    pub change_descriptor: Option<String>,
    pub birthday: Option<u32>,
    pub gap: Option<u32>,
}
impl DescriptorWallet {
    fn new(
        descriptor: &str,
        change_descriptor: Option<&str>,
        birthday: Option<u64>,
        gap: Option<u64>,
    ) -> Result<Self, WatchError> {
        let mut params = DescriptorWallet::from_descriptor(descriptor)?;
        if change_descriptor.is_some() {
            params = params.with_change_descriptor(change_descriptor.unwrap())?
        }
        if birthday.is_some() {
            params = params.with_birthday(birthday.unwrap())?
        }
        if gap.is_some() {
            params = params.with_gap(gap.unwrap())?
        }
        Ok(params)
    }

    fn from_descriptor(descriptor: &str) -> Result<Self, WatchError> {
        Ok(Self {
            descriptor: descriptor.to_owned(),
            change_descriptor: None,
            birthday: None,
            gap: None,
        })
    }

    fn with_change_descriptor(self, change_descriptor: &str) -> Result<Self, WatchError> {
        if change_descriptor.is_empty() {
            Err(WatchError::InvalidChangeDescriptor(
                "change_descriptor is empty".to_owned(),
            ))
        } else {
            Ok(Self {
                change_descriptor: Some(String::from(change_descriptor)),
                ..self
            })
        }
    }

    fn with_birthday(self, birthday: u64) -> Result<Self, WatchError> {
        if birthday > u32::MAX as u64 {
            Err(WatchError::InvalidBirthday(format!(
                "birthday must be between 0 and 4294967295. Received: {birthday}"
            )))
        } else {
            Ok(Self {
                birthday: Some(birthday as u32),
                ..self
            })
        }
    }

    fn with_gap(self, gap: u64) -> Result<Self, WatchError> {
        if gap > u32::MAX as u64 / 2 {
            Err(WatchError::InvalidBirthday(format!(
                "gap must be between 0 and 2147483647. Received: {gap}"
            )))
        } else {
            Ok(Self {
                gap: Some(gap as u32),
                ..self
            })
        }
    }
}

impl TryFrom<serde_json::Value> for DescriptorWallet {
    type Error = WatchError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        log::info!("entering try_from");
        match value {
            serde_json::Value::Array(a) => {
                log::info!("try_from: array detected = {:?}", a);
                let param_count = a.len();

                match param_count {
                    // 1 => DescriptorWallet::try_from(a.pop().unwrap()),
                    1..=4 => {
                        let descriptor = a.get(0).unwrap().as_str().ok_or_else(|| WatchError::InvalidDescriptor("descriptor must be a string".to_string()))?;
                        // let change_descriptor = Some(a.get(1).unwrap().as_str().ok_or_else(|| WatchError::InvalidChangeDescriptor("change_descriptor must be a string".to_string()))?);
                        log::info!("try_from array: change_descriptor = {:?}", a.get(1));
                        let change_descriptor = if let Some(cd) = a.get(1) {
                            Some(cd.as_str().ok_or_else(|| WatchError::InvalidChangeDescriptor(format!("change_descriptor must be a string. Received: {cd}")))?)
                        } else {
                            None
                        };
                        let birthday = if let Some(b) = a.get(2) {
                            Some(b.as_u64().ok_or_else(|| WatchError::InvalidBirthday(format!("birthday must be a number. Received: {b}")))?)
                        } else {
                            None
                        };
                        let gap = if let Some(g) = a.get(3) {
                            Some(g.as_u64().ok_or_else(|| WatchError::InvalidGap(format!("gap must be a number. Received: {g}")))?)
                        } else {
                            None
                        };

                        DescriptorWallet::new(descriptor, change_descriptor, birthday, gap)
                    }
                    _ => Err(WatchError::InvalidFormat(format!("Unexpected request format. The request needs 1-4 parameters. Received: {param_count}"))),
                }
            },
            serde_json::Value::Object(m) => {
                log::info!("try_from: object detected");
                let allowed_keys = ["descriptor", "change_descriptor", "birthday", "gap"];
                let param_count = m.len();

                 if m.is_empty() || param_count > allowed_keys.len() {
                    Err(WatchError::InvalidFormat(format!("Unexpected request format. The request needs 1-4 parameters. Received: {param_count}")))
                 } else if !m.contains_key(allowed_keys[0]){
                    Err(WatchError::InvalidDescriptor(format!("{} is mandatory", allowed_keys[0])))
                 } else if !m.iter().all(|(k, _)| allowed_keys.contains(&k.as_str())) {
                    Err(WatchError::InvalidFormat(format!("Invalid named parameter found in request. Allowed named params: ['descriptor', 'change_descriptor', 'birthday', 'gap']")))
                 } else {
                    DescriptorWallet::new(
                        m.get("descriptor").unwrap().as_str().unwrap(),
                        match m.get("change_descriptor") {
                            Some(v) => Some(v.as_str().unwrap()),
                            None => None,
                        },
                        match m.get("birthday") {
                            Some(v) => Some(v.as_u64().unwrap()),
                            None => None,
                        },
                        match m.get("gap") {
                            Some(v) => Some(v.as_u64().unwrap()),
                            None => None,
                        },
                    )
                }
            },
            _ => Err(WatchError::InvalidFormat(
                format!("Unexpected request format. Expected: <descriptor>, [change_descriptor, birthday, gap], either as ordered or keyword args. Received: '{value}'"),
            )),
        }
    }
}
