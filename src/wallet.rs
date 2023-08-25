use bdk::{
    bitcoin::{
        secp256k1::{All, Secp256k1},
        Network, Txid,
    },
    chain::{keychain::LocalChangeSet, ConfirmationTimeAnchor},
    wallet::wallet_name_from_descriptor,
    KeychainKind, TransactionDetails, Wallet,
};
use bdk_esplora::{esplora_client, EsploraAsyncExt};
use bdk_file_store::Store;
use cln_plugin::Error;
use home::home_dir;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, fmt, io::Write};

pub const DATADIR: &str = ".watchdescriptor";
const STOP_GAP: usize = 50;
const PARALLEL_REQUESTS: usize = 5;

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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum WDNetwork {
    #[serde(rename = "bitcoin")]
    Mainnet,
    Testnet,
    Regtest,
    Signet,
    Mutinynet,
}

pub fn get_network_url(network: &str) -> String {
    match network {
        "bitcoin" | "mainnet" => "https://blockstream.info/api".to_owned(),
        "testnet" => "https://blockstream.info/testnet/api".to_owned(),
        "regtest" | "mutinynet" => "https://mutinynet.com/api".to_owned(),
        "signet" => "https://mempool.space/signet/api".to_owned(),
        _ => {
            panic!();
        }
    }
}

/// Parameters related to the `watchdescriptor` command.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DescriptorWallet {
    pub descriptor: String,
    pub change_descriptor: Option<String>,
    pub birthday: Option<u32>,
    pub gap: Option<u32>,
    // pub last_synced: Option<BlockTime>,
    // #[serde(skip_serializing, skip_deserializing)]
    pub transactions: BTreeMap<Txid, TransactionDetails>,
    pub network: Option<Network>,
}
impl DescriptorWallet {
    fn new(
        descriptor: &str,
        change_descriptor: Option<&str>,
        birthday: Option<u64>,
        gap: Option<u64>,
        network: Option<Network>,
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
        if network.is_some() {
            params = params.with_network(network.unwrap())?
        }
        Ok(params)
    }

