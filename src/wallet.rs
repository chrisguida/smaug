use anyhow::anyhow;
use bdk::{
    bitcoin::{
        secp256k1::{All, Secp256k1},
        BlockHash, Network, Transaction, Txid,
    },
    chain::{tx_graph::CanonicalTx, BlockId, ChainPosition, ConfirmationTimeAnchor},
    wallet::wallet_name_from_descriptor,
    Wallet,
};
use bdk_file_store::Store;
use bitcoincore_rpc::{
    bitcoincore_rpc_json::{
        ScanBlocksOptions, ScanBlocksRequest, ScanBlocksRequestDescriptor, ScanBlocksResult,
    },
    Auth, Client, RpcApi,
};
use clap::{command, Parser};
use cln_plugin::{Error, Plugin};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, fmt, path::PathBuf, time::Duration};

use crate::state::State;

pub const SMAUG_DATADIR: &str = ".smaug";

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

#[derive(Debug, Deserialize, Serialize, Clone)]
struct RpcConnection {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
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

pub fn get_esplora_url(network: &str) -> String {
    match network {
        "bitcoin" | "mainnet" => "https://blockstream.info/api".to_owned(),
        "testnet" => "https://blockstream.info/testnet/api".to_owned(),
        "regtest" | "mutinynet" => "https://mutinynet.com/api".to_owned(),
        "signet" => "https://mempool.space/signet/api".to_owned(),
        _ => {
            panic!("network must be bitcoin, testnet, regtest, signet, or mutinynet");
        }
    }
}

pub fn get_currency(network: Network) -> String {
    match network {
        Network::Bitcoin => "bc".to_owned(),
        Network::Regtest => "bcrt".to_owned(),
        Network::Signet => "tbs".to_owned(),
        Network::Testnet => "tb".to_owned(),
        _ => {
            panic!("Unknown bitcoin::network::constants::Network match arm");
        }
    }
}

pub fn parse_network(network: &Option<String>) -> Result<Network, Error> {
    return match network {
        Some(n) => match n.as_str() {
            "mutinynet" => Ok(Network::Signet),
            _ => match n.parse::<Network>() {
                Ok(n) => Ok(n),
                Err(e) => return Err(e.into()),
            },
        },
        None => return Err(anyhow!("network is None")),
    };
}

fn parse_currency(network: &Option<String>) -> Result<String, Error> {
    Ok(get_currency(parse_network(network)?))
}

fn find_closest_lower_key(map: &BTreeMap<u32, BlockHash>, key: u32) -> Option<(u32, BlockHash)> {
    let mut iter = map.range(..key);
    iter.next_back().map(|(&k, v)| (k, v.clone()))
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
    pub last_synced: Option<u32>,
    // #[serde(skip_serializing, skip_deserializing)]
    pub transactions: BTreeMap<Txid, Transaction>,
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
            last_synced: None,
        })
    }

    fn from_descriptor(descriptor: &str) -> Result<Self, WatchError> {
        Ok(Self {
            descriptor: descriptor.to_owned(),
            change_descriptor: None,
            birthday: None,
            gap: None,
            transactions: BTreeMap::new(),
            network: None,
            last_synced: None,
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

    fn sats_to_msats(amount: u64) -> u64 {
        amount * 1000
    }

    pub fn update_transactions<'a>(
        &mut self,
        transactions: Vec<CanonicalTx<'a, Transaction, ConfirmationTimeAnchor>>,
    ) -> Vec<CanonicalTx<'a, Transaction, ConfirmationTimeAnchor>> {
        let mut new_txs = vec![];
        for tx in transactions {
            if !self.transactions.contains_key(&tx.tx_node.txid) {
                new_txs.push(tx.clone());
                self.transactions
                    .insert(tx.tx_node.txid.clone(), tx.tx_node.tx.clone());
            }
        }
        new_txs
    }

    pub fn update_last_synced(&mut self, height: u32) {
        self.last_synced = Some(height);
    }

    pub fn get_network(&self) -> Result<Network, Error> {
        parse_network(&self.network)
    }

    pub fn get_name(&self) -> Result<String, Error> {
        log::trace!("get_name called");
        let network = parse_network(&self.network);
        log::trace!("get_network succeeded");
        Ok(wallet_name_from_descriptor(
            &self.descriptor,
            self.change_descriptor.as_ref(),
            network?,
            &Secp256k1::<All>::new(),
        )?)
    }

    pub async fn fetch_wallet<'a>(
        &mut self,
        db_dir: PathBuf,
        brpc_host: String,
        brpc_port: u16,
        brpc_auth: Auth,
    ) -> Result<Wallet<Store<'_, bdk::wallet::ChangeSet>>, Error> {
        log::trace!("creating path");
        let db_filename = self.get_name()?;
        let db_path = db_dir.join(format!("{}.db", db_filename,));
        log::trace!("searching for path: {:?}", db_path);
        let db = Store::<bdk::wallet::ChangeSet>::new_from_path(SMAUG_DATADIR.as_bytes(), db_path)?;
        log::trace!("db created!");
        let external_descriptor = self.descriptor.clone();
        let internal_descriptor = self.change_descriptor.clone();
        let mut wallet = Wallet::new(
            &external_descriptor,
            internal_descriptor.as_ref(),
            db,
            self.get_network()?,
        )?;
        log::trace!("wallet created!");

        let balance = wallet.get_balance();
        log::trace!("Wallet balance before syncing: {} sats", balance.total());

        log::trace!("Syncing...");
        log::debug!("using network: {}", json!(self.network).as_str().unwrap());

        log::trace!("using auth = {:?}", brpc_auth);
        log::trace!(
            "using url = {}",
            format!("http://{}:{}", brpc_host.clone(), brpc_port.clone())
        );

        let rpc_client = Client::new_with_timeout(
            &format!("http://{}:{}", brpc_host.clone(), brpc_port.clone()),
            brpc_auth,
            // Auth::UserPass(brpc_user.clone(), brpc_pass.clone()), // Auth::CookieFile(PathBuf::from("/home/cguida/.bitcoin/regtest/.cookie"))
            Duration::from_secs(3600),
        )?;

        let external_descriptor = ScanBlocksRequestDescriptor::Extended {
            desc: external_descriptor.to_string(),
            range: None,
        };
        let mut descriptors_vec = vec![external_descriptor];

        if let Some(id) = internal_descriptor {
            descriptors_vec.push(ScanBlocksRequestDescriptor::Extended {
                desc: id.to_string(),
                range: None,
            });
        }

        wallet.set_lookahead_for_all(20)?;

        log::info!("last_synced = {:?}", self.last_synced);
        let start_height: Option<u64> = match self.last_synced {
            Some(ct) => Some(ct.into()),
            None => None,
        };

        let descriptors = &descriptors_vec[..];
        let request = ScanBlocksRequest {
            scanobjects: descriptors,
            start_height,
            stop_height: None,
            filtertype: None,
            options: Some(ScanBlocksOptions {
                filter_false_positives: Some(true),
            }),
        };
        let res: ScanBlocksResult = rpc_client.scan_blocks_blocking(request)?;
        log::trace!("scanblocks result: {:?}", res);
        log::trace!("wallet = {:?}", wallet);

        let chain_tip = wallet.latest_checkpoint();
        let mut prev_block_id = match chain_tip {
            Some(ct) => Some(ct.block_id()),
            None => None,
        };

        // prev_block_id needs to be the block immediately before our current block

        for bh in res.relevant_blocks {
            let block = rpc_client.get_block(&bh)?;
            // let height: u32 = block.bip34_block_height()?.try_into().unwrap();
            // we really should not have to make two separate RPC calls here.
            // unfortunately rust-bitcoin does not expose an rpc method that returns
            // both the full transaction dump and the height.
            let height: u32 = rpc_client
                .get_block_header_info(&bh)?
                .height
                .try_into()
                .unwrap();
            if let Some(p) = prev_block_id {
                if height <= p.height {
                    if let Some((height, hash)) =
                        find_closest_lower_key(wallet.local_chain().blocks(), height)
                    {
                        prev_block_id = Some(BlockId { height, hash });
                    } else {
                        prev_block_id = None;
                    }
                };
            }
            wallet.apply_block_relevant(block.clone(), prev_block_id, height)?;
            wallet.commit()?;
            prev_block_id = Some(BlockId { height, hash: bh });
        }

        self.update_last_synced(res.to_height.try_into().unwrap());

        log::debug!("last_synced after scan = {:?}", self.last_synced);

        let balance = wallet.get_balance();
        log::trace!("Wallet balance after syncing: {} sats", balance.total());
        return Ok(wallet);
    }

    // assume we own all inputs, ie sent from our wallet. all inputs and outputs should generate coin movement bookkeeper events
    async fn spend_tx_notify<'a>(
        &self,
        plugin: &Plugin<State>,
        wallet: &Wallet<Store<'_, bdk::wallet::ChangeSet>>,
        tx: &CanonicalTx<'_, Transaction, ConfirmationTimeAnchor>,
    ) -> Result<(), Error> {
        let coin_type = parse_currency(&self.network)?;
        // send spent notification for each input
        for input in tx.tx_node.tx.input.iter() {
            if let Some(po) = wallet.tx_graph().get_txout(input.previous_output) {
                match tx.chain_position {
                    ChainPosition::Unconfirmed(_) => {
                        continue;
                    }
                    ChainPosition::Confirmed(a) => {
                        let acct = format!("smaug:{}", self.get_name()?);
                        let amount = po.value;
                        let outpoint = format!("{}", input.previous_output.to_string());
                        log::trace!("outpoint = {}", format!("{}", outpoint));
                        let onchain_spend = json!({UTXO_SPENT_TAG: {
                            "account": acct,
                            "outpoint": outpoint,
                            "spending_txid": tx.tx_node.txid,
                            "amount_msat": Self::sats_to_msats(amount),
                            "coin_type": coin_type,
                            "timestamp": format!("{}", a.confirmation_time),
                            "blockheight": format!("{}", a.confirmation_height),
                        }});
                        log::trace!("INSIDE SEND SPEND NOTIFICATION ON SMAUG SIDE");
                        let cloned_plugin = plugin.clone();
                        tokio::spawn(async move {
                            if let Err(e) = cloned_plugin
                                .send_custom_notification(UTXO_SPENT_TAG.to_string(), onchain_spend)
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
        for (vout, output) in tx.tx_node.tx.output.iter().enumerate() {
            match tx.chain_position {
                ChainPosition::Unconfirmed(_) => {
                    continue;
                }
                ChainPosition::Confirmed(a) => {
                    let acct: String;
                    // all outputs are being transferred from our wallet
                    let transfer_from = format!("smaug:{}", self.get_name()?);
                    if wallet.is_mine(&output.script_pubkey) {
                        // this is a deposit from ourselves to ourselves, ie change
                        acct = transfer_from.clone();
                    } else {
                        // this is a deposit from ourselves to an external wallet
                        acct = "external".to_owned();
                    }
                    let amount = output.value;
                    let outpoint = format!("{}:{}", tx.tx_node.txid.to_string(), vout.to_string());
                    log::trace!("outpoint = {}", format!("{}:{}", tx.tx_node.txid, vout));
                    let onchain_deposit = json!({UTXO_DEPOSIT_TAG:{
                            "account": acct,
                            "transfer_from": transfer_from,
                            "outpoint": outpoint,
                            "spending_txid": tx.tx_node.txid,
                            "amount_msat": Self::sats_to_msats(amount),
                            "coin_type": coin_type,
                            "timestamp": format!("{}", a.confirmation_time),
                            "blockheight": format!("{}", a.confirmation_height),
                    }});
                    log::trace!("INSIDE SEND DEPOSIT NOTIFICATION ON SMAUG SIDE");
                    let cloned_plugin = plugin.clone();
                    tokio::spawn(async move {
                        if let Err(e) = cloned_plugin
                            .send_custom_notification(UTXO_DEPOSIT_TAG.to_string(), onchain_deposit)
                            .await
                        {
                            log::error!("Error sending custom notification: {:?}", e);
                        }
                    });
                }
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
        wallet: &Wallet<Store<'_, bdk::wallet::ChangeSet>>,
        tx: &CanonicalTx<'_, Transaction, ConfirmationTimeAnchor>,
    ) -> Result<(), Error> {
        let coin_type = parse_currency(&self.network)?;
        for (vout, output) in tx.tx_node.tx.output.iter().enumerate() {
            if wallet.is_mine(&output.script_pubkey) {
                match tx.chain_position {
                    ChainPosition::Unconfirmed(_) => {
                        continue;
                    }
                    ChainPosition::Confirmed(a) => {
                        let acct: String;
                        let transfer_from: String;
                        if wallet.is_mine(&output.script_pubkey) {
                            acct = format!("smaug:{}", self.get_name()?);
                            transfer_from = "external".to_owned();
                        } else {
                            continue;
                        }
                        let amount = output.value;
                        let outpoint = format!("{}:{}", tx.tx_node.txid, vout);
                        log::trace!(
                            "outpoint = {}",
                            format!("{}:{}", tx.tx_node.txid.to_string(), vout.to_string())
                        );
                        let onchain_deposit = json!({UTXO_DEPOSIT_TAG: {
                                "account": acct,
                                "transfer_from": transfer_from,
                                "outpoint": outpoint,
                                "spending_txid": tx.tx_node.txid.to_string(),
                                "amount_msat": Self::sats_to_msats(amount),
                                "coin_type": coin_type,
                                "timestamp": format!("{}", a.confirmation_time),
                                "blockheight": format!("{}", a.confirmation_height),
                        }});
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
        Ok(())
    }

    // assume we own some inputs and not others.
    // this tx was generated collaboratively between our wallet and (an)other wallet(s).
    // send events for all our owned inputs.
    // request manual intervention to identify which outputs are ours. send them to bkpr in a temporary account?
    async fn shared_tx_notify<'a>(
        &self,
        plugin: &Plugin<State>,
        wallet: &Wallet<Store<'_, bdk::wallet::ChangeSet>>,
        tx: &CanonicalTx<'_, Transaction, ConfirmationTimeAnchor>,
    ) -> Result<(), Error> {
        let coin_type = parse_currency(&self.network)?;
        for input in tx.tx_node.input.iter() {
            if let Some(po) = wallet.tx_graph().get_txout(input.previous_output) {
                match tx.chain_position {
                    ChainPosition::Unconfirmed(_) => {
                        continue;
                    }
                    ChainPosition::Confirmed(a) => {
                        let our_acct = format!("smaug:{}", self.get_name()?);
                        let ext_acct = "external".to_owned();
                        let acct: String;
                        if wallet.is_mine(&po.script_pubkey) {
                            acct = our_acct;
                        } else {
                            acct = ext_acct;
                        }
                        let amount = po.value;
                        let outpoint = format!("{}", input.previous_output.to_string());
                        log::trace!("outpoint = {}", format!("{}", outpoint));
                        let onchain_spend = json!({UTXO_SPENT_TAG: {
                            "account": acct,
                            "outpoint": outpoint,
                            "spending_txid": tx.tx_node.txid.to_string(),
                            "amount_msat": Self::sats_to_msats(amount),
                            "coin_type": coin_type,
                            "timestamp": format!("{}", a.confirmation_time),
                            "blockheight": format!("{}", a.confirmation_height),
                        }});
                        log::trace!("INSIDE SEND SPEND NOTIFICATION ON SMAUG SIDE");
                        let cloned_plugin = plugin.clone();
                        tokio::spawn(async move {
                            if let Err(e) = cloned_plugin
                                .send_custom_notification(UTXO_SPENT_TAG.to_string(), onchain_spend)
                                .await
                            {
                                log::error!("Error sending custom notification: {:?}", e);
                            }
                        });
                        // }
                    }
                }
            } else {
                log::debug!("Transaction prevout not found");
            }
        }

        // send deposit notification for every output, since all of them *might be* spends from our wallet.
        // store them in a temp account and let the user update later as needed.
        // don't send transfer_from if output is_mine
        for (vout, output) in tx.tx_node.tx.output.iter().enumerate() {
            match tx.chain_position {
                ChainPosition::Unconfirmed(_) => {
                    continue;
                }
                ChainPosition::Confirmed(a) => {
                    let acct: String;
                    let transfer_from: String;
                    let our_acct = format!("smaug:{}", self.get_name()?);
                    let ext_acct = "external".to_owned();
                    if wallet.is_mine(&output.script_pubkey) {
                        acct = our_acct;
                        transfer_from = ext_acct;
                    } else {
                        acct = ext_acct;
                        transfer_from = our_acct;
                    }
                    let amount = output.value;
                    let outpoint = format!("{}:{}", tx.tx_node.txid, vout);
                    log::trace!("outpoint = {}", format!("{}:{}", tx.tx_node.txid, vout));
                    let onchain_deposit = json!({UTXO_DEPOSIT_TAG: {
                            "account": acct,
                            "transfer_from": transfer_from,
                            "outpoint": outpoint,
                            "spending_txid": tx.tx_node.txid,
                            "amount_msat": Self::sats_to_msats(amount),
                            "coin_type": coin_type,
                            "timestamp": format!("{}", a.confirmation_time),
                            "blockheight": format!("{}", a.confirmation_height),
                    }});
                    log::trace!("INSIDE SEND DEPOSIT NOTIFICATION ON SMAUG SIDE");
                    let cloned_plugin = plugin.clone();
                    tokio::spawn(async move {
                        if let Err(e) = cloned_plugin
                            .send_custom_notification(UTXO_DEPOSIT_TAG.to_string(), onchain_deposit)
                            .await
                        {
                            log::error!("Error sending custom notification: {:?}", e);
                        }
                    });
                }
            }
        }
        Ok(())
    }

    pub async fn send_notifications_for_tx<'a>(
        &self,
        plugin: &Plugin<State>,
        wallet: &Wallet<Store<'_, bdk::wallet::ChangeSet>>,
        tx: CanonicalTx<'_, Transaction, ConfirmationTimeAnchor>,
    ) -> Result<(), Error> {
        log::debug!("sending notifs for txid/tx: {:?} {:?}", tx.tx_node.txid, tx);
        // we own all inputs
        if tx.clone().tx_node.tx.input.iter().all(|x| {
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
        if !tx.clone().tx_node.tx.input.iter().any(|x| {
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
                    1..=4 => {
                        let descriptor = a.get(0).unwrap().as_str().ok_or_else(|| WatchError::InvalidDescriptor("descriptor must be a string".to_string()))?;
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
