use anyhow::anyhow;
use bdk::{
    bitcoin::{
        secp256k1::{All, Secp256k1},
        Network, Txid,
    },
    chain::{keychain::LocalChangeSet, ConfirmationTime, ConfirmationTimeAnchor},
    wallet::wallet_name_from_descriptor,
    KeychainKind, TransactionDetails, Wallet,
};
use bdk_esplora::{esplora_client, EsploraAsyncExt};
use bdk_file_store::Store;
use clap::{command, Parser};
use cln_plugin::{Error, Plugin};
use home::home_dir;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, fmt, io::Write};

use crate::state::State;

pub const DATADIR: &str = ".smaug";
const STOP_GAP: usize = 50;
const PARALLEL_REQUESTS: usize = 5;

pub const UTXO_DEPOSIT_TAG: &str = "utxo_deposit";
pub const UTXO_SPENT_TAG: &str = "utxo_spent";

/// Errors related to the `smaug` command.
#[derive(Debug)]
pub enum WatchError {
    InvalidDescriptor(String),
    InvalidChangeDescriptor(String),
    InvalidBirthday(String),
    InvalidGap(String),
    InvalidFormat(String),
}

impl std::error::Error for WatchError {}

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

#[derive(Debug, Deserialize, Serialize, Clone, Parser)]
#[command(author, version, about, long_about = None)]
pub struct AddArgs {
    /// External descriptor of wallet to add
    pub descriptor: String,
    /// Internal descriptor of wallet to add
    pub change_descriptor: Option<String>,
    /// Birthday of wallet to add. Must be a block height between 0 and 4294967295
    pub birthday: Option<u32>,
    /// Number of empty addresses to scan before giving up. Must be between 0 and 2147483647
    pub gap: Option<u32>,
}