    fn from_descriptor(descriptor: &str) -> Result<Self, WatchError> {
        Ok(Self {
            descriptor: descriptor.to_owned(),
            change_descriptor: None,
            birthday: None,
            gap: None,
            // last_synced: None,
            transactions: BTreeMap::new(),
            network: None,
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

    fn with_network(self, network: Network) -> Result<Self, WatchError> {
        Ok(Self {
            network: Some(network),
            ..self
        })
    }

    // pub fn update_last_synced(&mut self, last_synced: BlockTime) {
    //     self.last_synced = Some(last_synced);
    // }

    pub fn update_transactions(
        &mut self,
        transactions: Vec<TransactionDetails>,
    ) -> Vec<TransactionDetails> {
        let mut new_txs = vec![];
        for tx in transactions {
            if !self.transactions.contains_key(&tx.txid) {
                new_txs.push(tx.clone());
                self.transactions.insert(tx.txid, tx);
            }
        }
        new_txs
        // self.transactions = transactions;
    }

    pub fn get_name(&self) -> Result<String, Error> {
        Ok(wallet_name_from_descriptor(
            &self.descriptor,
            self.change_descriptor.as_ref(),
            self.network.unwrap(),
            &Secp256k1::<All>::new(),
        )?)
    }

    pub async fn fetch_wallet<'a>(
        &self,
        // dw: &DescriptorWallet,
    ) -> Result<Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>, Error>
    {
        // let db_path = std::env::temp_dir().join("bdk-esplora-async-example");
        log::info!("creating path");
        // let db_filename: String = general_purpose::STANDARD_NO_PAD.encode(dw.descriptor.as_bytes());
        // let db_filename: String = calc_checksum(&dw.descriptor)?;
        let db_filename = self.get_name()?;
        let db_path = home_dir()
            .unwrap()
            .join(DATADIR)
            .join(format!("{}.db", db_filename,));
        log::info!("searching for path: {:?}", db_path);
        let db = Store::<bdk::wallet::ChangeSet>::new_from_path(DATADIR.as_bytes(), db_path)?;
        log::info!("db created!");
        // let external_descriptor = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/0'/0'/0/*)";
        // mutinynet_descriptor = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/*)"
        let _mutinynet_descriptor_ext = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/84'/0'/0'/0/*)";
        let _mutinynet_descriptor_int = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/84'/0'/0'/1/*)";
        let _mutinynet_descriptor_ext_2 = "wpkh(tprv8ZgxMBicQKsPeRye8MhHA8hLxMuomycmGYXyRs7zViNck2VJsCJMTPt81Que8qp3PyPgQRnN7Gb1JyBVBKgj8AKEoEmmYxYDwzZJ63q1yjA/84'/0'/0'/0/*)";
        let _mutinynet_descriptor_int_2 = "wpkh(tprv8ZgxMBicQKsPeRye8MhHA8hLxMuomycmGYXyRs7zViNck2VJsCJMTPt81Que8qp3PyPgQRnN7Gb1JyBVBKgj8AKEoEmmYxYDwzZJ63q1yjA/84'/0'/0'/1/*)";
        // let external_descriptor = "wpkh(tpubEBr4i6yk5nf5DAaJpsi9N2pPYBeJ7fZ5Z9rmN4977iYLCGco1VyjB9tvvuvYtfZzjD5A8igzgw3HeWeeKFmanHYqksqZXYXGsw5zjnj7KM9/*)";
        // let internal_descriptor = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/0'/0'/1/*)";

        // let external_descriptor = mutinynet_descriptor_ext;
        // let internal_descriptor = mutinynet_descriptor_int;
        let external_descriptor = self.descriptor.clone();
        let internal_descriptor = self.change_descriptor.clone();
        // log::info!(
        //     "about to create wallet {}, {:?}",
        //     &dw.descriptor,
        //     &dw.change_descriptor,
        // );
        let mut wallet = Wallet::new(
            &external_descriptor,
            internal_descriptor.as_ref(),
            db,
            Network::Testnet,
        )?;
        log::info!("wallet created!");

        // let address = wallet.get_address(AddressIndex::New);
        // log::info!("Generated Address: {}", address);

        let balance = wallet.get_balance();
        log::info!("Wallet balance before syncing: {} sats", balance.total());

        log::info!("Syncing...");
        log::info!("using network: {}", json!(self.network).as_str().unwrap());
        log::info!(
            "using esplora url: {}",
            get_network_url(json!(self.network).as_str().unwrap()).as_str()
        );
        let client =
            // esplora_client::Builder::new("https://blockstream.info/testnet/api").build_async()?;
            esplora_client::Builder::new(
                get_network_url(
                        json!(self.network).as_str().unwrap()
                ).as_str()
            ).build_async()?;

        let local_chain = wallet.checkpoints();
        let keychain_spks = wallet
            .spks_of_all_keychains()
            .into_iter()
            .map(|(k, k_spks)| {
                let mut once = Some(());
                let mut stdout = std::io::stdout();
                let k_spks = k_spks
                    .inspect(move |(spk_i, _)| match once.take() {
                        Some(_) => log::info!("\nScanning keychain [{:?}]", k),
                        None => log::info!(" {:<3}", spk_i),
                    })
                    .inspect(move |_| stdout.flush().expect("must flush"));
                (k, k_spks)
            })
            .collect();
        log::info!("CAG finished scanning");
        let update = client
            .scan(
                local_chain,
                keychain_spks,
                [],
                [],
                STOP_GAP,
                PARALLEL_REQUESTS,
            )
            .await?;
        wallet.apply_update(update)?;
        wallet.commit()?;

        let balance = wallet.get_balance();
        log::info!("Wallet balance after syncing: {} sats", balance.total());
        return Ok(wallet);
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

                        DescriptorWallet::new(descriptor, change_descriptor, birthday, gap, None)
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
                        None,
                    )
                }
            },
            _ => Err(WatchError::InvalidFormat(
                format!("Unexpected request format. Expected: <descriptor>, [change_descriptor, birthday, gap], either as ordered or keyword args. Received: '{value}'"),
            )),
        }
    }
}