/// Parameters related to the `smaug` command.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DescriptorWallet {
    pub descriptor: String,
    pub change_descriptor: Option<String>,
    pub birthday: Option<u32>,
    pub gap: Option<u32>,
    // pub last_synced: Option<BlockTime>,
    // #[serde(skip_serializing, skip_deserializing)]
    pub transactions: BTreeMap<Txid, TransactionDetails>,
    pub network: Option<String>,
}
impl DescriptorWallet {
    fn new(
        descriptor: &str,
        change_descriptor: Option<&str>,
        birthday: Option<u64>,
        gap: Option<u64>,
        network: Option<String>,
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

    pub fn from_args(args: AddArgs, network: String) -> Result<Self, WatchError> {
        Ok(Self {
            descriptor: args.descriptor,
            change_descriptor: args.change_descriptor,
            birthday: args.birthday,
            gap: args.gap,
            transactions: BTreeMap::new(),
            network: Some(network),
        })
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

    fn with_network(self, network: String) -> Result<Self, WatchError> {
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
        let network = match self.network.clone() {
            Some(n) => match n.as_str() {
                "mutinynet" => Network::Signet,
                _ => match n.parse::<Network>() {
                    Ok(n) => n,
                    Err(e) => return Err(e.into()),
                },
            },
            None => return Err(anyhow!("network is None")),
        };
        Ok(wallet_name_from_descriptor(
            &self.descriptor,
            self.change_descriptor.as_ref(),
            network,
            &Secp256k1::<All>::new(),
        )?)
    }

    pub async fn fetch_wallet<'a>(
        &self,
        // dw: &DescriptorWallet,
    ) -> Result<Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>, Error>
    {
        log::trace!("creating path");
        let db_filename = self.get_name()?;
        let db_path = home_dir()
            .unwrap()
            .join(DATADIR)
            .join(format!("{}.db", db_filename,));
        log::trace!("searching for path: {:?}", db_path);
        let db = Store::<bdk::wallet::ChangeSet>::new_from_path(DATADIR.as_bytes(), db_path)?;
        log::trace!("db created!");
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
        let mut wallet = Wallet::new(
            &external_descriptor,
            internal_descriptor.as_ref(),
            db,
            Network::Signet,
        )?;
        log::trace!("wallet created!");

        let balance = wallet.get_balance();
        log::trace!("Wallet balance before syncing: {} sats", balance.total());

        log::trace!("Syncing...");
        log::debug!("using network: {}", json!(self.network).as_str().unwrap());
        log::debug!(
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
                        Some(_) => log::debug!("\nScanning keychain [{:?}]", k),
                        None => log::trace!(" {:<3}", spk_i),
                    })
                    .inspect(move |_| stdout.flush().expect("must flush"));
                (k, k_spks)
            })
            .collect();
        log::trace!("Finished scanning");
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
        log::trace!("Wallet balance after syncing: {} sats", balance.total());
        return Ok(wallet);
    }

    // assume we own all inputs, ie sent from our wallet. all inputs and outputs should generate coin movement bookkeeper events
    async fn spend_tx_notify<'a>(
        &self,
        plugin: &Plugin<State>,
        wallet: &Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>,
        tx: &TransactionDetails,
    ) -> Result<(), Error> {
        match tx.transaction.clone() {
            Some(t) => {
                // send spent notification for each input
                for input in t.input.iter() {
                    if let Some(po) = wallet.tx_graph().get_txout(input.previous_output) {
                        match tx.confirmation_time {
                            ConfirmationTime::Unconfirmed { .. } => {
                                continue;
                            }
                            ConfirmationTime::Confirmed { height, time } => {
                                let acct = format!("smaug:{}", self.get_name()?);
                                let amount = po.value;
                                let outpoint = format!("{}", input.previous_output.to_string());
                                log::trace!("outpoint = {}", format!("{}", outpoint));
                                let onchain_spend = json!({
                                    "account": acct,
                                    "outpoint": outpoint,
                                    "spending_txid": tx.txid.to_string(),
                                    "amount_msat": amount,
                                    "coin_type": "bcrt",
                                    "timestamp": format!("{}", time),
                                    "blockheight": format!("{}", height),
                                });
                                log::trace!("INSIDE SEND SPEND NOTIFICATION ON SMAUG SIDE");
                                let cloned_plugin = plugin.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = cloned_plugin
                                        .send_custom_notification(
                                            UTXO_SPENT_TAG.to_string(),
                                            onchain_spend,
                                        )
                                        .await
                                    {
                                        log::error!("Error sending custom notification: {:?}", e);
                                    }
                                });
                            }
                        }
                    } else {
                        log::trace!("Transaction prevout not found");
                    }
                }

                // send deposit notification for every output, since all of them are spends from our wallet
                for (vout, output) in t.output.iter().enumerate() {
                    match tx.confirmation_time {
                        ConfirmationTime::Unconfirmed { .. } => {
                            continue;
                        }
                        ConfirmationTime::Confirmed { height, time } => {
                            let acct: String;
                            let transfer_from: String;
                            if wallet.is_mine(&output.script_pubkey) {
                                acct = format!("smaug:{}", self.get_name()?);
                                transfer_from = "external".to_owned();
                            } else {
                                transfer_from = format!("smaug:{}", self.get_name()?);
                                acct = "external".to_owned();
                            }
                            let amount = output.value;
                            let outpoint = format!("{}:{}", tx.txid.to_string(), vout.to_string());
                            log::trace!(
                                "outpoint = {}",
                                format!("{}:{}", tx.txid.to_string(), vout.to_string())
                            );
                            let onchain_deposit = json!({
                                    "account": acct,
                                    "transfer_from": transfer_from,
                                    "outpoint": outpoint,
                                    "spending_txid": tx.txid.to_string(),
                                    "amount_msat": amount,
                                    "coin_type": "bcrt",
                                    "timestamp": format!("{}", time),
                                    "blockheight": format!("{}", height),
                            });
                            log::trace!("INSIDE SEND DEPOSIT NOTIFICATION ON SMAUG SIDE");
                            let cloned_plugin = plugin.clone();
                            tokio::spawn(async move {
                                if let Err(e) = cloned_plugin
                                    .send_custom_notification(
                                        UTXO_DEPOSIT_TAG.to_string(),
                                        onchain_deposit,
                                    )
                                    .await
                                {
                                    log::error!("Error sending custom notification: {:?}", e);
                                }
                            });
                        }
                    }
                }
            }
            None => {
                log::debug!("TransactionDetails is missing a Transaction");
            }
        }
        Ok(())
    }

    // assume we own no inputs. sent to us from someone else's wallet.
    // all outputs we own should generate utxo deposit events.
    // outputs we don't own should not generate events.
    async fn receive_tx_notify<'a>(
        &self,
        plugin: &Plugin<State>,
        wallet: &Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>,
        tx: &TransactionDetails,
    ) -> Result<(), Error> {
        match tx.transaction.clone() {
            Some(t) => {
                for (vout, output) in t.output.iter().enumerate() {
                    if wallet.is_mine(&output.script_pubkey) {
                        match tx.confirmation_time {
                            ConfirmationTime::Unconfirmed { .. } => {
                                continue;
                            }
                            ConfirmationTime::Confirmed { height, time } => {
                                let acct: String;
                                let transfer_from: String;
                                if wallet.is_mine(&output.script_pubkey) {
                                    acct = format!("smaug:{}", self.get_name()?);
                                    transfer_from = "external".to_owned();
                                } else {
                                    // transfer_from = format!(
                                    //     "smaug:{}",
                                    //     self.get_name?
                                    // );
                                    // acct = "external".to_owned();
                                    continue;
                                }
                                let amount = output.value;
                                let outpoint =
                                    format!("{}:{}", tx.txid.to_string(), vout.to_string());
                                log::trace!(
                                    "outpoint = {}",
                                    format!("{}:{}", tx.txid.to_string(), vout.to_string())
                                );
                                let onchain_deposit = json!({
                                        "account": acct,
                                        "transfer_from": transfer_from,
                                        "outpoint": outpoint,
                                        "spending_txid": tx.txid.to_string(),
                                        "amount_msat": amount,
                                        "coin_type": "bcrt",
                                        "timestamp": format!("{}", time),
                                        "blockheight": format!("{}", height),
                                });
                                log::trace!("INSIDE SEND DEPOSIT NOTIFICATION ON SMAUG SIDE");
                                let cloned_plugin = plugin.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = cloned_plugin
                                        .send_custom_notification(
                                            UTXO_DEPOSIT_TAG.to_string(),
                                            onchain_deposit,
                                        )
                                        .await
                                    {
                                        log::error!("Error sending custom notification: {:?}", e);
                                    }
                                });
                            }
                        }
                    }
                }
            }
            None => {
                log::debug!("TransactionDetails is missing a Transaction");
            }
        }
        Ok(())
    }

    // assume we own some inputs and not others.
    // this tx was generated collaboratively between our wallet and (an)other wallet(s).
    // send events for all our owned inputs.
    // request manual intervention to identify which outputs are ours. send them to bkpr in a temporary account?
    async fn shared_tx_notify<'a>(
        &self,
        plugin: &Plugin<State>,
        wallet: &Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>,
        tx: &TransactionDetails,
    ) -> Result<(), Error> {
        match tx.transaction.clone() {
            Some(t) => {
                // send spent notification for each input that spends one of our outputs
                for input in t.input.iter() {
                    if let Some(po) = wallet.tx_graph().get_txout(input.previous_output) {
                        match tx.confirmation_time {
                            ConfirmationTime::Unconfirmed { .. } => {
                                continue;
                            }
                            ConfirmationTime::Confirmed { height, time } => {
                                if wallet.is_mine(&po.script_pubkey) {
                                    let acct = format!("smaug:{}", self.get_name()?);
                                    let amount = po.value;
                                    let outpoint = format!("{}", input.previous_output.to_string());
                                    log::trace!("outpoint = {}", format!("{}", outpoint));
                                    let onchain_spend = json!({
                                        "account": acct,
                                        "outpoint": outpoint,
                                        "spending_txid": tx.txid.to_string(),
                                        "amount_msat": amount,
                                        "coin_type": "bcrt",
                                        "timestamp": format!("{}", time),
                                        "blockheight": format!("{}", height),
                                    });
                                    log::trace!("INSIDE SEND SPEND NOTIFICATION ON SMAUG SIDE");
                                    let cloned_plugin = plugin.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = cloned_plugin
                                            .send_custom_notification(
                                                UTXO_SPENT_TAG.to_string(),
                                                onchain_spend,
                                            )
                                            .await
                                        {
                                            log::error!(
                                                "Error sending custom notification: {:?}",
                                                e
                                            );
                                        }
                                    });
                                }
                            }
                        }
                    } else {
                        log::debug!("Transaction prevout not found");
                    }
                }

                // send deposit notification for every output, since all of them *might be* spends from our wallet.
                // store them in a temp account and let the user update later as needed.
                for (vout, output) in t.output.iter().enumerate() {
                    match tx.confirmation_time {
                        ConfirmationTime::Unconfirmed { .. } => {
                            continue;
                        }
                        ConfirmationTime::Confirmed { height, time } => {
                            let acct: String;
                            let transfer_from: String;
                            let our_acct = format!("smaug:{}:shared_outputs", self.get_name()?);
                            let ext_acct = "external".to_owned();
                            if wallet.is_mine(&output.script_pubkey) {
                                acct = our_acct;
                                transfer_from = ext_acct;
                            } else {
                                acct = ext_acct;
                                transfer_from = our_acct;
                            }
                            let amount = output.value;
                            let outpoint = format!("{}:{}", tx.txid.to_string(), vout.to_string());
                            log::trace!(
                                "outpoint = {}",
                                format!("{}:{}", tx.txid.to_string(), vout.to_string())
                            );
                            let onchain_deposit = json!({
                                    "account": acct,
                                    "transfer_from": transfer_from,
                                    "outpoint": outpoint,
                                    "spending_txid": tx.txid.to_string(),
                                    "amount_msat": amount,
                                    "coin_type": "bcrt",
                                    "timestamp": format!("{}", time),
                                    "blockheight": format!("{}", height),
                            });
                            log::trace!("INSIDE SEND DEPOSIT NOTIFICATION ON SMAUG SIDE");
                            let cloned_plugin = plugin.clone();
                            tokio::spawn(async move {
                                if let Err(e) = cloned_plugin
                                    .send_custom_notification(
                                        UTXO_DEPOSIT_TAG.to_string(),
                                        onchain_deposit,
                                    )
                                    .await
                                {
                                    log::error!("Error sending custom notification: {:?}", e);
                                }
                            });
                        }
                    }
                }
            }
            None => {
                log::debug!("TransactionDetails is missing a Transaction");
            }
        }
        Ok(())
    }

    pub async fn send_notifications_for_tx<'a>(
        &self,
        plugin: &Plugin<State>,
        wallet: &Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>,
        tx: TransactionDetails,
    ) -> Result<(), Error> {
        log::debug!("sending notifs for txid/tx: {:?} {:?}", tx.txid, tx);
        // we own all inputs
        if tx.clone().transaction.unwrap().input.iter().all(|x| {
            match wallet.tx_graph().get_txout(x.previous_output) {
                Some(o) => {
                    log::trace!(
                        "output is mine?: {:?} {:?}",
                        o,
                        wallet.is_mine(&o.script_pubkey)
                    );
                    wallet.is_mine(&o.script_pubkey)
                }
                None => {
                    log::trace!("output not found in tx graph: {:?}", x.previous_output);
                    false
                }
            }
        }) {
            log::debug!("sending spend notif");
            self.spend_tx_notify(plugin, wallet, &tx).await?;
        } else
        // we own no inputs
        if !tx.clone().transaction.unwrap().input.iter().any(|x| {
            match wallet.tx_graph().get_txout(x.previous_output) {
                Some(o) => {
                    log::trace!(
                        "output is mine?: {:?} {:?}",
                        o,
                        wallet.is_mine(&o.script_pubkey)
                    );
                    wallet.is_mine(&o.script_pubkey)
                }
                None => {
                    log::trace!("output not found in tx graph: {:?}", x.previous_output);
                    false
                }
            }
        }) {
            log::debug!("sending deposit notif");
            self.receive_tx_notify(plugin, wallet, &tx).await?;
        }
        // we own some inputs but not others
        else {
            log::debug!("sending shared notif");
            self.shared_tx_notify(plugin, wallet, &tx).await?;
        }

        // if tx.sent > 0 {

        // }

        // if tx.received > 0 {

        // }
        Ok(())
    }
}

impl TryFrom<serde_json::Value> for DescriptorWallet {
    type Error = WatchError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        log::trace!("entering try_from");
        match value {
            serde_json::Value::Array(a) => {
                log::trace!("try_from: array detected = {:?}", a);
                let param_count = a.len();

                match param_count {
                    // 1 => DescriptorWallet::try_from(a.pop().unwrap()),
                    1..=4 => {
                        let descriptor = a.get(0).unwrap().as_str().ok_or_else(|| WatchError::InvalidDescriptor("descriptor must be a string".to_string()))?;
                        // let change_descriptor = Some(a.get(1).unwrap().as_str().ok_or_else(|| WatchError::InvalidChangeDescriptor("change_descriptor must be a string".to_string()))?);
                        log::trace!("try_from array: change_descriptor = {:?}", a.get(1));
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
                log::trace!("try_from: object detected");
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
